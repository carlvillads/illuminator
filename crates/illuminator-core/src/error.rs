//! error.rs
//!
//! The errors of illuminator-core, and a `Result` that uses them.
//!

use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// User cancelled
    #[error("Loading cancelled")]
    Cancelled,
    /// The file is malformed, unsupported or too large
    #[error("{0}")]
    Invalid(String),
    /// OS refused a file operation
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl Error {
    /// "Error::Invalid(message.into())" shorthand
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
