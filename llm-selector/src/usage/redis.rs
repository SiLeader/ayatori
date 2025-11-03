use crate::{Usage, UsageError, UsageStore};
use redis::aio::MultiplexedConnection;
use redis::{
    AsyncConnectionConfig, Client, ClientTlsConfig, ConnectionAddr, ConnectionInfo, ErrorKind,
    RedisConnectionInfo, RedisError, RedisResult, TlsCertificates, pipe,
};
use std::fs::read;
use std::path::Path;
use std::time::Duration;

#[derive(Clone)]
pub struct RedisUsageStore {
    client: Client,
    response_timeout: Duration,
    connect_timeout: Duration,
}

pub struct RedisUsageStoreBuilder {
    host: String,
    port: u16,
    db: i64,
    username: Option<String>,
    password: Option<String>,
    ca_cert: Option<Vec<u8>>,
    client_cert: Option<Vec<u8>>,
    client_key: Option<Vec<u8>>,
    response_timeout: Duration,
    connect_timeout: Duration,
}

impl RedisUsageStoreBuilder {
    pub(crate) fn new() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 6379,
            db: 0,
            username: None,
            password: None,
            ca_cert: None,
            client_cert: None,
            client_key: None,
            response_timeout: Duration::from_secs(3),
            connect_timeout: Duration::from_secs(3),
        }
    }
}

impl RedisUsageStore {
    pub fn builder() -> RedisUsageStoreBuilder {
        RedisUsageStoreBuilder::new()
    }

    async fn get_connection(&self) -> Result<MultiplexedConnection, UsageError> {
        Ok(self
            .client
            .get_multiplexed_async_connection_with_config(
                &AsyncConnectionConfig::default()
                    .set_connection_timeout(self.connect_timeout)
                    .set_response_timeout(self.response_timeout),
            )
            .await?)
    }
}

fn keys(client_id: &str) -> (String, String) {
    (
        format!("ayatori:usage:{client_id}:input_tokens"),
        format!("ayatori:usage:{client_id}:requests"),
    )
}

#[async_trait::async_trait]
impl UsageStore for RedisUsageStore {
    async fn get_usage(&self, id: &str) -> Result<Usage, UsageError> {
        let mut conn = self.get_connection().await?;

        let (input_key, request_key) = keys(id);
        let (input_tokens, requests): (u64, u64) = pipe()
            .atomic()
            .get(input_key)
            .get(request_key)
            .query_async(&mut conn)
            .await?;

        Ok(Usage {
            input_tokens,
            requests,
        })
    }

    async fn append_usage(&self, id: &str, usage: &Usage) -> Result<(), UsageError> {
        let mut conn = self.get_connection().await?;
        let (input_key, request_key) = keys(id);

        let (_input_tokens, _requests): (u64, u64) = pipe()
            .atomic()
            .incr(input_key, usage.input_tokens)
            .incr(request_key, usage.requests)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    async fn remove_usage(&self, id: &str, usage: &Usage) -> Result<(), UsageError> {
        let mut conn = self.get_connection().await?;
        let (input_key, request_key) = keys(id);

        let (_input_tokens, _requests): (u64, u64) = pipe()
            .atomic()
            .decr(input_key, usage.input_tokens)
            .decr(request_key, usage.requests)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }
}

impl RedisUsageStoreBuilder {
    pub fn build(self) -> RedisResult<RedisUsageStore> {
        let client = if let Some(ca_cert) = self.ca_cert {
            let client_cert_key_pair = self
                .client_cert
                .and_then(|c| self.client_key.map(|k| (c, k)));
            Client::build_with_tls(
                ConnectionInfo {
                    addr: ConnectionAddr::TcpTls {
                        host: self.host,
                        port: self.port,
                        insecure: false,
                        tls_params: None,
                    },
                    redis: RedisConnectionInfo {
                        db: self.db,
                        username: self.username,
                        password: self.password,
                        protocol: Default::default(),
                    },
                },
                TlsCertificates {
                    client_tls: client_cert_key_pair.map(|(c, k)| ClientTlsConfig {
                        client_cert: c,
                        client_key: k,
                    }),
                    root_cert: Some(ca_cert),
                },
            )
        } else {
            Client::open(ConnectionInfo {
                addr: ConnectionAddr::Tcp(self.host, self.port),
                redis: RedisConnectionInfo {
                    db: self.db,
                    username: self.username,
                    password: self.password,
                    protocol: Default::default(),
                },
            })
        }?;
        Ok(RedisUsageStore {
            client,
            response_timeout: self.response_timeout,
            connect_timeout: self.connect_timeout,
        })
    }

    pub fn host(mut self, host: String) -> Self {
        self.host = host;
        self
    }

    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    pub fn db(mut self, db: i64) -> Self {
        self.db = db;
        self
    }

    pub fn username(mut self, username: String) -> Self {
        self.username = Some(username);
        self
    }

    pub fn password(mut self, password: String) -> Self {
        self.password = Some(password);
        self
    }

    pub fn ca_cert_file(mut self, ca_cert_file: impl AsRef<Path>) -> std::io::Result<Self> {
        let content = read(ca_cert_file)?;
        self.ca_cert = Some(content);
        Ok(self)
    }

    pub fn client_cert(
        mut self,
        client_cert_file: impl AsRef<Path>,
        client_key_file: impl AsRef<Path>,
    ) -> std::io::Result<Self> {
        let cert_content = read(client_cert_file)?;
        let key_content = read(client_key_file)?;
        self.client_cert = Some(cert_content);
        self.client_key = Some(key_content);
        Ok(self)
    }

    pub fn connect_timeout(mut self, connect_timeout: Duration) -> Self {
        self.connect_timeout = connect_timeout;
        self
    }

    pub fn response_timeout(mut self, response_timeout: Duration) -> Self {
        self.response_timeout = response_timeout;
        self
    }
}

impl From<RedisError> for UsageError {
    fn from(value: RedisError) -> Self {
        let kind = match value.kind() {
            ErrorKind::ResponseError => std::io::ErrorKind::InvalidData,
            ErrorKind::ParseError => std::io::ErrorKind::InvalidData,
            ErrorKind::AuthenticationFailed => std::io::ErrorKind::PermissionDenied,
            ErrorKind::TypeError => std::io::ErrorKind::InvalidData,
            ErrorKind::ExecAbortError => std::io::ErrorKind::ConnectionAborted,
            ErrorKind::BusyLoadingError => std::io::ErrorKind::ResourceBusy,
            ErrorKind::NoScriptError => std::io::ErrorKind::InvalidData,
            ErrorKind::InvalidClientConfig => std::io::ErrorKind::ConnectionRefused,
            ErrorKind::Moved => std::io::ErrorKind::InvalidFilename,
            ErrorKind::Ask => std::io::ErrorKind::InvalidData,
            ErrorKind::TryAgain => std::io::ErrorKind::ResourceBusy,
            ErrorKind::ClusterDown => std::io::ErrorKind::ConnectionRefused,
            ErrorKind::CrossSlot => std::io::ErrorKind::InvalidData,
            ErrorKind::MasterDown => std::io::ErrorKind::ConnectionRefused,
            ErrorKind::IoError => std::io::ErrorKind::InvalidData,
            ErrorKind::ClientError => std::io::ErrorKind::InvalidData,
            ErrorKind::ExtensionError => std::io::ErrorKind::InvalidData,
            ErrorKind::ReadOnly => std::io::ErrorKind::PermissionDenied,
            ErrorKind::MasterNameNotFoundBySentinel => std::io::ErrorKind::ConnectionRefused,
            ErrorKind::NoValidReplicasFoundBySentinel => std::io::ErrorKind::ConnectionRefused,
            ErrorKind::EmptySentinelList => std::io::ErrorKind::ConnectionRefused,
            ErrorKind::NotBusy => std::io::ErrorKind::ConnectionRefused,
            ErrorKind::ClusterConnectionNotFound => std::io::ErrorKind::ConnectionRefused,
            ErrorKind::NoSub => std::io::ErrorKind::InvalidData,
            ErrorKind::RESP3NotSupported => std::io::ErrorKind::Unsupported,
            _ => std::io::ErrorKind::InvalidData,
        };
        Self {
            kind,
            message: value.to_string(),
        }
    }
}
