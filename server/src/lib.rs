mod endpoints;

use actix_web::middleware::Logger;
use actix_web::web::Data;
use actix_web::{App, HttpResponse, HttpServer, get};
use actix_web_httpauth::extractors::bearer::BearerAuth;
use llm_selector::LlmSelector;
use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use token_measure::TokenMeasure;

pub struct OpenAiServer {
    selector: LlmSelector,
    listen: String,
    tls_config: Option<TlsConfig>,
    api_key: Option<String>,
    client_fallback_enabled: bool,
    token_measure: TokenMeasure,
}

pub struct TlsConfig {
    private_key_file: String,
    certificate_chain_file: String,
}

#[derive(Clone, Debug)]
struct ApiKey(Option<String>);

#[derive(Clone, Debug)]
struct AppConfig {
    pub(crate) client_fallback_enabled: bool,
}

impl TlsConfig {
    pub fn new(private_key_file: String, certificate_chain_file: String) -> Self {
        Self {
            private_key_file,
            certificate_chain_file,
        }
    }
}

impl ApiKey {
    fn is_match(&self, auth: Option<BearerAuth>) -> bool {
        if let Some(auth) = auth {
            self.0.as_ref().is_some_and(|s| auth.token() == s)
        } else {
            self.0.is_none()
        }
    }
}

impl OpenAiServer {
    pub fn new(
        selector: LlmSelector,
        listen: String,
        tls_config: Option<TlsConfig>,
        api_key: Option<String>,
        client_fallback_enabled: bool,
        token_measure: TokenMeasure,
    ) -> Self {
        Self {
            selector,
            listen,
            tls_config,
            api_key,
            client_fallback_enabled,
            token_measure,
        }
    }

    pub async fn run(self) {
        let api_key = ApiKey(self.api_key);
        let app_config = AppConfig {
            client_fallback_enabled: self.client_fallback_enabled,
        };
        let server = HttpServer::new(move || {
            App::new()
                .wrap(Logger::default().exclude("/healthz"))
                .app_data(Data::new(self.selector.clone()))
                .app_data(Data::new(api_key.clone()))
                .app_data(Data::new(app_config.clone()))
                .app_data(Data::new(self.token_measure.clone()))
                .service(health_check)
                .configure(endpoints::register_endpoints)
        });

        if let Some(tls_config) = self.tls_config {
            let cert_chain = CertificateDer::pem_file_iter(tls_config.certificate_chain_file)
                .expect("Failed to load TLS certificate chain")
                .flatten()
                .collect();
            let private_key = PrivateKeyDer::from_pem_file(tls_config.private_key_file)
                .expect("Failed to load TLS private key");
            let config = ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(cert_chain, private_key)
                .expect("Failed to build TLS config");
            server
                .bind_rustls_0_23(self.listen, config)
                .expect("Failed to bind server")
                .run()
                .await
                .expect("Failed to run server");
        } else {
            server
                .bind(self.listen)
                .expect("Failed to bind server")
                .run()
                .await
                .expect("Failed to run server");
        }
    }
}

#[get("/healthz")]
async fn health_check() -> HttpResponse {
    HttpResponse::Ok().finish()
}
