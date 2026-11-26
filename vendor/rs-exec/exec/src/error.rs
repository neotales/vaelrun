//! The error type shared by commands, pipelines, and executable lookups.

use std::ffi::OsStr;
use std::fmt;
use std::io;

/// A command or a lookup that could not start, could not finish, or exited non-zero.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The command text had no program name in it.
    EmptyCommand,
    /// The program could not be started.
    Spawn {
        /// The program that was not started.
        program: std::ffi::OsString,
        /// What the operating system reported.
        source: io::Error,
    },
    /// The program started but its exit status could not be collected.
    Wait {
        /// The program whose status was not collected.
        program: std::ffi::OsString,
        /// What the operating system reported.
        source: io::Error,
    },
    /// The program ended without an exit code, because a signal killed it.
    Terminated {
        /// The program that was killed.
        program: std::ffi::OsString,
    },
    /// The program exited with a non-zero code.
    ///
    /// [`crate::Output::to_error`] produces this from an [`crate::Output`]; a non-zero exit is not
    /// an error until the caller asks for one.
    Failed {
        /// The program that exited non-zero.
        program: std::ffi::OsString,
        /// The exit code.
        code: i32,
        /// Standard error as text, possibly empty.
        stderr: String,
    },
    /// No executable was found for a registered name.
    NotFound {
        /// The name that was looked up.
        name: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyCommand => formatter.write_str("the command text has no program"),
            Self::Spawn { program, source } => {
                write!(formatter, "failed to start {}: {source}", display(program))
            }
            Self::Wait { program, source } => {
                write!(
                    formatter,
                    "failed to wait for {}: {source}",
                    display(program)
                )
            }
            Self::Terminated { program } => {
                write!(formatter, "{} was terminated by a signal", display(program))
            }
            Self::Failed {
                program,
                code,
                stderr,
            } => {
                let stderr = stderr.trim();
                if stderr.is_empty() {
                    write!(formatter, "{} failed with code {code}", display(program))
                } else {
                    write!(
                        formatter,
                        "{} failed with code {code}: {stderr}",
                        display(program)
                    )
                }
            }
            Self::NotFound { name } => write!(formatter, "executable not found: {name}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } | Self::Wait { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn display(program: &OsStr) -> String {
    program.to_string_lossy().into_owned()
}
