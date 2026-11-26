//! Windows PowerShell commands.

use std::ffi::OsString;
use std::sync::Once;

use neotales_exec::{Command, Executable, register};

use crate::common;

const NAME: &str = "powershell";
const SCRIPT_ARGS: [&str; 4] = ["-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass"];
const VARIABLE: &str = "POWERSHELL_EXE";

static REGISTER: Once = Once::new();

fn ensure_registered() {
    REGISTER.call_once(|| {
        register(
            NAME,
            Executable {
                name: NAME.to_owned(),
                variable: VARIABLE.to_owned(),
                windows: vec![
                    "${SystemRoot}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe".to_owned(),
                ],
                linux: vec!["/bin/pwsh".to_owned(), "/usr/bin/pwsh".to_owned()],
                ..Executable::default()
            },
        );
    });
}

fn build(args: Vec<OsString>) -> Command {
    ensure_registered();
    common::command(NAME, args)
}

/// Builds `powershell` with `args` and no script.
pub fn new<'a>(args: impl IntoIterator<Item = &'a str>) -> Command {
    build(args.into_iter().map(OsString::from).collect())
}

/// Builds `powershell -NoLogo -NoProfile -ExecutionPolicy Bypass -File <path> <args>`.
pub fn file<'a>(path: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    file_with_shell_args(path, SCRIPT_ARGS, args)
}

/// Builds `powershell <shell_args> -File <path> <args>`.
///
/// `shell_args` replaces the default `-NoLogo -NoProfile -ExecutionPolicy Bypass` prefix, and an empty list
/// means no prefix; [`file()`] is the default shape.
pub fn file_with_shell_args<'a, S, A>(path: &str, shell_args: S, args: A) -> Command
where
    S: IntoIterator<Item = &'a str>,
    A: IntoIterator<Item = &'a str>,
{
    let mut all: Vec<OsString> = shell_args.into_iter().map(OsString::from).collect();
    all.push("-File".into());
    all.push(path.into());
    all.extend(args.into_iter().map(OsString::from));
    build(all)
}

/// Builds `powershell -NoLogo -NoProfile -ExecutionPolicy Bypass -Command <script> <args>`.
pub fn inline<'a>(script: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    inline_with_shell_args(script, SCRIPT_ARGS, args)
}

/// Builds `powershell <shell_args> -Command <script> <args>`.
///
/// `shell_args` replaces the default `-NoLogo -NoProfile -ExecutionPolicy Bypass` prefix, and an empty list
/// means no prefix; [`inline()`] is the default shape.
pub fn inline_with_shell_args<'a, S, A>(script: &str, shell_args: S, args: A) -> Command
where
    S: IntoIterator<Item = &'a str>,
    A: IntoIterator<Item = &'a str>,
{
    let mut all: Vec<OsString> = shell_args.into_iter().map(OsString::from).collect();
    all.push("-Command".into());
    all.push(script.into());
    all.extend(args.into_iter().map(OsString::from));
    build(all)
}

/// Runs `text` as a file when it is one line ending in `.ps1`, and inline otherwise.
pub fn script<'a>(text: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    match common::script_file(text, &[".ps1"]) {
        Some(path) => file(path, args),
        None => inline(text, args),
    }
}
