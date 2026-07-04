//! Error type shared across the crate.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("could not determine your home directory")]
    NoHomeDir,

    #[error("profile '{0}' does not exist")]
    UnknownProfile(String),

    #[error("a profile named '{0}' already exists")]
    DuplicateProfile(String),

    #[error("'{0}' is not a valid profile name (use letters, digits, '-' or '_')")]
    InvalidName(String),

    #[error("cannot delete the last remaining profile while it is active")]
    CannotDeleteLastActive,

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("could not parse {path}: {source}")]
    Metadata {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}
