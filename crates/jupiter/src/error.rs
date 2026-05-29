use thiserror::Error;

pub type JupiterResult<T> = Result<T, JupiterError>;

#[derive(Debug, Error)]
pub enum JupiterError {
    #[error("jupiter HTTP transport error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("jupiter upstream returned HTTP {status}: {body}")]
    Upstream { status: u16, body: String },

    #[error("jupiter response JSON parse failed: {0}")]
    JsonParse(#[from] serde_json::Error),

    #[error("invalid transaction payload: {0}")]
    Transaction(String),

    #[error("invalid pubkey `{pubkey}`: {reason}")]
    Pubkey { pubkey: String, reason: String },

    #[error("failed to resolve address lookup tables: {0}")]
    LookupResolution(String),

    #[error("jupiter extension is disabled in this surfpool instance")]
    Disabled,
}
