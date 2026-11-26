use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::error::Error;

/// Where an unqualified name was found. Trust allows hints and `[bin]`, not a PATH-only hit.
#[derive(Debug)]
pub(crate) enum Hit {
    Hint(PathBuf),
    Bin(PathBuf),
    Path(PathBuf),
}

impl Hit {
    pub(crate) fn into_path(self) -> PathBuf {
        match self {
            Self::Hint(path) | Self::Bin(path) | Self::Path(path) => path,
        }
    }

    pub(crate) fn allowed(&self, trust_on: bool) -> bool {
        match self {
            Self::Hint(_) | Self::Bin(_) => true,
            Self::Path(_) => !trust_on,
        }
    }
}

/// Built-in hints, then `[bin]`, then `PATH`.
pub(crate) struct Finder {
    bin_dirs: Vec<PathBuf>,
    path_dirs: Vec<PathBuf>,
    pathext: String,
    home: Option<PathBuf>,
    dotnet_root: Option<PathBuf>,
    goroot: Option<PathBuf>,
    program_files: Option<PathBuf>,
}

impl Finder {
    pub(crate) fn new(
        bin_dirs: Vec<PathBuf>,
        path_var: Option<&OsStr>,
        pathext: Option<&OsStr>,
        home: Option<PathBuf>,
        dotnet_root: Option<PathBuf>,
        goroot: Option<PathBuf>,
        program_files: Option<PathBuf>,
    ) -> Self {
        let path_dirs = path_var.map(split_search_path).unwrap_or_default();
        let pathext = pathext
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or(".COM;.EXE;.BAT;.CMD;.PS1")
            .to_string();
        Self {
            bin_dirs,
            path_dirs,
            pathext,
            home,
            dotnet_root,
            goroot,
            program_files,
        }
    }

    pub(crate) fn lookup(&self, name: &str) -> Option<Hit> {
        for path in hint_candidates(name, &self.hint_input()) {
            if path.is_file() {
                return Some(Hit::Hint(path));
            }
        }
        for dir in &self.bin_dirs {
            if let Some(path) = find_in_dir(dir, name, &self.pathext) {
                return Some(Hit::Bin(path));
            }
        }
        for dir in &self.path_dirs {
            if let Some(path) = find_in_dir(dir, name, &self.pathext) {
                if !is_runnable(&path) {
                    continue;
                }
                return Some(Hit::Path(path));
            }
        }
        None
    }

    fn hint_input(&self) -> HintInput<'_> {
        HintInput {
            windows: cfg!(windows),
            home: self.home.as_deref(),
            dotnet_root: self.dotnet_root.as_deref(),
            goroot: self.goroot.as_deref(),
            program_files: self.program_files.as_deref(),
        }
    }
}

struct HintInput<'a> {
    windows: bool,
    home: Option<&'a Path>,
    dotnet_root: Option<&'a Path>,
    goroot: Option<&'a Path>,
    program_files: Option<&'a Path>,
}

fn hint_candidates(name: &str, input: &HintInput<'_>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    match name {
        "dotnet" => {
            if let Some(root) = input.dotnet_root {
                out.push(root.join(exe_name("dotnet", input.windows)));
            }
            if let Some(pf) = input.program_files {
                out.push(pf.join("dotnet").join(exe_name("dotnet", input.windows)));
            }
            out.push(PathBuf::from("/usr/share/dotnet/dotnet"));
            out.push(PathBuf::from("/usr/local/share/dotnet/dotnet"));
        }
        "go" => {
            if let Some(root) = input.goroot {
                out.push(root.join("bin").join(exe_name("go", input.windows)));
            }
            out.push(PathBuf::from("/usr/local/go/bin/go"));
            out.push(PathBuf::from("/usr/lib/go/bin/go"));
        }
        "pwsh" => {
            out.push(PathBuf::from("/usr/bin/pwsh"));
            out.push(PathBuf::from("/usr/local/bin/pwsh"));
            if let Some(pf) = input.program_files {
                out.push(pf.join("PowerShell").join("7").join("pwsh.exe"));
                out.push(pf.join("PowerShell").join("7-preview").join("pwsh.exe"));
            }
        }
        "bun" => {
            if let Some(home) = input.home {
                out.push(
                    home.join(".bun")
                        .join("bin")
                        .join(exe_name("bun", input.windows)),
                );
            }
            out.push(PathBuf::from("/usr/local/bin/bun"));
        }
        "ghostty" => {
            out.push(PathBuf::from("/usr/bin/ghostty"));
            out.push(PathBuf::from("/usr/local/bin/ghostty"));
        }
        _ => {}
    }
    out
}

fn exe_name(name: &str, windows: bool) -> String {
    if windows {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn find_in_dir(dir: &Path, name: &str, pathext: &str) -> Option<PathBuf> {
    for candidate in name_candidates(name, pathext) {
        let path = dir.join(candidate);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

#[cfg(windows)]
fn name_candidates(name: &str, pathext: &str) -> Vec<String> {
    pathext_candidates(name, Some(pathext))
}

#[cfg(not(windows))]
fn name_candidates(name: &str, _pathext: &str) -> Vec<String> {
    vec![name.to_string()]
}

/// Bare name, then each `PATHEXT` suffix. Default is `.COM;.EXE;.BAT;.CMD;.PS1`.
#[cfg(any(windows, test))]
pub(crate) fn pathext_candidates(name: &str, pathext: Option<&str>) -> Vec<String> {
    let ext = pathext.unwrap_or(".COM;.EXE;.BAT;.CMD;.PS1");
    let mut out = vec![name.to_string()];
    for item in ext.split(';') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let suffix = if item.starts_with('.') {
            item.to_string()
        } else {
            format!(".{item}")
        };
        if !name
            .to_ascii_lowercase()
            .ends_with(&suffix.to_ascii_lowercase())
        {
            out.push(format!("{name}{suffix}"));
        }
    }
    out
}

fn is_runnable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        is_executable(path)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub(crate) fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn split_search_path(value: &OsStr) -> Vec<PathBuf> {
    let text = value.to_string_lossy();
    let sep = if cfg!(windows) { ';' } else { ':' };
    text.split(sep)
        .filter(|part| !part.is_empty())
        .map(PathBuf::from)
        .collect()
}

pub(crate) fn resolve_program(
    token: &OsStr,
    finder: &Finder,
    cwd: &Path,
    trust_on: bool,
) -> Result<PathBuf, Error> {
    let display = token.to_string_lossy();
    let path = Path::new(token);
    if path.is_absolute() || !crate::cli::is_unqualified(token) {
        let full = if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        };
        if full.is_file() {
            return Ok(full);
        }
        if full.exists() {
            return Err(Error::cannot_execute(&full.display().to_string()));
        }
        return Err(Error::not_found(&display));
    }
    match finder.lookup(&display) {
        None => Err(Error::not_found(&display)),
        Some(hit) if !hit.allowed(trust_on) => Err(Error::trust(&display)),
        Some(hit) => Ok(hit.into_path()),
    }
}

/// Editor and the platform opener are requested explicitly, so a PATH hit is allowed.
pub(crate) fn resolve_lenient(name: &str, finder: &Finder, cwd: &Path) -> Result<PathBuf, Error> {
    if !crate::cli::is_unqualified(OsStr::new(name)) {
        return resolve_program(OsStr::new(name), finder, cwd, false);
    }
    match finder.lookup(name) {
        Some(hit) => Ok(hit.into_path()),
        None => Err(Error::not_found(name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pathext_joins_default_and_custom_lists() {
        let names = pathext_candidates("go", None);
        assert_eq!(
            names,
            vec![
                "go".to_string(),
                "go.COM".to_string(),
                "go.EXE".to_string(),
                "go.BAT".to_string(),
                "go.CMD".to_string(),
                "go.PS1".to_string(),
            ]
        );
        let custom = pathext_candidates("tool", Some(".EXE;.COM"));
        assert_eq!(
            custom,
            vec![
                "tool".to_string(),
                "tool.EXE".to_string(),
                "tool.COM".to_string()
            ]
        );
    }

    #[test]
    fn windows_hints_use_program_files_and_exe() {
        let input = HintInput {
            windows: true,
            home: Some(Path::new(r"C:\Users\me")),
            dotnet_root: Some(Path::new(r"C:\dotnet")),
            goroot: Some(Path::new(r"C:\go")),
            program_files: Some(Path::new(r"C:\Program Files")),
        };
        let dotnet = hint_candidates("dotnet", &input);
        assert!(
            dotnet
                .iter()
                .any(|p| p == &Path::new(r"C:\dotnet").join("dotnet.exe"))
        );
        assert!(dotnet.iter().any(|p| {
            p == &Path::new(r"C:\Program Files")
                .join("dotnet")
                .join("dotnet.exe")
        }));
        let pwsh = hint_candidates("pwsh", &input);
        assert!(pwsh.iter().any(|p| p.ends_with("pwsh.exe")));
        let bun = hint_candidates("bun", &input);
        assert!(bun.iter().any(|p| {
            p == &Path::new(r"C:\Users\me")
                .join(".bun")
                .join("bin")
                .join("bun.exe")
        }));
    }
}
