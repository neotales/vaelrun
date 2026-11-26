//! Executable lookup along `PATH`, with the executable bit checked on Unix.

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

/// Options that control an executable lookup.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WhichOptions {
    /// Remember a resolved path for the next lookup of the same command.
    ///
    /// A miss is not remembered, so installing a tool makes it visible without a restart. The
    /// cache is not used when [`WhichOptions::prepend_paths`] is set, because the result depends
    /// on those directories.
    pub use_cache: bool,
    /// Directories searched before `PATH`, in order.
    pub prepend_paths: Vec<PathBuf>,
}

/// Finds `command` on `PATH` with default options.
///
/// An absolute path, or one containing a separator, is checked directly.
#[must_use]
pub fn which(command: &str) -> Option<PathBuf> {
    which_first(command, &WhichOptions::default())
}

/// Finds `command` using `options`.
///
/// On Unix the name must match exactly and the file must have an executable bit set. On Windows
/// each directory is tried with `PATHEXT` (default `.COM;.EXE;.BAT;.CMD` when it is unset).
///
/// An absolute path, or one containing a separator, is checked directly and returned as given.
#[must_use]
pub fn which_first(command: &str, options: &WhichOptions) -> Option<PathBuf> {
    if command.is_empty() {
        return None;
    }

    // `prepend_paths` changes the answer for the same command, so a lookup that uses them is
    // never served from, nor stored in, the name-only cache.
    let cacheable = options.use_cache && options.prepend_paths.is_empty();

    if cacheable && let Some(found) = cache_get(command) {
        return Some(found);
    }

    let found = resolve(command, options);

    if cacheable && let Some(path) = &found {
        cache_put(command, path.clone());
    }

    found
}

fn resolve(command: &str, options: &WhichOptions) -> Option<PathBuf> {
    let path = Path::new(command);
    if path.is_absolute() || has_separator(command) {
        return executable_file(path).then(|| path.to_path_buf());
    }

    for directory in search_paths(options) {
        if let Some(found) = find_in_dir(&directory, command) {
            return Some(found);
        }
    }

    None
}

fn search_paths(options: &WhichOptions) -> Vec<PathBuf> {
    let mut directories = options.prepend_paths.clone();
    if let Some(value) = env::var_os("PATH") {
        directories.extend(env::split_paths(&value).filter(|path| !path.as_os_str().is_empty()));
    }
    directories
}

#[cfg(windows)]
fn has_separator(command: &str) -> bool {
    command.contains('/') || command.contains('\\')
}

#[cfg(not(windows))]
fn has_separator(command: &str) -> bool {
    command.contains('/')
}

#[cfg(unix)]
fn find_in_dir(directory: &Path, command: &str) -> Option<PathBuf> {
    let candidate = directory.join(command);
    executable_file(&candidate).then_some(candidate)
}

#[cfg(windows)]
fn find_in_dir(directory: &Path, command: &str) -> Option<PathBuf> {
    let candidate = directory.join(command);
    if candidate.is_file() {
        return Some(candidate);
    }

    // A name that already carries an extension is only tried as written.
    if Path::new(command).extension().is_some() {
        return None;
    }

    for extension in windows_extensions() {
        let candidate = directory.join(format!("{command}{extension}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

#[cfg(windows)]
fn windows_extensions() -> Vec<String> {
    let value = env::var("PATHEXT").unwrap_or_default();
    let value = if value.trim().is_empty() {
        ".COM;.EXE;.BAT;.CMD".to_owned()
    } else {
        value
    };

    value
        .split(';')
        .filter(|extension| !extension.is_empty())
        .map(|extension| {
            if extension.starts_with('.') {
                extension.to_owned()
            } else {
                format!(".{extension}")
            }
        })
        .collect()
}

#[cfg(unix)]
fn executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable_file(path: &Path) -> bool {
    path.is_file()
}

fn cache() -> &'static Mutex<HashMap<String, PathBuf>> {
    static CACHE: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_get(command: &str) -> Option<PathBuf> {
    cache()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(command)
        .cloned()
}

fn cache_put(command: &str, path: PathBuf) {
    cache()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(command.to_owned(), path);
}
