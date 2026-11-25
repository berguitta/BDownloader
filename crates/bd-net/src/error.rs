use thiserror::Error;

/// Errors returned by the download engine.
#[derive(Debug, Error)]
pub enum Error {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("server answered with HTTP status {0}")]
    Status(u16),

    #[error("server ignored the requested byte range")]
    RangeNotHonored,

    #[error("server sent more data than announced")]
    TooMuchData,

    #[error("connection closed before the segment was complete")]
    Incomplete,

    #[error("resume data is invalid: {0}")]
    State(#[from] serde_json::Error),

    #[error("download cancelled")]
    Cancelled,
}

impl Error {
    /// Whether retrying the same request may succeed.
    pub fn is_transient(&self) -> bool {
        match self {
            // A body cut mid-transfer surfaces as a decode error in reqwest.
            Error::Http(e) => e.is_timeout() || e.is_connect() || e.is_request() || e.is_body() || e.is_decode(),
            Error::Status(code) => *code == 408 || *code == 429 || *code >= 500,
            Error::Incomplete => true,
            _ => false,
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
