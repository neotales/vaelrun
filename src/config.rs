use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::Error;

/// Which host the config search path is built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostOs {
    Linux,
    Macos,
    Windows,
}

/// Loaded INI. `[bin]` and `[detach]` append; the other sections replace by key.
#[derive(Clone, Debug, Default)]
pub(crate) struct Config {
    pub bin_dirs: Vec<String>,
    pub detach: Vec<String>,
    /// Lowercased suffix including the dot, mapped to the raw template.
    pub extensions: BTreeMap<String, String>,
    pub aliases: BTreeMap<String, Vec<String>>,
    /// `None` when no file set `[trust]`.
    pub trust: Option<bool>,
}

impl Config {
    pub(crate) fn merge(&mut self, other: Config) {
        for dir in other.bin_dirs {
            if !self.bin_dirs.iter().any(|have| have == &dir) {
                self.bin_dirs.push(dir);
            }
        }
        for name in other.detach {
            if !self
                .detach
                .iter()
                .any(|have| have.eq_ignore_ascii_case(&name))
            {
                self.detach.push(name);
            }
        }
        for (key, value) in other.extensions {
            self.extensions.insert(key, value);
        }
        for (key, value) in other.aliases {
            self.aliases.insert(key, value);
        }
        if other.trust.is_some() {
            self.trust = other.trust;
        }
    }

    pub(crate) fn trust_on(&self) -> bool {
        self.trust.unwrap_or(true)
    }
}

/// Search order. Later files override the same extension, alias, or trust key.
pub(crate) fn config_file_paths(
    host: HostOs,
    home: Option<&Path>,
    userprofile: Option<&Path>,
    vaelrun_config: Option<&Path>,
) -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from("/etc/vaelrun/config.ini")];
    if host == HostOs::Windows {
        paths.push(PathBuf::from(r"C:\ProgramData\vaelrun\etc\config.ini"));
    }
    let user_home = if let Some(home) = home {
        Some(home.to_path_buf())
    } else if host == HostOs::Windows {
        userprofile.map(Path::to_path_buf)
    } else {
        None
    };
    if let Some(home) = user_home {
        paths.push(home.join(".config").join("vaelrun").join("config.ini"));
    }
    if let Some(path) = vaelrun_config {
        paths.push(path.to_path_buf());
    }
    paths
}

pub(crate) fn load_config(paths: &[PathBuf]) -> Result<Config, Error> {
    let mut cfg = Config::default();
    for path in paths {
        match std::fs::read_to_string(path) {
            Ok(text) => cfg.merge(parse_config(&text, path)?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(Error::io(path, &err)),
        }
    }
    Ok(cfg)
}

pub(crate) fn parse_config(text: &str, path: &Path) -> Result<Config, Error> {
    let mut cfg = Config::default();
    let mut section = String::new();
    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            if !line.ends_with(']') || line.len() < 3 {
                return Err(Error::at(path, line_no, "invalid section"));
            }
            section = line[1..line.len() - 1].trim().to_ascii_lowercase();
            if section.is_empty() {
                return Err(Error::at(path, line_no, "invalid section"));
            }
            continue;
        }
        match section.as_str() {
            "" => return Err(Error::at(path, line_no, "key outside section")),
            "bin" | "detach" => {
                if line.contains('=') {
                    return Err(Error::at(path, line_no, "expected a bare value"));
                }
                if section == "bin" {
                    cfg.bin_dirs.push(line.to_string());
                } else {
                    cfg.detach.push(line.to_string());
                }
            }
            "extensions" | "alias" | "trust" => {
                let Some((key, value)) = split_kv(line) else {
                    return Err(Error::at(path, line_no, "expected key=value"));
                };
                if key.is_empty() {
                    return Err(Error::at(path, line_no, "expected key=value"));
                }
                match section.as_str() {
                    "extensions" => {
                        if value.is_empty() {
                            return Err(Error::at(path, line_no, "empty extension template"));
                        }
                        cfg.extensions
                            .insert(normalize_suffix(key), value.to_string());
                    }
                    "alias" => {
                        let argv = crate::template::split_quoted(value)
                            .map_err(|_| Error::at(path, line_no, "invalid alias"))?;
                        if argv.is_empty() {
                            return Err(Error::at(path, line_no, "invalid alias"));
                        }
                        cfg.aliases.insert(key.to_string(), argv);
                    }
                    "trust" => {
                        if key != "mode" {
                            return Err(Error::at(path, line_no, "unknown trust key"));
                        }
                        cfg.trust = Some(parse_trust_mode(value).ok_or_else(|| {
                            Error::at(path, line_no, "trust mode must be on or off")
                        })?);
                    }
                    _ => unreachable!(),
                }
            }
            _ => return Err(Error::at(path, line_no, "unknown section")),
        }
    }
    Ok(cfg)
}

fn normalize_suffix(key: &str) -> String {
    let lower = key.to_ascii_lowercase();
    if lower.starts_with('.') {
        lower
    } else {
        format!(".{lower}")
    }
}

fn parse_trust_mode(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

fn split_kv(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once('=')?;
    Some((key.trim(), value.trim()))
}

fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == b'"' || c == b'\'' {
            quote = Some(c);
            i += 1;
            continue;
        }
        if (c == b'#' || c == b';') && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            return line[..i].trim_end();
        }
        i += 1;
    }
    line
}

pub(crate) fn expand_bin_dir(raw: &str, home: Option<&Path>, cwd: &Path) -> PathBuf {
    let expanded = if raw == "~" {
        home.map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(raw))
    } else if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = home {
            home.join(rest)
        } else {
            PathBuf::from(raw)
        }
    } else {
        PathBuf::from(raw)
    };
    if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn search_paths_follow_host_and_home() {
        let home = Path::new("/home/me");
        let linux = config_file_paths(
            HostOs::Linux,
            Some(home),
            Some(Path::new("C:\\Users\\me")),
            None,
        );
        assert_eq!(
            linux,
            vec![
                PathBuf::from("/etc/vaelrun/config.ini"),
                PathBuf::from("/home/me/.config/vaelrun/config.ini"),
            ]
        );

        let windows = config_file_paths(
            HostOs::Windows,
            None,
            Some(Path::new(r"C:\Users\me")),
            Some(Path::new(r"D:\custom.ini")),
        );
        let user = Path::new(r"C:\Users\me")
            .join(".config")
            .join("vaelrun")
            .join("config.ini");
        assert_eq!(
            windows,
            vec![
                PathBuf::from("/etc/vaelrun/config.ini"),
                PathBuf::from(r"C:\ProgramData\vaelrun\etc\config.ini"),
                user,
                PathBuf::from(r"D:\custom.ini"),
            ]
        );

        let home_wins = config_file_paths(
            HostOs::Windows,
            Some(Path::new(r"D:\home")),
            Some(Path::new(r"C:\Users\me")),
            None,
        );
        assert!(home_wins.iter().any(|p| p.starts_with(r"D:\home")));
        assert!(!home_wins.iter().any(|p| p.starts_with(r"C:\Users")));
    }

    #[test]
    fn later_file_overrides_extension_and_appends_bin() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("a.ini");
        let second = dir.path().join("b.ini");
        fs::write(
            &first,
            "[extensions]\n.ts = bun $s\n[bin]\n/opt/a\n[trust]\nmode = off\n",
        )
        .unwrap();
        fs::write(
            &second,
            "[extensions]\n.ts = node $s\n[bin]\n/opt/a\n/opt/b\n[alias]\ncmd = /bin/true\n",
        )
        .unwrap();
        let missing = dir.path().join("missing.ini");
        let cfg = load_config(&[first, missing, second]).unwrap();
        assert_eq!(
            cfg.extensions.get(".ts").map(String::as_str),
            Some("node $s")
        );
        assert_eq!(
            cfg.bin_dirs,
            vec!["/opt/a".to_string(), "/opt/b".to_string()]
        );
        assert_eq!(cfg.trust, Some(false));
        assert!(cfg.aliases.contains_key("cmd"));
    }
}
