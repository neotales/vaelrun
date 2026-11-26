//! Julia commands.

use std::ffi::OsString;
use std::sync::Once;

use neotales_exec::{Command, Executable, register};

use crate::common;

const NAME: &str = "julia";
const EXTENSIONS: [&str; 1] = [".jl"];
const VARIABLE: &str = "RUN_JULIA_EXE";

static REGISTER: Once = Once::new();

fn ensure_registered() {
    REGISTER.call_once(|| {
        register(
            NAME,
            Executable {
                name: NAME.to_owned(),
                variable: VARIABLE.to_owned(),
                windows: vec![
                    "${LOCALAPPDATA}\\Programs\\Julia\\bin\\julia.exe".to_owned(),
                    "${ProgramFiles}\\Julia\\bin\\julia.exe".to_owned(),
                ],
                linux: vec![
                    "/usr/bin/julia".to_owned(),
                    "/usr/local/bin/julia".to_owned(),
                    "${HOME}/.juliaup/bin/julia".to_owned(),
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

/// Builds `julia` with `args` and no script.
pub fn new<'a>(args: impl IntoIterator<Item = &'a str>) -> Command {
    build(args.into_iter().map(OsString::from).collect())
}

/// Builds `julia <path> <args>`.
pub fn file<'a>(path: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    let mut all: Vec<OsString> = vec![path.into()];
    all.extend(args.into_iter().map(OsString::from));
    build(all)
}

/// Builds `julia -e <script> -- <args>`.
///
/// `--` is always passed, so a caller argument that looks like a Julia flag reaches `ARGS`.
pub fn inline<'a>(script: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    let mut all: Vec<OsString> = vec!["-e".into(), script.into(), "--".into()];
    all.extend(args.into_iter().map(OsString::from));
    build(all)
}

/// Runs `text` as a file when it is one line ending in `.jl`, and inline otherwise.
pub fn script<'a>(text: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    match common::script_file(text, &EXTENSIONS) {
        Some(path) => file(path, args),
        None => inline(text, args),
    }
}
