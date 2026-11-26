//! Python commands.

use std::ffi::OsString;
use std::sync::Once;

use neotales_exec::{Command, Executable, register};

use crate::common;

const NAME: &str = "python";
const EXTENSIONS: [&str; 2] = [".py", ".pyw"];
const VARIABLE: &str = "RUN_PYTHON_EXE";

static REGISTER: Once = Once::new();

fn ensure_registered() {
    REGISTER.call_once(|| {
        register(
            NAME,
            Executable {
                name: NAME.to_owned(),
                variable: VARIABLE.to_owned(),
                windows: vec![
                    "${SystemRoot}\\py.exe".to_owned(),
                    "${LOCALAPPDATA}\\Programs\\Python\\Launcher\\py.exe".to_owned(),
                    "${ProgramFiles}\\Python\\python.exe".to_owned(),
                ],
                linux: vec![
                    "/usr/bin/python3".to_owned(),
                    "/usr/local/bin/python3".to_owned(),
                    "/usr/bin/python".to_owned(),
                    "/usr/local/bin/python".to_owned(),
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

/// Builds `python` with `args` and no script.
pub fn new<'a>(args: impl IntoIterator<Item = &'a str>) -> Command {
    build(args.into_iter().map(OsString::from).collect())
}

/// Builds `python <path> <args>`.
pub fn file<'a>(path: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    let mut all: Vec<OsString> = vec![path.into()];
    all.extend(args.into_iter().map(OsString::from));
    build(all)
}

/// Builds `python -c <script> <args>`.
///
/// No `$0` placeholder is inserted: Python puts the first caller argument in `sys.argv[1]`.
pub fn inline<'a>(script: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    let mut all: Vec<OsString> = vec!["-c".into(), script.into()];
    all.extend(args.into_iter().map(OsString::from));
    build(all)
}

/// Runs `text` as a file when it is one line ending in a Python extension, and inline otherwise.
pub fn script<'a>(text: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    match common::script_file(text, &EXTENSIONS) {
        Some(path) => file(path, args),
        None => inline(text, args),
    }
}
