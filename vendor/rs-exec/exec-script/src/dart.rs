//! Dart commands.

use std::ffi::{OsStr, OsString};
use std::sync::Once;

use neotales_exec::{Command, Executable, register};

use crate::common;

const NAME: &str = "dart";
const EXTENSIONS: [&str; 1] = [".dart"];
const VARIABLE: &str = "RUN_DART_EXE";

static REGISTER: Once = Once::new();

fn ensure_registered() {
    REGISTER.call_once(|| {
        register(
            NAME,
            Executable {
                name: NAME.to_owned(),
                variable: VARIABLE.to_owned(),
                windows: vec!["${ProgramFiles}\\dart-sdk\\bin\\dart.exe".to_owned()],
                linux: vec![
                    "/usr/bin/dart".to_owned(),
                    "/usr/lib/dart/bin/dart".to_owned(),
                    "/usr/local/bin/dart".to_owned(),
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

/// Builds `dart` with `args` and no script.
pub fn new<'a>(args: impl IntoIterator<Item = &'a str>) -> Command {
    build(args.into_iter().map(OsString::from).collect())
}

/// Builds `dart run <path> <args>`.
pub fn file<'a>(path: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    file_with(
        OsStr::new(path),
        args.into_iter().map(OsString::from).collect(),
    )
}

fn file_with(path: &OsStr, args: Vec<OsString>) -> Command {
    let mut all: Vec<OsString> = vec!["run".into(), path.to_os_string()];
    all.extend(args);
    build(all)
}

/// Writes `script` to a private `.dart` file and builds `dart run <file> <args>`.
///
/// Dart has no stable eval flag, so the source is materialised before the run. The file is left in
/// place, and the path is recorded on the command for [`Command::temp_file`].
///
/// # Errors
///
/// Returns the I/O error when the private file cannot be written.
pub fn inline<'a>(
    script: &str,
    args: impl IntoIterator<Item = &'a str>,
) -> std::io::Result<Command> {
    let path = common::temp_file(script, "dart")?;
    let command = file_with(
        path.as_os_str(),
        args.into_iter().map(OsString::from).collect(),
    );
    Ok(command.with_temp_file(path))
}

/// Runs `text` as a file when it is one line ending in `.dart`, and inline otherwise.
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
