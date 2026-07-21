use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("unsupported file type: {0}")]
    UnsupportedFileType(String),

    #[error("parse error in {file}: {message}")]
    Parse { file: String, message: String },

    #[error("index error: {0}")]
    Index(String),

    #[error("embedding error: {0}")]
    Embedding(String),

    #[error("knowledge source error: {0}")]
    Source(String),

    #[error("assembly error: {0}")]
    Assembly(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, CoreError>;
