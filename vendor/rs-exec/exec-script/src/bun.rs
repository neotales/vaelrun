//! Bun commands.

use std::ffi::OsString;
use std::sync::Once;

use neotales_exec::{Command, Executable, register};

use crate::common;

const NAME: &str = "bun";
const EXTENSIONS: [&str; 4] = [".js", ".mjs", ".cjs", ".ts"];

#[cfg(windows)]
const VARIABLE: &str = "RUN_WIN_BUN_EXE";
#[cfg(not(windows))]
const VARIABLE: &str = "RUN_BUN_EXE";

static REGISTER: Once = Once::new();

fn ensure_registered() {
    REGISTER.call_once(|| {
        register(
            NAME,
            Executable {
                name: NAME.to_owned(),
                variable: VARIABLE.to_owned(),
                windows: vec![
                    "${USERPROFILE}\\.bun\\bin\\bun.exe".to_owned(),
                    "${LOCALAPPDATA}\\Programs\\bin\\bun.exe".to_owned(),
                    "${LOCALAPPDATA}\\Microsoft\\WinGet\\Links\\bin.exe".to_owned(),
                    "${ProgramFiles}\\bun\\bin\\bun.exe".to_owned(),
                    "${ProgramFiles(x86)}\\bun\\bin\\bun.exe".to_owned(),
                ],
                linux: vec![
                    "${HOME}/.bun/bin/bun".to_owned(),
                    "${HOME}/.local/bin/bun".to_owned(),
                    "/usr/bin/bun".to_owned(),
                    "/usr/local/bin/bun".to_owned(),
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

/// Builds `bun` with `args` and no script.
pub fn new<'a>(args: impl IntoIterator<Item = &'a str>) -> Command {
    build(args.into_iter().map(OsString::from).collect())
}

/// Builds `bun <path> <args>`.
pub fn file<'a>(path: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    let mut all: Vec<OsString> = vec![path.into()];
    all.extend(args.into_iter().map(OsString::from));
    build(all)
}

/// Builds `bun -e <script> <args>`.
pub fn inline<'a>(script: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    let mut all: Vec<OsString> = vec!["-e".into(), script.into()];
    all.extend(args.into_iter().map(OsString::from));
    build(all)
}

/// Runs `text` as a file when it is one line ending in a JavaScript extension, and inline
/// otherwise.
pub fn script<'a>(text: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    match common::script_file(text, &EXTENSIONS) {
        Some(path) => file(path, args),
        None => inline(text, args),
    }
}
