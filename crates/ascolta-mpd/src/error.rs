/// Errors returned by the MPD client.
#[derive(Debug, thiserror::Error)]
pub enum MpdError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("MPD error {code} in `{command}`: {message}")]
    Ack {
        code: u32,
        command: String,
        message: String,
    },

    #[error("connection closed by MPD")]
    Closed,

    #[error("response exceeds the {0}-byte safety limit")]
    TooLarge(usize),
}

impl MpdError {
    /// True when the error means the connection is unusable.
    pub fn is_connection_lost(&self) -> bool {
        matches!(self, Self::Io(_) | Self::Closed)
    }

    pub fn ack_message(&self) -> Option<&str> {
        match self {
            Self::Ack { message, .. } => Some(message),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, MpdError>;
