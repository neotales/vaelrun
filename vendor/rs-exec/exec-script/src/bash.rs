//! Bash commands.

use std::ffi::OsString;
use std::sync::Once;

use neotales_exec::{Command, Executable, register};

use crate::common;

const NAME: &str = "bash";
const SCRIPT_ARGS: [&str; 4] = ["--noprofile", "--norc", "-eo", "pipefail"];

#[cfg(windows)]
const VARIABLE: &str = "RUN_WIN_BASH_EXE";
#[cfg(not(windows))]
const VARIABLE: &str = "RUN_BASH_EXE";

static REGISTER: Once = Once::new();

fn ensure_registered() {
    REGISTER.call_once(|| {
        register(
            NAME,
            Executable {
                name: NAME.to_owned(),
                variable: VARIABLE.to_owned(),
                windows: vec![
                    "${ProgramFiles}\\Git\\bin\\bash.exe".to_owned(),
                    "%ProgramFiles(x86)%\\Git\\bin\\bash.exe".to_owned(),
                    "${SystemRoot}\\System32\\bash.exe".to_owned(),
                ],
                linux: vec!["/bin/bash".to_owned(), "/usr/bin/bash".to_owned()],
                ..Executable::default()
            },
        );
    });
}

fn build(args: Vec<OsString>) -> Command {
    ensure_registered();
    common::command(NAME, args)
}

/// Builds `bash` with `args` and no script.
pub fn new<'a>(args: impl IntoIterator<Item = &'a str>) -> Command {
    build(args.into_iter().map(OsString::from).collect())
}

/// Builds `bash --noprofile --norc -eo pipefail <path> <args>`.
pub fn file<'a>(path: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    file_with_shell_args(path, SCRIPT_ARGS, args)
}

/// Builds `bash <shell_args> <path> <args>`.
///
/// `shell_args` replaces the default `--noprofile --norc -eo pipefail` prefix, and an empty list
/// means no prefix; [`file()`] is the default shape.
pub fn file_with_shell_args<'a, S, A>(path: &str, shell_args: S, args: A) -> Command
where
    S: IntoIterator<Item = &'a str>,
    A: IntoIterator<Item = &'a str>,
{
    let path = resolve_script_file(path);
    let mut all: Vec<OsString> = shell_args.into_iter().map(OsString::from).collect();
    all.push(path.into());
    all.extend(args.into_iter().map(OsString::from));
    build(all)
}

/// Builds `bash --noprofile --norc -eo pipefail -c <script> bash <args>`.
///
/// The caller's arguments follow a `$0` placeholder, so the first one reaches the script as `$1`.
pub fn inline<'a>(script: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    inline_with_shell_args(script, SCRIPT_ARGS, args)
}

/// Builds `bash <shell_args> -c <script> bash <args>`.
///
/// `shell_args` replaces the default `--noprofile --norc -eo pipefail` prefix, and an empty list
/// means no prefix; [`inline()`] is the default shape.
pub fn inline_with_shell_args<'a, S, A>(script: &str, shell_args: S, args: A) -> Command
where
    S: IntoIterator<Item = &'a str>,
    A: IntoIterator<Item = &'a str>,
{
    let mut all: Vec<OsString> = shell_args.into_iter().map(OsString::from).collect();
    all.push("-c".into());
    all.push(script.into());
    // `-c` makes the next word `$0`, so a placeholder keeps the first caller argument at `$1`.
    all.push(NAME.into());
    all.extend(args.into_iter().map(OsString::from));
    build(all)
}

/// Runs `text` as a file when it is one line ending in `.sh`, and inline otherwise.
pub fn script<'a>(text: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    match common::script_file(text, &[".sh"]) {
        Some(path) => file(path, args),
        None => inline(text, args),
    }
}

/// Rewrites an absolute path for a WSL bash, which sees drives under `/mnt`.
#[cfg(windows)]
fn resolve_script_file(path: &str) -> String {
    use std::path::Path;

    let absolute = if Path::new(path).is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().map_or_else(
            |_| path.to_owned(),
            |directory| directory.join(path).to_string_lossy().into_owned(),
        )
    };

    // A missing result means the fallback `bash`, which is not the WSL one.
    if !neotales_exec::find(NAME, None)
        .as_deref()
        .is_ok_and(contains_system32)
    {
        return absolute;
    }

    let rewritten = absolute.replace('\\', "/");
    let bytes = rewritten.as_bytes();
    if bytes.len() > 2 && bytes[1] == b':' {
        let drive = char::from(bytes[0].to_ascii_lowercase());
        return format!("/mnt/{drive}{}", &rewritten[2..]);
    }

    rewritten
}

/// ASCII-lowercases one UTF-16 unit, which is enough for the `system32` needle.
#[cfg(windows)]
fn lowercase_unit(unit: u16) -> u16 {
    const CASE_BIT: u16 = 0x20;

    if (u16::from(b'A')..=u16::from(b'Z')).contains(&unit) {
        unit | CASE_BIT
    } else {
        unit
    }
}

/// Whether the path contains `system32`, ASCII case-insensitively.
///
/// `str` has no `contains_ignore_ascii_case`, and lowercasing the whole path would allocate a
/// path-sized copy, so the needle is compared window by window instead.
#[cfg(any(windows, test))]
fn contains_system32(path: &std::path::Path) -> bool {
    const NEEDLE: [u8; 8] = *b"system32";

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;

        let mut window = [0_u16; NEEDLE.len()];
        let mut filled = 0_usize;

        for unit in path.as_os_str().encode_wide() {
            if filled == window.len() {
                window.copy_within(1.., 0);
                window[window.len() - 1] = unit;
            } else {
                window[filled] = unit;
                filled += 1;
            }

            if filled == window.len()
                && window
                    .iter()
                    .copied()
                    .zip(NEEDLE)
                    .all(|(unit, needle)| lowercase_unit(unit) == u16::from(needle))
            {
                return true;
            }
        }

        false
    }

    #[cfg(not(windows))]
    {
        path.as_os_str()
            .as_encoded_bytes()
            .windows(NEEDLE.len())
            .any(|window| {
                window
                    .iter()
                    .zip(NEEDLE)
                    .all(|(byte, needle)| byte.to_ascii_lowercase() == needle)
            })
    }
}

#[cfg(not(windows))]
fn resolve_script_file(path: &str) -> String {
    path.to_owned()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::contains_system32;

    #[test]
    fn system32_matches_ascii_case_insensitively() {
        assert!(contains_system32(Path::new(
            "C:\\Windows\\System32\\bash.exe"
        )));
        assert!(contains_system32(Path::new(
            "C:\\Windows\\SYSTEM32\\bash.exe"
        )));
        assert!(contains_system32(Path::new(
            "C:\\Windows\\system32\\bash.exe"
        )));
        assert!(!contains_system32(Path::new(
            "C:\\Windows\\system\\bash.exe"
        )));
        assert!(contains_system32(Path::new("system322")));
        assert!(!contains_system32(Path::new("sys")));
        assert!(!contains_system32(Path::new("")));
    }
}
