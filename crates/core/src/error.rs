use std::fmt;
use std::io;

#[derive(Debug)]
pub enum AppError {
    Io(io::Error),
    InvalidInput(String),
    /// A plugin failed to load, or one of its nodes failed while processing.
    Plugin(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::InvalidInput(message) => write!(f, "invalid input: {message}"),
            Self::Plugin(message) => write!(f, "plugin error: {message}"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<io::Error> for AppError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}
