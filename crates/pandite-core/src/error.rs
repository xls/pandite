//! Shared error type.

use std::fmt;

/// Result alias used across pandite crates.
pub type Result<T> = std::result::Result<T, Error>;

/// Every failure a provider or operation can report.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// No registered provider accepts the data or extension.
    Unsupported(String),
    /// The data is recognized but damaged or truncated.
    Corrupt(String),
    /// The image is larger than the configured limits allow.
    TooLarge {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
    },
    /// Arguments are outside the valid range (zero size, crop outside image).
    Invalid(String),
    /// The caller cancelled the work through a [`crate::CancelToken`].
    Cancelled,
    /// An operating system I/O failure.
    Io(std::io::Error),
    /// A platform codec or GPU backend failure.
    Backend(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported(what) => write!(f, "unsupported format: {what}"),
            Error::Corrupt(what) => write!(f, "corrupt data: {what}"),
            Error::TooLarge { width, height } => {
                write!(f, "image too large: {width} x {height}")
            }
            Error::Invalid(what) => write!(f, "invalid argument: {what}"),
            Error::Cancelled => f.write_str("cancelled"),
            Error::Io(err) => write!(f, "i/o error: {err}"),
            Error::Backend(what) => write!(f, "backend error: {what}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_names_the_cause() {
        let err = Error::TooLarge {
            width: 70000,
            height: 2,
        };
        assert_eq!(err.to_string(), "image too large: 70000 x 2");
        let io: Error = std::io::Error::new(std::io::ErrorKind::NotFound, "gone").into();
        assert!(io.to_string().contains("gone"));
        assert!(std::error::Error::source(&io).is_some());
    }
}
