use std::fmt;

#[derive(Debug)]
pub enum AiError {
    Cancelled,
    Network(String),
    Status(u16, String),
    Protocol(String),
    Refused,
    Secrets(String),
    Cli(String),
    Unavailable(String),
}

impl fmt::Display for AiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("cancelled"),
            Self::Network(e) => write!(f, "network error: {e}"),
            Self::Status(code, msg) => write!(f, "HTTP {code}: {msg}"),
            Self::Protocol(e) => write!(f, "unexpected response: {e}"),
            Self::Refused => f.write_str("the model declined to answer"),
            Self::Secrets(e) => write!(f, "secret store: {e}"),
            Self::Cli(e) => write!(f, "claude cli: {e}"),
            Self::Unavailable(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for AiError {}
