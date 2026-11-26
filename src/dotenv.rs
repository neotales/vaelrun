use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use neotales_dotenv::{Document, QuoteStyle};
use neotales_env::{Environment, Platform};

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

    fn from_environment(env: &Environment, non_utf8: Vec<(OsString, OsString)>) -> Self {
        let mut pairs: Vec<(OsString, OsString)> = env
            .as_map()
            .iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value)))
            .collect();
        for (key, value) in non_utf8 {
            if !pairs.iter().any(|(existing, _)| existing == &key) {
                pairs.push((key, value));
            }
        }
        let mut utf8 = HashMap::new();
        for (key, value) in env.as_map() {
            utf8.insert(key.clone(), value.clone());
        }
        Self { pairs, utf8 }
    }
}

/// Parent pairs, skipping the detach mark, for resolving the injector.
pub(crate) fn parent_map(parent: &[(OsString, OsString)]) -> EnvMap {
    let mut pairs = Vec::new();
    let mut utf8 = HashMap::new();
    for (key, value) in parent {
        if key.to_str() == Some("VAELRUN_DETACHED") {
            continue;
        }
        pairs.push((key.clone(), value.clone()));
        if let (Some(key), Some(value)) = (key.to_str(), value.to_str()) {
            utf8.insert(key.to_string(), value.to_string());
        }
    }
    EnvMap { pairs, utf8 }
}

/// An injector whose program has already passed the trust resolver.
pub(crate) struct Injector<'a> {
    pub argv: &'a [String],
    pub program: &'a Path,
}

/// Parent, then dotenv, then `-u`, then inline. An injector may replace that map.
/// Expansion runs after the injector, against values inserted so far.
pub(crate) fn build_env(
    parent: &[(OsString, OsString)],
    dotenv_files: &[PathBuf],
    unset: &[String],
    inline: &[(String, OsString)],
    cwd: &Path,
    injector: Option<Injector<'_>>,
) -> Result<EnvMap, Error> {
    let mut env = Environment::new(Platform::host());
    let mut quotes: HashMap<String, QuoteStyle> = HashMap::new();
    let mut non_utf8 = Vec::new();

    for (key, value) in parent {
        if key.to_str() == Some("VAELRUN_DETACHED") {
            continue;
        }
        let (Some(key), Some(value)) = (key.to_str(), value.to_str()) else {
            non_utf8.push((key.clone(), value.clone()));
            continue;
        };
        put(&mut env, &mut quotes, key, value, None);
    }
    for path in dotenv_files {
        let text = fs::read_to_string(path).map_err(|err| Error::io(path, &err))?;
        let document = neotales_dotenv::parse(&text);
        reject_invalid(path, &document)?;
        for assignment in document.assignments() {
            put(
                &mut env,
                &mut quotes,
                &assignment.key,
                &assignment.value,
                Some(assignment.quote),
            );
        }
    }
    for name in unset {
        env.remove(name);
        quotes.retain(|key, _| !keys_equal(key, name));
    }
    for (key, value) in inline {
        let Some(value) = value.to_str() else {
            continue;
        };
        put(&mut env, &mut quotes, key, value, None);
    }

    if let Some(injector) = injector {
        let stdout = run_injector(
            injector.program,
            injector.argv,
            &serialize(&env),
            cwd,
            parent,
        )?;
        let document = neotales_dotenv::parse(&stdout);
        if let Some((_, span)) = document.first_invalid() {
            return Err(Error::new(
                1,
                format!(
                    "injector {}:{}: invalid dotenv line",
                    injector.program.display(),
                    span.line
                ),
            ));
        }
        env = Environment::new(Platform::host());
        quotes.clear();
        for assignment in document.assignments() {
            put(
                &mut env,
                &mut quotes,
                &assignment.key,
                &assignment.value,
                Some(assignment.quote),
            );
        }
    }

    let expanded = expand(env, &quotes);
    Ok(EnvMap::from_environment(&expanded, non_utf8))
}

fn expand(source: Environment, quotes: &HashMap<String, QuoteStyle>) -> Environment {
    let mut out = Environment::new(Platform::host());
    for (key, value) in source.as_map() {
        let literal = matches!(quotes.get(key), Some(QuoteStyle::Single | QuoteStyle::Raw));
        let next = if literal {
            value.clone()
        } else {
            neotales_env::interpolate(value, &out)
        };
        out.insert(key, next);
    }
    out.remove("VAELRUN_DETACHED");
    out
}

fn put(
    env: &mut Environment,
    quotes: &mut HashMap<String, QuoteStyle>,
    key: &str,
    value: &str,
    quote: Option<QuoteStyle>,
) {
    env.insert(key, value);
    let stored = stored_key(env, key);
    match quote {
        Some(style) => {
            quotes.insert(stored, style);
        }
        None => {
            quotes.remove(&stored);
        }
    }
}

fn stored_key(env: &Environment, key: &str) -> String {
    env.as_map()
        .keys()
        .find(|existing| keys_equal(existing, key))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

fn keys_equal(left: &str, right: &str) -> bool {
    match Platform::host() {
        Platform::Unix => left == right,
        Platform::Windows => left.eq_ignore_ascii_case(right),
    }
}

fn reject_invalid(path: &Path, document: &Document) -> Result<(), Error> {
    if let Some((_, span)) = document.first_invalid() {
        return Err(Error::at(
            path,
            usize::try_from(span.line).unwrap_or(0),
            "invalid dotenv line",
        ));
    }
    Ok(())
}

fn serialize(env: &Environment) -> String {
    let mut out = String::new();
    for (key, value) in env.as_map() {
        out.push_str(key);
        out.push('=');
        out.push_str(&encode_value(value));
        out.push('\n');
    }
    out
}

fn encode_value(value: &str) -> String {
    let quote = value.contains([' ', '#', '\'', '"', '$', '\n'])
        || value.starts_with(' ')
        || value.ends_with(' ');
    if !quote {
        return value.to_string();
    }
    let mut encoded = String::from("\"");
    for ch in value.chars() {
        if ch == '"' || ch == '\\' {
            encoded.push('\\');
        }
        encoded.push(ch);
    }
    encoded.push('"');
    encoded
}

fn run_injector(
    program: &Path,
    argv: &[String],
    document: &str,
    cwd: &Path,
    parent: &[(OsString, OsString)],
) -> Result<String, Error> {
    let mut cmd = Command::new(program);
    if argv.len() > 1 {
        cmd.args(&argv[1..]);
    }
    cmd.current_dir(cwd);
    cmd.env_clear();
    cmd.envs(parent.iter().cloned());
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::inherit());
    let mut child = cmd.spawn().map_err(|err| Error::io(program, &err))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(document.as_bytes())
            .map_err(|err| Error::io(program, &err))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|err| Error::io(program, &err))?;
    if !output.status.success() {
        let code = output.status.code().unwrap_or(1);
        return Err(Error::new(
            1,
            format!("injector {}: status {code}", program.display()),
        ));
    }
    String::from_utf8(output.stdout).map_err(|_| {
        Error::new(
            1,
            format!("injector {}: stdout is not utf-8", program.display()),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_line_names_file_and_line_not_the_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.env");
        fs::write(&path, "nope\n").unwrap();
        let err = build_env(&[], &[path], &[], &[], dir.path(), None).unwrap_err();
        assert_eq!(err.code, 1);
        assert!(err.message.contains("f.env"));
        assert!(err.message.contains('1'));
        assert!(!err.message.contains("nope"));
    }

    #[test]
    fn expands_in_order_and_keeps_single_quotes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        fs::write(
            &path,
            "BAR=there\nFOO=${BAR}\nKEEP=\"a $b\"\nLIT='${BAR}'\n",
        )
        .unwrap();
        let env = build_env(&[], &[path], &[], &[], dir.path(), None).unwrap();
        assert_eq!(env.get("FOO"), Some("there"));
        assert_eq!(env.get("KEEP"), Some("a "));
        assert_eq!(env.get("LIT"), Some("${BAR}"));
    }

    #[test]
    fn quoted_serializer_escapes_quotes_and_dollars() {
        let mut env = Environment::new(Platform::Unix);
        env.insert("PLAIN", "ok");
        env.insert("SPACE", "a b");
        env.insert("DOLLAR", "x$y");
        env.insert("QUOTE", "say \"hi\"");
        let text = serialize(&env);
        assert!(text.contains("PLAIN=ok\n"), "{text}");
        assert!(text.contains("SPACE=\"a b\"\n"), "{text}");
        assert!(text.contains("DOLLAR=\"x$y\"\n"), "{text}");
        assert!(text.contains("QUOTE=\"say \\\"hi\\\"\"\n"), "{text}");
    }
}
