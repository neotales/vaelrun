//! .NET commands.

use std::ffi::OsString;
use std::path::Path;
use std::sync::Once;

use neotales_exec::{Command, Executable, register};

use crate::common;

const NAME: &str = "dotnet";
const EXTENSIONS: [&str; 7] = [
    ".cs", ".csx", ".dll", ".exe", ".csproj", ".fsproj", ".vbproj",
];

#[cfg(windows)]
const VARIABLE: &str = "RUN_WIN_DOTNET_EXE";
#[cfg(not(windows))]
const VARIABLE: &str = "RUN_DOTNET_EXE";

static REGISTER: Once = Once::new();

fn ensure_registered() {
    REGISTER.call_once(|| {
        register(
            NAME,
            Executable {
                name: NAME.to_owned(),
                variable: VARIABLE.to_owned(),
                windows: vec![
                    "${HOME}\\.dotnet\\dotnet.exe".to_owned(),
                    "${LOCALAPPDATA}\\dotnet\\dotnet.exe".to_owned(),
                    "${ProgramFiles}\\dotnet\\dotnet.exe".to_owned(),
                    "%ProgramFiles(x86)%\\dotnet\\dotnet.exe".to_owned(),
                ],
                linux: vec![
                    "$HOME/.dotnet/dotnet".to_owned(),
                    "$HOME/.local/share/dotnet/dotnet".to_owned(),
                    "/usr/local/bin/dotnet".to_owned(),
                    "/usr/bin/dotnet".to_owned(),
                    "/bin/dotnet".to_owned(),
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

/// Builds `dotnet` with `args` and no script.
pub fn new<'a>(args: impl IntoIterator<Item = &'a str>) -> Command {
    build(args.into_iter().map(OsString::from).collect())
}

/// Builds the command for one file.
///
/// `.cs` and `.csx` become `dotnet run --file <path> -- <args>`, `.dll` and `.exe` become
/// `dotnet <path> <args>`, and everything else becomes `dotnet run --project <path> -- <args>`.
/// The `--` separator is left out when there are no program arguments.
pub fn file<'a>(path: &str, args: impl IntoIterator<Item = &'a str>) -> Command {
    let extension = Path::new(path)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default();
    let args: Vec<&str> = args.into_iter().collect();

    match extension {
        "cs" | "csx" => with_separator(&["run", "--file", path], &args),
        "dll" | "exe" => {
            let mut all: Vec<OsString> = vec![path.into()];
            all.extend(args.into_iter().map(OsString::from));
            build(all)
        }
        _ => with_separator(&["run", "--project", path], &args),
    }
}

fn with_separator(prefix: &[&str], args: &[&str]) -> Command {
    let mut all: Vec<OsString> = prefix.iter().copied().map(OsString::from).collect();
    if !args.is_empty() {
        all.push("--".into());
        all.extend(args.iter().copied().map(OsString::from));
    }
    build(all)
}

/// Writes `script` to a fresh private `.cs` file in the user's directory and
/// builds the `.cs` file form.
///
/// On Windows privacy relies on the per-user `LocalAppData` directory and reparse-point refusal,
/// not a custom ACL.
///
/// The file is left in place, and the path is recorded on the command for [`Command::temp_file`].
///
/// # Errors
///
/// Returns the I/O error when the private directory or the file cannot be created or written.
pub fn inline<'a>(
    script: &str,
    args: impl IntoIterator<Item = &'a str>,
) -> std::io::Result<Command> {
    let path = common::temp_file(script, "cs")?;
    let argument = path.to_string_lossy().into_owned();
    Ok(file(&argument, args).with_temp_file(path))
}

/// Runs `text` as a file when it is one line ending in a known .NET extension, and inline
/// otherwise.
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
