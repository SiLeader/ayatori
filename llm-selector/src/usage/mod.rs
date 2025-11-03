use configuration::CapacityLimits;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

mod local;
mod redis;
mod selector;

use crate::usage::local::LocalUsageStore;
use crate::usage::redis::RedisUsageStore;
pub(crate) use selector::UsageSelector;

#[derive(Clone, Debug, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub requests: u64,
}

#[derive(Debug)]
pub struct UsageError {
    kind: std::io::ErrorKind,
    message: String,
}

#[async_trait::async_trait]
pub trait UsageStore: Send + Sync {
    async fn get_usage(&self, id: &str) -> Result<Usage, UsageError>;
    async fn append_usage(&self, id: &str, usage: &Usage) -> Result<(), UsageError>;
    async fn remove_usage(&self, id: &str, usage: &Usage) -> Result<(), UsageError>;
}

impl Usage {
    pub(crate) fn is_reached(&self, capacity: &CapacityLimits) -> bool {
        if let Some(input_tokens) = capacity.input_tokens
            && input_tokens < self.input_tokens
        {
            return true;
        }
        if let Some(requests) = capacity.requests
            && requests < self.requests
        {
            return true;
        }
        false
    }
}

impl UsageError {
    pub fn kind(&self) -> std::io::ErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
pub enum UsageStoreConfig {
    Local,
    Redis {
        host: Option<String>,
        port: Option<u16>,
        db: Option<i64>,
        username: Option<String>,
        password: Option<String>,
        password_env: Option<String>,
        ca_cert_file: Option<String>,
        client_cert_file: Option<String>,
        client_key_file: Option<String>,
        response_timeout_seconds: Option<u64>,
        connect_timeout_seconds: Option<u64>,
    },
}

impl UsageStoreConfig {
    pub async fn create(&self) -> Arc<dyn UsageStore> {
        match self {
            UsageStoreConfig::Local => Arc::new(LocalUsageStore::default()),
            UsageStoreConfig::Redis {
                host,
                port,
                db,
                username,
                password,
                password_env,
                ca_cert_file,
                client_cert_file,
                client_key_file,
                response_timeout_seconds,
                connect_timeout_seconds,
            } => {
                let mut builder = RedisUsageStore::builder();
                if let Some(host) = host {
                    builder = builder.host(host.clone());
                }
                if let Some(port) = port {
                    builder = builder.port(*port);
                }
                if let Some(db) = db {
                    builder = builder.db(*db);
                }
                if let Some(username) = username {
                    builder = builder.username(username.clone());
                }
                if let Some(password) = password {
                    builder = builder.password(password.clone());
                }
                if let Some(password_env) = password_env {
                    builder = builder.password(
                        std::env::var(password_env).expect("Failed to read password env"),
                    );
                }
                if let Some(ca_cert_file) = ca_cert_file {
                    builder = builder
                        .ca_cert_file(ca_cert_file)
                        .expect("Failed to read ca cert file");
                }
                if let Some(client_cert_file) = client_cert_file
                    && let Some(client_key_file) = client_key_file
                {
                    builder = builder
                        .client_cert(client_cert_file, client_key_file)
                        .expect("Failed to read client cert file");
                }
                if let Some(response_timeout_seconds) = response_timeout_seconds {
                    builder =
                        builder.response_timeout(Duration::from_secs(*response_timeout_seconds));
                }
                if let Some(connect_timeout_seconds) = connect_timeout_seconds {
                    builder =
                        builder.connect_timeout(Duration::from_secs(*connect_timeout_seconds));
                }
                Arc::new(builder.build().expect("Failed to create redis usage store"))
            }
        }
    }
}
