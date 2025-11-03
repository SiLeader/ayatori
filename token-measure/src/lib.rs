mod byte_length_measure;
mod char_count_measure;

pub use byte_length_measure::ByteLengthTokenMeasure;
pub use char_count_measure::CharCountTokenMeasure;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug)]
pub enum MeasureTokenError {}

#[async_trait::async_trait]
pub trait MeasureToken: Send + Sync {
    async fn measure_token(&self, client_id: &str, value: &str) -> Result<u64, MeasureTokenError>;
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum TokenMeasureConfig {
    ByteLength {
        #[serde(default)]
        magnification_ratio: Option<f64>,
    },
    CharCount {
        #[serde(default)]
        magnification_ratio: Option<f64>,
    },
}

#[derive(Clone)]
pub struct TokenMeasure {
    measure: Arc<dyn MeasureToken>,
}

impl TokenMeasure {
    pub fn new<M>(measure: M) -> Self
    where
        M: MeasureToken + 'static,
    {
        Self {
            measure: Arc::new(measure),
        }
    }
}

#[async_trait::async_trait]
impl MeasureToken for TokenMeasure {
    async fn measure_token(&self, client_id: &str, value: &str) -> Result<u64, MeasureTokenError> {
        self.measure.measure_token(client_id, value).await
    }
}

impl From<TokenMeasureConfig> for TokenMeasure {
    fn from(value: TokenMeasureConfig) -> Self {
        match value {
            TokenMeasureConfig::ByteLength {
                magnification_ratio,
            } => TokenMeasure::new(ByteLengthTokenMeasure::new(
                magnification_ratio.unwrap_or(1. / 3.),
            )),
            TokenMeasureConfig::CharCount {
                magnification_ratio,
            } => TokenMeasure::new(CharCountTokenMeasure::new(
                magnification_ratio.unwrap_or(1. / 2.),
            )),
        }
    }
}
