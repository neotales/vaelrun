//! Lua commands.

use std::ffi::OsString;
use std::sync::Once;

use neotales_exec::{Command, Executable, register};

use crate::common;

const NAME: &str = "lua";
const EXTENSIONS: [&str; 1] = [".lua"];
const VARIABLE: &str = "RUN_LUA_EXE";

static REGISTER: Once = Once::new();

fn ensure_registered() {
    REGISTER.call_once(|| {
        register(
            NAME,
            Executable {
                name: NAME.to_owned(),
                variable: VARIABLE.to_owned(),
                windows: vec!["${ProgramFiles}\\Lua\\lua.exe".to_owned()],
                linux: vec![
                    "/usr/bin/lua".to_owned(),
                    "/usr/bin/lua5.4".to_owned(),
                    "/usr/bin/lua5.3".to_owned(),
                    "/usr/local/bin/lua".to_owned(),
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

/// Builds `lua` with `args` and no script.
pub fn new<'a>(args: impl IntoIterator<Item = &'a str>) -> Command {
    build(args.into_iter().map(OsString::from).collect())
}

/// Builds `lua <path> <args>`.
pub fn file<'a>(path: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    let mut all: Vec<OsString> = vec![path.into()];
    all.extend(args.into_iter().map(OsString::from));
    build(all)
}

/// Builds `lua -e <script> <args>`.
///
/// Caller arguments are appended unchanged, and Lua may treat a non-option argument as a script
/// filename to open, or consume a flag-like argument itself. Use [`file()`] to pass arguments to a
/// script.
pub fn inline<'a>(script: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    let mut all: Vec<OsString> = vec!["-e".into(), script.into()];
    all.extend(args.into_iter().map(OsString::from));
    build(all)
}

/// Runs `text` as a file when it is one line ending in `.lua`, and inline otherwise.
pub fn script<'a>(text: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    match common::script_file(text, &EXTENSIONS) {
        Some(path) => file(path, args),
        None => inline(text, args),
    }
}
