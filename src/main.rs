use crate::config::Config;
use clap::Parser;
use llm_responses::LlmResponsesComposer;
use llm_selector::LlmSelector;
use server::{OpenAiServer, TlsConfig};
use token_measure::TokenMeasure;
use tracing::{debug, info};

mod config;

#[derive(Debug, clap::Parser)]
struct Args {
    #[arg(long, help = "Enable JSON logging")]
    json_log: bool,

    #[arg(
        long,
        help = "Configuration file",
        default_value = "/etc/ayatori/config.toml"
    )]
    config: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if args.json_log {
        tracing_subscriber::fmt().json().init();
    } else {
        tracing_subscriber::fmt().init();
    }

    info!("Starting Ayatori server");
    debug!("Arguments: {:?}", args);

    let config = Config::load(args.config);
    debug!("Configuration: {:?}", config);

    let configuration = config.load_configuration();
    debug!("LLM configuration: {:?}", configuration);

    let api_key = config.load_api_key();
    debug!("API key loaded");

    debug!("Loading LLM selector");
    let responses_composer = LlmResponsesComposer::new(configuration.clone());
    let selector = {
        let usage_store = config.load_usage_store().await;
        LlmSelector::new(configuration, usage_store)
    };

    let token_measure = TokenMeasure::from(config.token_measure);

    OpenAiServer::new(
        selector,
        responses_composer,
        config.server.listen,
        config
            .server
            .tls
            .map(|tls| TlsConfig::new(tls.private_key_file, tls.certificate_chain_file)),
        api_key,
        config.server.client_fallback_enabled.unwrap_or(false),
        token_measure,
    )
    .run()
    .await
}
