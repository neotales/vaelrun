//! Perl commands.

use std::ffi::OsString;
use std::sync::Once;

use neotales_exec::{Command, Executable, register};

use crate::common;

const NAME: &str = "perl";
const EXTENSIONS: [&str; 1] = [".pl"];
const VARIABLE: &str = "RUN_PERL_EXE";

static REGISTER: Once = Once::new();

fn ensure_registered() {
    REGISTER.call_once(|| {
        register(
            NAME,
            Executable {
                name: NAME.to_owned(),
                variable: VARIABLE.to_owned(),
                windows: vec![
                    "${SystemDrive}\\Strawberry\\perl\\bin\\perl.exe".to_owned(),
                    "${ProgramFiles}\\Perl\\bin\\perl.exe".to_owned(),
                ],
                linux: vec!["/usr/bin/perl".to_owned(), "/usr/local/bin/perl".to_owned()],
                ..Executable::default()
            },
        );
    });
}

fn build(args: Vec<OsString>) -> Command {
    ensure_registered();
    common::command(NAME, args)
}

/// Builds `perl` with `args` and no script.
pub fn new<'a>(args: impl IntoIterator<Item = &'a str>) -> Command {
    build(args.into_iter().map(OsString::from).collect())
}

/// Builds `perl <path> <args>`.
pub fn file<'a>(path: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    let mut all: Vec<OsString> = vec![path.into()];
    all.extend(args.into_iter().map(OsString::from));
    build(all)
}

/// Builds `perl -e <script> -- <args>`.
///
/// `--` is always passed, so the caller's arguments land in `@ARGV` and a leading dash is not read
/// as a perl flag.
pub fn inline<'a>(script: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    let mut all: Vec<OsString> = vec!["-e".into(), script.into(), "--".into()];
    all.extend(args.into_iter().map(OsString::from));
    build(all)
}

/// Runs `text` as a file when it is one line ending in `.pl`, and inline otherwise.
pub fn script<'a>(text: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    match common::script_file(text, &EXTENSIONS) {
        Some(path) => file(path, args),
        None => inline(text, args),
    }
}
