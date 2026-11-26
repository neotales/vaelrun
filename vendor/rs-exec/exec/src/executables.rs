//! A registry of program names and the places their executables live.

use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, PoisonError};

use crate::error::Error;
use crate::finder::{WhichOptions, which_first};

/// A program name and every place its executable may be found.
///
/// The registry is consulted by [`find`]. A runtime crate registers its tool once and callers look
/// it up by name; [`Executable::variable`] names the environment variable that overrides the
/// search, and the three candidate lists are per platform.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Executable {
    /// The name the executable is registered under.
    pub name: String,
    /// An explicit path checked before the platform lists.
    pub path: Option<PathBuf>,
    /// The environment variable that holds an explicit path, checked first.
    pub variable: String,
    /// Candidates tried on Windows.
    pub windows: Vec<String>,
    /// Candidates tried on Unix other than macOS, and after [`Executable::darwin`] on macOS.
    pub linux: Vec<String>,
    /// Candidates tried first on macOS.
    pub darwin: Vec<String>,
}

/// Registers `executable` under `name`, replacing an earlier registration.
///
/// When [`Executable::variable`] is empty it is derived from `name` in screaming snake case. Every
/// runtime crate calls this on first use, so registering the same definition twice is harmless.
pub fn register(name: &str, mut executable: Executable) {
    if executable.variable.is_empty() {
        executable.variable = derived_variable(name);
    }
    registry()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(name.to_owned(), executable);
}

/// Finds the executable registered as `name`.
///
/// The search order matches the reference implementation: the environment variable named by
/// [`Executable::variable`] (expanded), then [`Executable::path`], then the platform candidate
/// lists. macOS tries [`Executable::darwin`] and then [`Executable::linux`]. A candidate that
/// expands to an empty string is skipped, and `$VAR`, `${VAR}` and `%VAR%` are expanded from the
/// process environment.
///
/// # Errors
///
/// Returns [`Error::NotFound`] when no candidate resolves to an executable.
pub fn find(name: &str, options: Option<&WhichOptions>) -> Result<PathBuf, Error> {
    let default_options = WhichOptions::default();
    let options = options.unwrap_or(&default_options);
    let executable = lookup(name);

    if !executable.variable.is_empty()
        && let Some(value) = env::var_os(&executable.variable)
    {
        let expanded = expand(&value.to_string_lossy());
        if !expanded.is_empty()
            && let Some(found) = which_first(&expanded, options)
        {
            return Ok(found);
        }
    }

    if let Some(path) = &executable.path {
        let expanded = expand(&path.to_string_lossy());
        if !expanded.is_empty()
            && let Some(found) = which_first(&expanded, options)
        {
            return Ok(found);
        }
    }

    for candidate in candidates(&executable) {
        let expanded = expand(candidate);
        if expanded.is_empty() {
            continue;
        }
        if let Some(found) = which_first(&expanded, options) {
            return Ok(found);
        }
    }

    Err(Error::NotFound {
        name: name.to_owned(),
    })
}

fn candidates(executable: &Executable) -> Vec<&String> {
    if cfg!(windows) {
        executable.windows.iter().collect()
    } else if cfg!(target_os = "macos") {
        executable
            .darwin
            .iter()
            .chain(executable.linux.iter())
            .collect()
    } else {
        executable.linux.iter().collect()
    }
}

fn lookup(name: &str) -> Executable {
    let mut executable = registry()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(name)
        .cloned()
        .unwrap_or_default();

    if executable.name.is_empty() {
        name.clone_into(&mut executable.name);
    }
    if executable.variable.is_empty() {
        executable.variable = derived_variable(name);
    }

    executable
}

fn derived_variable(name: &str) -> String {
    let mut variable = String::with_capacity(name.len());
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            variable.push(character.to_ascii_uppercase());
        } else if !variable.is_empty() && !variable.ends_with('_') {
            variable.push('_');
        }
    }
    variable
}

/// Expands `$VAR`, `${VAR}` and `%VAR%` from the process environment.
///
/// A name inside `${...}` may contain characters a `$VAR` name cannot, such as the parentheses in
/// `${ProgramFiles(x86)}`. An unset variable expands to the empty string.
fn expand(input: &str) -> String {
    let characters: Vec<char> = input.chars().collect();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;

    while index < characters.len() {
        let character = characters[index];
        if character == '$' {
            if characters.get(index + 1) == Some(&'{')
                && let Some(offset) = characters[index + 2..].iter().position(|c| *c == '}')
            {
                let name: String = characters[index + 2..index + 2 + offset].iter().collect();
                output.push_str(&environment(&name));
                index += offset + 3;
                continue;
            }

            let start = index + 1;
            let mut end = start;
            while end < characters.len()
                && (characters[end].is_ascii_alphanumeric() || characters[end] == '_')
            {
                end += 1;
            }
            if end > start {
                let name: String = characters[start..end].iter().collect();
                output.push_str(&environment(&name));
                index = end;
                continue;
            }

            output.push('$');
            index += 1;
            continue;
        }

        if character == '%'
            && let Some(offset) = characters[index + 1..].iter().position(|c| *c == '%')
        {
            let name: String = characters[index + 1..index + 1 + offset].iter().collect();
            if !name.is_empty() {
                output.push_str(&environment(&name));
                index += offset + 2;
                continue;
            }

            output.push('%');
            index += 1;
            continue;
        }

        output.push(character);
        index += 1;
    }

    output
}

fn environment(name: &str) -> String {
    env::var_os(name).map_or_default(|value| value.to_string_lossy().into_owned())
}

fn registry() -> &'static Mutex<HashMap<String, Executable>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Executable>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}
