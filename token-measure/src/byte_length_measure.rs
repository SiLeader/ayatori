use crate::{MeasureToken, MeasureTokenError};

#[derive(Debug, Clone)]
pub struct ByteLengthTokenMeasure {
    magnification_ratio: f64,
}

impl ByteLengthTokenMeasure {
    pub fn new(magnification_ratio: f64) -> Self {
        Self {
            magnification_ratio,
        }
    }
}

#[async_trait::async_trait]
impl MeasureToken for ByteLengthTokenMeasure {
    async fn measure_token(&self, _client_id: &str, value: &str) -> Result<u64, MeasureTokenError> {
        Ok(((value.len() as f64) * self.magnification_ratio).round() as u64)
    }
}
