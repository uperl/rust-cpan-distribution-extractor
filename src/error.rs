//! The crate's error type.

use std::path::PathBuf;

use crate::Format;

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Anything that can go wrong while detecting or unpacking an archive.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A filesystem or stream I/O failure, optionally naming the path involved.
    #[error("i/o error{}: {source}", .path.as_ref().map(|p| format!(" ({})", p.display())).unwrap_or_default())]
    Io {
        /// The path being operated on, when one is known.
        path: Option<PathBuf>,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },

    /// The archive's format could not be recognised from its bytes or its name.
    #[error("{path}: unrecognised archive format")]
    UnknownFormat {
        /// The archive that could not be identified.
        path: PathBuf,
    },

    /// The archive format is understood but support for it was compiled out.
    #[error(
        "{format} archives are not supported in this build; \
         rebuild `cpan-distribution-extractor` with the `{feature}` feature"
    )]
    UnsupportedFormat {
        /// The detected format.
        format: Format,
        /// The Cargo feature that would enable it.
        feature: &'static str,
    },

    /// The archive does not unpack to exactly one top-level directory, as a
    /// CPAN distribution release must.
    #[error("archive does not unpack to a single top-level directory ({saw})")]
    Layout {
        /// A short description of what was seen instead.
        saw: String,
    },

    /// An archive member has an unsafe path (absolute, or escaping the archive
    /// root with `..`).
    #[error("unsafe archive member path: {member}")]
    UnsafeMember {
        /// The offending member path.
        member: PathBuf,
    },

    /// The `zip` reader rejected the archive.
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
}

impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        Error::Io { path: None, source }
    }
}

impl Error {
    /// An [`Error::Io`] that names the path it happened on.
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: Some(path.into()),
            source,
        }
    }
}
