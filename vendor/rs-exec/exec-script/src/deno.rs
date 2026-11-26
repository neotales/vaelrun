//! Deno commands.

use std::ffi::OsString;
use std::sync::Once;

use neotales_exec::{Command, Executable, register};

use crate::common;

const NAME: &str = "deno";
const FILE_ARGS: [&str; 2] = ["run", "-A"];
const EXTENSIONS: [&str; 4] = [".js", ".mjs", ".cjs", ".ts"];
const VARIABLE: &str = "RUN_DENO_EXE";

static REGISTER: Once = Once::new();

fn ensure_registered() {
    REGISTER.call_once(|| {
        register(
            NAME,
            Executable {
                name: NAME.to_owned(),
                variable: VARIABLE.to_owned(),
                windows: vec![
                    "C:\\Program Files\\deno\\deno.exe".to_owned(),
                    "${USERPROFILE}\\.deno\\bin\\deno.exe".to_owned(),
                ],
                linux: vec![
                    "/usr/bin/deno".to_owned(),
                    "/usr/local/bin/deno".to_owned(),
                    "${HOME}/.deno/bin/deno".to_owned(),
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

/// Builds `deno` with `args` and no script.
pub fn new<'a>(args: impl IntoIterator<Item = &'a str>) -> Command {
    build(args.into_iter().map(OsString::from).collect())
}

/// Builds `deno run -A <path> <args>`.
///
/// `-A` is cast's allow-all flag, kept as the default the reference implementation uses.
pub fn file<'a>(path: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    file_with_shell_args(path, FILE_ARGS, args)
}

/// Builds `deno <shell_args> <path> <args>`.
///
/// `shell_args` replaces the default `run -A` prefix, and an empty list means no prefix; [`file()`]
/// is the default shape.
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

/// Builds `deno eval <script> <args>`.
pub fn inline<'a>(script: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    inline_with_shell_args(script, [], args)
}

/// Builds `deno <shell_args> eval <script> <args>`.
///
/// [`inline()`] has no prefix today, so `shell_args` is inserted before `eval`; an empty list means
/// no prefix, and a caller who wants the default uses [`inline()`].
pub fn inline_with_shell_args<'a, S, A>(script: &str, shell_args: S, args: A) -> Command
where
    S: IntoIterator<Item = &'a str>,
    A: IntoIterator<Item = &'a str>,
{
    let mut all: Vec<OsString> = shell_args.into_iter().map(OsString::from).collect();
    all.push("eval".into());
    all.push(script.into());
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
