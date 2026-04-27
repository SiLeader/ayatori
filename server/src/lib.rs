mod endpoints;
mod error;
mod model;

use crate::error::ErrorResponse;
use actix_web::middleware::Logger;
use actix_web::web::Data;
use actix_web::{App, HttpResponse, HttpServer, get};
use actix_web_httpauth::extractors::bearer::BearerAuth;
use llm_responses::{LocalResponseStore, ResponseStore};
use llm_selector::LlmSelector;
use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::sync::Arc;
use std::time::Duration;
use token_measure::TokenMeasure;

pub struct OpenAiServer {
    selector: LlmSelector,
    listen: String,
    tls_config: Option<TlsConfig>,
    api_key: Option<String>,
    client_fallback_enabled: bool,
    response_store: Arc<dyn ResponseStore>,
    response_store_ttl: Option<Duration>,
    token_measure: TokenMeasure,
}

pub struct TlsConfig {
    private_key_file: String,
    certificate_chain_file: String,
}

#[derive(Clone, Debug)]
pub struct ApiKey(Option<String>);

#[derive(Clone)]
pub struct AppConfig {
    pub(crate) client_fallback_enabled: bool,
    pub(crate) response_store: Arc<dyn ResponseStore>,
    pub(crate) response_store_ttl: Option<Duration>,
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
    pub fn new(value: Option<String>) -> Self {
        Self(value)
    }

    fn check_api_key_using_token_str(&self, auth: Option<&str>) -> Result<(), ErrorResponse> {
        if let Some(auth) = auth {
            if self.0.as_ref().is_none_or(|s| auth != s) {
                return Err(ErrorResponse::incorrect_api_key_provided());
            }
        } else if self.0.is_some() {
            return Err(ErrorResponse::invalid_authentication());
        }
        Ok(())
    }

    fn check_api_key(&self, auth: Option<BearerAuth>) -> Result<(), ErrorResponse> {
        self.check_api_key_using_token_str(auth.as_ref().map(|a| a.token()))
    }
}

impl AppConfig {
    pub fn new(client_fallback_enabled: bool) -> Self {
        Self::with_response_store(
            client_fallback_enabled,
            Arc::new(LocalResponseStore::new(10_000)),
            Some(Duration::from_secs(24 * 60 * 60)),
        )
    }

    pub fn with_response_store(
        client_fallback_enabled: bool,
        response_store: Arc<dyn ResponseStore>,
        response_store_ttl: Option<Duration>,
    ) -> Self {
        Self {
            client_fallback_enabled,
            response_store,
            response_store_ttl,
        }
    }
}

impl std::fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppConfig")
            .field("client_fallback_enabled", &self.client_fallback_enabled)
            .field("response_store_ttl", &self.response_store_ttl)
            .finish()
    }
}

pub fn configure_openai_compatible_endpoints(config: &mut actix_web::web::ServiceConfig) {
    endpoints::register_endpoints(config);
}

impl OpenAiServer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        selector: LlmSelector,
        listen: String,
        tls_config: Option<TlsConfig>,
        api_key: Option<String>,
        client_fallback_enabled: bool,
        response_store: Arc<dyn ResponseStore>,
        response_store_ttl: Option<Duration>,
        token_measure: TokenMeasure,
    ) -> Self {
        Self {
            selector,
            listen,
            tls_config,
            api_key,
            client_fallback_enabled,
            response_store,
            response_store_ttl,
            token_measure,
        }
    }

    pub async fn run(self) {
        let api_key = ApiKey::new(self.api_key);
        let app_config = AppConfig::with_response_store(
            self.client_fallback_enabled,
            self.response_store,
            self.response_store_ttl,
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn error_code(err: ErrorResponse) -> String {
        let value = serde_json::to_value(err).unwrap();
        value["error"]["code"].as_str().unwrap().to_string()
    }

    #[test]
    fn no_key_configured_no_token_provided() {
        let api_key = ApiKey(None);
        assert!(api_key.check_api_key_using_token_str(None).is_ok());
    }

    #[test]
    fn no_key_configured_token_provided() {
        let api_key = ApiKey(None);
        let err = api_key
            .check_api_key_using_token_str(Some("any"))
            .unwrap_err();
        assert_eq!(error_code(err), "incorrect_api_key_provided");
    }

    #[test]
    fn key_configured_no_token_provided() {
        let api_key = ApiKey(Some("secret".to_string()));
        let err = api_key.check_api_key_using_token_str(None).unwrap_err();
        assert_eq!(error_code(err), "invalid_authentication");
    }

    #[test]
    fn key_configured_matching_token() {
        let api_key = ApiKey(Some("secret".to_string()));
        assert!(
            api_key
                .check_api_key_using_token_str(Some("secret"))
                .is_ok()
        );
    }

    #[test]
    fn key_configured_wrong_token() {
        let api_key = ApiKey(Some("secret".to_string()));
        let err = api_key
            .check_api_key_using_token_str(Some("wrong"))
            .unwrap_err();
        assert_eq!(error_code(err), "incorrect_api_key_provided");
    }
}
