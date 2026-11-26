//! Windows `cmd` commands.
//!
//! There is no `cmd` runtime in the reference implementation; this module adds it with the same
//! shape as the others. On a system without `cmd`, [`new`] resolves to the bare name and the spawn
//! reports the missing program.

use std::ffi::OsString;
use std::sync::Once;

use neotales_exec::{Command, Executable, register};

use crate::common;

const NAME: &str = "cmd";
const SCRIPT_ARGS: [&str; 2] = ["/d", "/c"];
const EXTENSIONS: [&str; 2] = [".bat", ".cmd"];
const VARIABLE: &str = "COMSPEC";

static REGISTER: Once = Once::new();

fn ensure_registered() {
    REGISTER.call_once(|| {
        register(
            NAME,
            Executable {
                name: NAME.to_owned(),
                variable: VARIABLE.to_owned(),
                windows: vec!["${SystemRoot}\\System32\\cmd.exe".to_owned()],
                ..Executable::default()
            },
        );
    });
}

fn build(args: Vec<OsString>) -> Command {
    ensure_registered();
    common::command(NAME, args)
}

/// Builds `cmd` with `args` and no script.
pub fn new<'a>(args: impl IntoIterator<Item = &'a str>) -> Command {
    build(args.into_iter().map(OsString::from).collect())
}

/// Builds `cmd /d /c <path> <args>`.
pub fn file<'a>(path: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    file_with_shell_args(path, SCRIPT_ARGS, args)
}

/// Builds `cmd <shell_args> <path> <args>`.
///
/// `shell_args` replaces the default `/d /c` prefix, and an empty list means no prefix; [`file()`] is
/// the default shape. Replacing the prefix also drops `/c`, so pass `/c` back unless the caller
/// wants something else.
pub fn file_with_shell_args<'a, S, A>(path: &str, shell_args: S, args: A) -> Command
where
    S: IntoIterator<Item = &'a str>,
    A: IntoIterator<Item = &'a str>,
{
    let mut all: Vec<OsString> = shell_args.into_iter().map(OsString::from).collect();
    all.push(path.into());
    all.extend(args.into_iter().map(OsString::from));
    build(all)
}

/// Builds `cmd /d /c <script> <args>`.
pub fn inline<'a>(script: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    inline_with_shell_args(script, SCRIPT_ARGS, args)
}

/// Builds `cmd <shell_args> <script> <args>`.
///
/// `shell_args` replaces the default `/d /c` prefix, and an empty list means no prefix; [`inline()`]
/// is the default shape. Replacing the prefix also drops `/c`, so pass `/c` back unless the caller
/// wants something else.
pub fn inline_with_shell_args<'a, S, A>(script: &str, shell_args: S, args: A) -> Command
where
    S: IntoIterator<Item = &'a str>,
    A: IntoIterator<Item = &'a str>,
{
    let mut all: Vec<OsString> = shell_args.into_iter().map(OsString::from).collect();
    all.push(script.into());
    all.extend(args.into_iter().map(OsString::from));
    build(all)
}

/// Runs `text` as a file when it is one line ending in `.bat` or `.cmd`, and inline otherwise.
pub fn script<'a>(text: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    match common::script_file(text, &EXTENSIONS) {
        Some(path) => file(path, args),
        None => inline(text, args),
    }
}
