use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::error::Error;

/// Tokens that sit in front of the command.
#[derive(Debug)]
pub(crate) struct Invocation {
    pub help: bool,
    pub version: bool,
    pub edit: Option<PathBuf>,
    pub cwd: PathBuf,
    pub unset: Vec<String>,
    pub inline: Vec<(String, OsString)>,
    pub dotenv: Vec<PathBuf>,
    pub trust: Option<bool>,
    pub detach: bool,
    pub sudo: bool,
    /// Set by `--`. The remainder is a command and is not opened.
    pub force_run: bool,
    pub command: Vec<OsString>,
}

/// Walk argv after the program name.
///
/// Relative dotenv, edit, and `--env-file` paths resolve against the working
/// directory as `-C` updates it. The command itself is left for dispatch.
pub(crate) fn parse(args: &[OsString], cwd: PathBuf) -> Result<Invocation, Error> {
    let mut cwd = cwd;
    let mut inv = Invocation {
        help: false,
        version: false,
        edit: None,
        cwd: cwd.clone(),
        unset: Vec::new(),
        inline: Vec::new(),
        dotenv: Vec::new(),
        trust: None,
        detach: false,
        sudo: false,
        force_run: false,
        command: Vec::new(),
    };

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        let text = arg.to_str();

        if text == Some("--") {
            inv.force_run = true;
            inv.command.extend(args[i + 1..].iter().cloned());
            break;
        }
        if text == Some("-h") || text == Some("--help") {
            inv.help = true;
            i += 1;
            continue;
        }
        if text == Some("--version") {
            inv.version = true;
            i += 1;
            continue;
        }
        if text == Some("-e") || text == Some("--edit") {
            let path = next_value(args, &mut i, "-e")?;
            inv.edit = Some(resolve_path(&cwd, path));
            continue;
        }
        if text == Some("--env-file") {
            let path = next_value(args, &mut i, "--env-file")?;
            inv.dotenv.push(resolve_path(&cwd, path));
            continue;
        }
        if text == Some("-C") || text == Some("--chdir") {
            let dir = next_value(args, &mut i, "-C")?;
            cwd = resolve_path(&cwd, dir);
            continue;
        }
        if text == Some("-u") || text == Some("--unset") {
            let name = next_value(args, &mut i, "-u")?;
            let Some(name) = name.to_str() else {
                return Err(Error::usage("-u needs a variable name"));
            };
            inv.unset.push(name.to_string());
            continue;
        }
        if text == Some("--trust") {
            inv.trust = Some(true);
            i += 1;
            continue;
        }
        if text == Some("--no-trust") {
            inv.trust = Some(false);
            i += 1;
            continue;
        }
        if text == Some("--detach") {
            inv.detach = true;
            i += 1;
            continue;
        }
        if text == Some("--sudo") {
            inv.sudo = true;
            i += 1;
            continue;
        }
        if let Some(text) = text {
            if text.starts_with('-') && text != "-" {
                return Err(Error::usage(format!("unknown flag `{text}`")));
            }
            if let Some((key, value)) = split_assignment(text) {
                inv.inline.push((key, value));
                i += 1;
                continue;
            }
        }
        if is_dotenv_file(arg, &cwd) {
            inv.dotenv.push(resolve_path(&cwd, arg));
            i += 1;
            continue;
        }

        inv.command.extend(args[i..].iter().cloned());
        break;
    }

    inv.cwd = cwd;
    if inv.edit.is_some() && !inv.command.is_empty() {
        return Err(Error::usage("-e does not take a command"));
    }
    Ok(inv)
}

fn next_value<'a>(args: &'a [OsString], i: &mut usize, flag: &str) -> Result<&'a OsStr, Error> {
    *i += 1;
    args.get(*i)
        .map(|value| {
            *i += 1;
            value.as_os_str()
        })
        .ok_or_else(|| Error::usage(format!("{flag} needs a value")))
}

pub(crate) fn resolve_path(cwd: &Path, token: &OsStr) -> PathBuf {
    let path = PathBuf::from(token);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn split_assignment(token: &str) -> Option<(String, OsString)> {
    let (name, value) = token.split_once('=')?;
    if !is_env_name(name) {
        return None;
    }
    Some((name.to_string(), OsString::from(value)))
}

pub(crate) fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_dotenv_file(token: &OsStr, cwd: &Path) -> bool {
    let path = resolve_path(cwd, token);
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    (name == ".env" || name.ends_with(".env")) && path.is_file()
}

pub(crate) fn is_unqualified(token: &OsStr) -> bool {
    let Some(text) = token.to_str() else {
        return false;
    };
    if text.is_empty() || text == "." || text == ".." {
        return false;
    }
    !text.contains('/') && !text.contains('\\') && !Path::new(text).is_absolute()
}

pub(crate) fn is_url(token: &OsStr) -> bool {
    let Some(text) = token.to_str() else {
        return false;
    };
    let lower = text.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Final suffix including the dot, lowercased. Dotfiles with no further dot are `None`.
pub(crate) fn extension_suffix(token: &OsStr) -> Option<String> {
    let text = token.to_str()?;
    let name = Path::new(text).file_name()?.to_str()?;
    let idx = name.rfind('.')?;
    if idx == 0 {
        return None;
    }
    Some(name[idx..].to_ascii_lowercase())
}
