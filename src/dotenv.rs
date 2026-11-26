use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::Path;

use crate::error::Error;

/// Merged environment passed to a child.
#[derive(Clone, Debug)]
pub(crate) struct EnvMap {
    pub pairs: Vec<(OsString, OsString)>,
    pub utf8: HashMap<String, String>,
}

impl EnvMap {
    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.utf8.get(key).map(String::as_str)
    }

    fn rebuild_utf8(&mut self) {
        self.utf8.clear();
        for (key, value) in &self.pairs {
            if let (Some(key), Some(value)) = (key.to_str(), value.to_str()) {
                self.utf8.insert(key.to_string(), value.to_string());
            }
        }
    }
}

/// Parent, then dotenv, then `-u`, then inline assignments.
pub(crate) fn merge_env(
    parent: Vec<(OsString, OsString)>,
    dotenv: impl IntoIterator<Item = (String, String)>,
    unset: &[String],
    inline: &[(String, OsString)],
    ignore_case: bool,
) -> EnvMap {
    let mut pairs: Vec<(OsString, OsString)> = parent
        .into_iter()
        .filter(|(key, _)| key.to_str() != Some("VAELRUN_DETACHED"))
        .collect();
    for (key, value) in dotenv {
        upsert(
            &mut pairs,
            OsString::from(key),
            OsString::from(value),
            ignore_case,
        );
    }
    for name in unset {
        remove_key(&mut pairs, name, ignore_case);
    }
    for (key, value) in inline {
        upsert(&mut pairs, OsString::from(key), value.clone(), ignore_case);
    }
    let mut env = EnvMap {
        pairs,
        utf8: HashMap::new(),
    };
    env.rebuild_utf8();
    env
}

pub(crate) fn load_dotenv_files(
    paths: &[std::path::PathBuf],
) -> Result<Vec<(String, String)>, Error> {
    let mut out = Vec::new();
    for path in paths {
        let text = fs::read_to_string(path).map_err(|err| Error::io(path, &err))?;
        out.extend(parse_dotenv(&text, path)?);
    }
    Ok(out)
}

pub(crate) fn parse_dotenv(text: &str, path: &Path) -> Result<Vec<(String, String)>, Error> {
    let mut out = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = strip_export(line);
        let Some((key, raw_value)) = line.split_once('=') else {
            return Err(Error::at(path, line_no, "invalid dotenv line"));
        };
        let key = key.trim();
        if !crate::cli::is_env_name(key) {
            return Err(Error::at(path, line_no, "invalid dotenv line"));
        }
        let value = parse_value(raw_value.trim())
            .map_err(|_| Error::at(path, line_no, "invalid dotenv line"))?;
        out.push((key.to_string(), value));
    }
    Ok(out)
}

fn strip_export(line: &str) -> &str {
    match line.strip_prefix("export") {
        Some(rest) if rest.starts_with(char::is_whitespace) => rest.trim_start(),
        _ => line,
    }
}

fn parse_value(raw: &str) -> Result<String, ()> {
    let mut chars = raw.chars().peekable();
    let Some(first) = chars.peek().copied() else {
        return Ok(String::new());
    };
    if first != '"' && first != '\'' {
        return Ok(cut_unquoted_comment(raw).trim_end().to_string());
    }
    let quote = first;
    chars.next();
    let mut out = String::new();
    let mut closed = false;
    while let Some(c) = chars.next() {
        if quote == '"' && c == '\\' {
            match chars.next() {
                Some(next) => out.push(next),
                None => return Err(()),
            }
            continue;
        }
        if c == quote {
            closed = true;
            break;
        }
        out.push(c);
    }
    if !closed {
        return Err(());
    }
    let rest: String = chars.collect();
    let rest = rest.trim_start();
    if !rest.is_empty() && !rest.starts_with('#') {
        return Err(());
    }
    Ok(out)
}

fn cut_unquoted_comment(value: &str) -> &str {
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            return &value[..i];
        }
        i += 1;
    }
    value
}

fn keys_eq(left: &std::ffi::OsStr, right: &std::ffi::OsStr, ignore_case: bool) -> bool {
    if ignore_case {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}

fn upsert(
    pairs: &mut Vec<(OsString, OsString)>,
    key: OsString,
    value: OsString,
    ignore_case: bool,
) {
    if let Some((_, slot)) = pairs
        .iter_mut()
        .find(|(existing, _)| keys_eq(existing, &key, ignore_case))
    {
        *slot = value;
    } else {
        pairs.push((key, value));
    }
}

fn remove_key(pairs: &mut Vec<(OsString, OsString)>, name: &str, ignore_case: bool) {
    let needle = OsString::from(name);
    pairs.retain(|(key, _)| !keys_eq(key, &needle, ignore_case));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_export_and_rejects_unknown_lines() {
        let text = "export FOO=bar\nBAZ=\"qu x\"\n# c\n\nQUX='a b'\nKEEP=\"a $b\"\n";
        let parsed = parse_dotenv(text, Path::new("f.env")).unwrap();
        let map: HashMap<_, _> = parsed.into_iter().collect();
        assert_eq!(map.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(map.get("BAZ").map(String::as_str), Some("qu x"));
        assert_eq!(map.get("QUX").map(String::as_str), Some("a b"));
        assert_eq!(map.get("KEEP").map(String::as_str), Some("a $b"));

        let err = parse_dotenv("nope\n", Path::new("f.env")).unwrap_err();
        assert_eq!(err.code, 1);
        assert!(err.message.contains("f.env"));
        assert!(err.message.contains('1'));
        assert!(!err.message.contains("nope"));
    }

    #[test]
    fn unset_sits_between_dotenv_and_inline() {
        let parent = vec![(OsString::from("FOO"), OsString::from("parent"))];
        let dotenv = [
            ("FOO".to_string(), "file".to_string()),
            ("OTHER".to_string(), "file".to_string()),
        ];
        let env = merge_env(
            parent,
            dotenv,
            &["FOO".to_string()],
            &[("FOO".to_string(), OsString::from("inline"))],
            false,
        );
        assert_eq!(env.get("FOO"), Some("inline"));
        assert_eq!(env.get("OTHER"), Some("file"));
    }

    #[test]
    fn windows_env_keys_ignore_case() {
        let parent = vec![(OsString::from("Path"), OsString::from("a"))];
        let env = merge_env(
            parent,
            [("PATH".to_string(), "b".to_string())],
            &[],
            &[],
            true,
        );
        assert_eq!(env.pairs.len(), 1);
        assert_eq!(env.pairs[0].1, OsString::from("b"));
    }
}
