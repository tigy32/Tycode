use anyhow::anyhow;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AiError {
    #[error("Retryable error: {0}")]
    Retryable(anyhow::Error),

    #[error("Terminal error: {0}")]
    Terminal(anyhow::Error),
}

impl From<serde_json::Error> for AiError {
    fn from(source: serde_json::Error) -> Self {
        Self::Terminal(anyhow!(source))
    }
}
