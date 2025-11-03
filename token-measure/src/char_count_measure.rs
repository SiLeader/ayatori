use crate::{MeasureToken, MeasureTokenError};

#[derive(Debug, Clone)]
pub struct CharCountTokenMeasure {
    magnification_ratio: f64,
}

impl CharCountTokenMeasure {
    pub fn new(magnification_ratio: f64) -> Self {
        Self {
            magnification_ratio,
        }
    }
}

#[async_trait::async_trait]
impl MeasureToken for CharCountTokenMeasure {
    async fn measure_token(&self, _client_id: &str, value: &str) -> Result<u64, MeasureTokenError> {
        Ok((value.chars().count() as f64 * self.magnification_ratio).round() as u64)
    }
}
