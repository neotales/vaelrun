use std::fmt;
use std::path::Path;

/// A failure vrun reports on stderr, with the process exit code.
#[derive(Debug)]
pub struct Error {
    /// Process exit code: 1, 2, 126, or 127.
    pub code: i32,
    /// Human-readable cause. Does not echo secret values.
    pub message: String,
}

impl Error {
    pub(crate) fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self::new(2, message)
    }

    pub(crate) fn trust(program: &str) -> Self {
        Self::new(126, format!("trust refused `{program}`"))
    }

    pub(crate) fn not_found(name: &str) -> Self {
        Self::new(127, format!("not found: {name}"))
    }

    pub(crate) fn cannot_execute(name: &str) -> Self {
        Self::new(126, format!("cannot execute: {name}"))
    }

    pub(crate) fn io(path: &Path, err: &std::io::Error) -> Self {
        Self::new(1, format!("{}: {err}", path.display()))
    }

    pub(crate) fn at(path: &Path, line: usize, reason: &str) -> Self {
        Self::new(1, format!("{}:{line}: {reason}", path.display()))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}
