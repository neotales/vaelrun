//! Nushell commands.
//!
//! The module is `nushell`; the executable it runs is `nu`.

use std::ffi::{OsStr, OsString};
use std::sync::Once;

use neotales_exec::{Command, Executable, register};

use crate::common;

const NAME: &str = "nu";
const EXTENSIONS: [&str; 1] = [".nu"];
const VARIABLE: &str = "RUN_NU_EXE";

static REGISTER: Once = Once::new();

fn ensure_registered() {
    REGISTER.call_once(|| {
        register(
            NAME,
            Executable {
                name: NAME.to_owned(),
                variable: VARIABLE.to_owned(),
                windows: vec![
                    "${ProgramFiles}\\nu\\nu.exe".to_owned(),
                    "${LOCALAPPDATA}\\Programs\\nu\\nu.exe".to_owned(),
                ],
                linux: vec![
                    "/usr/bin/nu".to_owned(),
                    "/usr/local/bin/nu".to_owned(),
                    "${HOME}/.cargo/bin/nu".to_owned(),
                    "${HOME}/.local/bin/nu".to_owned(),
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

/// Builds `nu` with `args` and no script.
pub fn new<'a>(args: impl IntoIterator<Item = &'a str>) -> Command {
    build(args.into_iter().map(OsString::from).collect())
}

/// Builds `nu <path> <args>`.
pub fn file<'a>(path: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    file_with(
        OsStr::new(path),
        args.into_iter().map(OsString::from).collect(),
    )
}

fn file_with(path: &OsStr, args: Vec<OsString>) -> Command {
    let mut all: Vec<OsString> = vec![path.to_os_string()];
    all.extend(args);
    build(all)
}

/// Writes `script` to a private `.nu` file and builds `nu <file> <args>`.
///
/// `nu -c` does not reliably forward the caller's arguments into `main`, so the source is
/// materialised and run as a file. The file is left in place, and the path is recorded on the
/// command for [`Command::temp_file`].
///
/// # Errors
///
/// Returns the I/O error when the private file cannot be written.
pub fn inline<'a>(
    script: &str,
    args: impl IntoIterator<Item = &'a str>,
) -> std::io::Result<Command> {
    let path = common::temp_file(script, "nu")?;
    let command = file_with(
        path.as_os_str(),
        args.into_iter().map(OsString::from).collect(),
    );
    Ok(command.with_temp_file(path))
}

/// Runs `text` as a file when it is one line ending in `.nu`, and inline otherwise.
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
