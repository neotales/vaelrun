//! Go commands.
//!
//! The module is `golang` because `go` is a Rust keyword.

use std::ffi::OsString;
use std::sync::Once;

use neotales_exec::{Command, Executable, register};

use crate::common;

const NAME: &str = "go";
const EXTENSIONS: [&str; 1] = [".go"];

#[cfg(windows)]
const VARIABLE: &str = "RUN_WIN_GO_EXE";
#[cfg(not(windows))]
const VARIABLE: &str = "RUN_GO_EXE";

static REGISTER: Once = Once::new();

fn ensure_registered() {
    REGISTER.call_once(|| {
        register(
            NAME,
            Executable {
                name: NAME.to_owned(),
                variable: VARIABLE.to_owned(),
                windows: vec![
                    "${ProgramFiles}\\Go\\bin\\go.exe".to_owned(),
                    "${ChocolateyInstall}\\lib\\go\\tools\\go\\bin\\go.exe".to_owned(),
                ],
                linux: vec![
                    "${HOME}/.local/shared/go/bin/go".to_owned(),
                    "/usr/local/go/bin/go".to_owned(),
                    "/usr/local/bin/go".to_owned(),
                    "/usr/bin/go".to_owned(),
                ],
                ..Executable::default()
            },
        );
    });
}

fn build(args: Vec<OsString>) -> Command {
    ensure_registered();
    common::command(NAME, args)
}

/// Builds `go` with `args` and no script.
pub fn new<'a>(args: impl IntoIterator<Item = &'a str>) -> Command {
    build(args.into_iter().map(OsString::from).collect())
}

/// Builds `go run <path> <args>`.
pub fn file<'a>(path: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    let mut all: Vec<OsString> = vec!["run".into(), path.into()];
    all.extend(args.into_iter().map(OsString::from));
    build(all)
}

/// Writes `script` to a fresh private file in the user's directory and builds
/// `go run <file> <args>`.
///
/// On Windows privacy relies on the per-user `LocalAppData` directory and reparse-point refusal,
/// not a custom ACL.
///
/// The file is left in place, matching the reference implementation, and the path is recorded on
/// the command for [`Command::temp_file`].
///
/// # Errors
///
/// Returns the I/O error when the private directory or the file cannot be created or written.
pub fn inline<'a>(
    script: &str,
    args: impl IntoIterator<Item = &'a str>,
) -> std::io::Result<Command> {
    let path = common::temp_file(script, "go")?;
    let mut all: Vec<OsString> = vec!["run".into(), path.clone().into_os_string()];
    all.extend(args.into_iter().map(OsString::from));
    Ok(build(all).with_temp_file(path))
}

/// Runs `text` as a file when it is one line ending in `.go`, and inline otherwise.
///
/// # Errors
///
/// Returns the I/O error from [`inline()`] when the source has to be written.
pub fn script<'a>(text: &str, args: impl IntoIterator<Item = &'a str>) -> std::io::Result<Command> {
    match common::script_file(text, &EXTENSIONS) {
        Some(path) => Ok(file(path, args)),
        None => inline(text, args),
    }
}
