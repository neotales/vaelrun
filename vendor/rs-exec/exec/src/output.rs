//! Captured output from one process or a whole pipeline.

use std::ffi::OsString;
use std::time::SystemTime;

use crate::error::Error;

/// One process in a single command or in a pipeline.
#[derive(Clone, Debug)]
pub struct Stage {
    /// The program as the caller set it.
    pub program: OsString,
    /// The arguments, in order.
    pub args: Vec<OsString>,
    /// The exit code of this stage.
    ///
    /// A stage killed by a signal reports `128` plus the signal number, the way a shell does. Only
    /// the last stage decides whether [`Output::is_ok`] is true.
    pub code: i32,
    /// The bytes this stage wrote to stdout that were captured.
    ///
    /// A middle stage of a pipeline hands its stdout to the next stage, so this is empty there.
    pub stdout: Vec<u8>,
    /// The bytes this stage wrote to stderr that were captured.
    pub stderr: Vec<u8>,
}

/// What a command or pipeline produced.
///
/// The top-level fields describe the last stage; every stage is also in [`Output::stages`]. A
/// non-zero exit is data: [`Output::is_ok`] reports it and [`Output::to_error`] turns it into an
/// [`Error`] when the caller wants one.
#[derive(Clone, Debug)]
pub struct Output {
    /// Bytes the last stage wrote to stdout, empty when stdout was inherited or discarded.
    pub stdout: Vec<u8>,
    /// Bytes the last stage wrote to stderr, empty when stderr was inherited or discarded.
    pub stderr: Vec<u8>,
    /// The exit code of the last stage.
    pub code: i32,
    /// The program of the last stage, as the caller set it.
    pub program: OsString,
    /// The arguments of the last stage, in order.
    pub args: Vec<OsString>,
    /// When the first process was ready to start.
    pub started_at: SystemTime,
    /// When the last process had been waited for.
    pub ended_at: SystemTime,
    /// One entry per stage. A single command has one; a pipeline has every stage.
    pub stages: Vec<Stage>,
}

impl Output {
    /// Standard output as text, with invalid UTF-8 replaced.
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// Standard error as text, with invalid UTF-8 replaced.
    #[must_use]
    pub fn error_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    /// Whether the last stage exited with code 0.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.code == 0
    }

    /// Turns a non-zero exit into an error naming the program and carrying its stderr.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Failed`] when [`Output::is_ok`] is false.
    pub fn to_error(&self) -> Result<(), Error> {
        if self.is_ok() {
            return Ok(());
        }
        Err(Error::Failed {
            program: self.program.clone(),
            code: self.code,
            stderr: self.error_text(),
        })
    }

    /// Standard output split into lines.
    ///
    /// Lines end at `\n` and at `\r\n`; the `\r` is not part of the line. A trailing newline does
    /// not produce a final empty line, and empty output has no lines.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        split_lines(&self.stdout)
    }

    /// Standard error split into lines, using the same rules as [`Output::lines`].
    #[must_use]
    pub fn error_lines(&self) -> Vec<String> {
        split_lines(&self.stderr)
    }
}

fn split_lines(bytes: &[u8]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            lines.push(line(&bytes[start..index]));
            start = index + 1;
        }
    }
    if start < bytes.len() {
        lines.push(line(&bytes[start..]));
    }
    lines
}

fn line(bytes: &[u8]) -> String {
    let bytes = match bytes.last() {
        Some(b'\r') => &bytes[..bytes.len() - 1],
        _ => bytes,
    };
    String::from_utf8_lossy(bytes).into_owned()
}
