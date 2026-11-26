use std::collections::{HashMap, HashSet};
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
    let mut inline_keys = HashSet::new();
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
        inline_keys.insert(stored_key(&env, key));
    }

    if let Some(injector) = injector {
        let stdout = run_injector(
            injector.program,
            injector.argv,
            &serialize(&env, &quotes),
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

    let expanded = expand(env, &inline_keys);
    Ok(EnvMap::from_environment(&expanded, non_utf8))
}

/// Interpolate command-line assignments only.
///
/// A dotenv value is literal, whatever its quotes. The parent environment was
/// already expanded by the shell. An inline assignment is the one case the
/// shell could not expand: `MY_VAR='$HOME'` arrives as the token `MY_VAR=$HOME`.
/// A key the injector added is not inline, so it stays literal. A key the
/// injector rewrote is still inline, so the returned value is expanded.
fn expand(source: Environment, inline_keys: &HashSet<String>) -> Environment {
    let mut out = Environment::new(Platform::host());
    for (key, value) in source.as_map() {
        let next = if inline_keys.iter().any(|name| keys_equal(name, key)) {
            neotales_env::interpolate(value, &out)
        } else {
            value.clone()
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

fn serialize(env: &Environment, quotes: &HashMap<String, QuoteStyle>) -> String {
    let mut out = String::new();
    for (key, value) in env.as_map() {
        out.push_str(key);
        out.push('=');
        out.push_str(&encode_value(value, quotes.get(key).copied()));
        out.push('\n');
    }
    out
}

/// Encode `value` so `neotales_dotenv::parse` reports the same characters and a
/// quote style that keeps the same expansion rule.
///
/// The parser strips one opening delimiter and one trailing delimiter. It does
/// not unescape `\"` or understand `'\''`, so those forms are not emitted.
/// Wrapping `'value'` still round-trips an embedded `'`. The case that does
/// not: a literal value that contains a newline and a line ending in both `'`
/// and `` ` ``. Neither quote can wrap it. That value is emitted in an
/// expandable form, and `$` in it will expand. That case is skipped.
fn encode_value(value: &str, style: Option<QuoteStyle>) -> String {
    let literal = matches!(style, Some(QuoteStyle::Single | QuoteStyle::Raw));
    if literal && let Some(encoded) = literal_form(value) {
        return encoded;
    }
    expandable_form(value)
}

fn literal_form(value: &str) -> Option<String> {
    for (delimiter, style) in [('\'', QuoteStyle::Single), ('`', QuoteStyle::Raw)] {
        let encoded = wrap(value, delimiter);
        if round_trips(&encoded, value, style) {
            return Some(encoded);
        }
    }
    None
}

fn expandable_form(value: &str) -> String {
    if !needs_quotes(value)
        && let Some(encoded) = unquoted_if_round_trips(value)
    {
        return encoded;
    }
    let encoded = wrap(value, '"');
    if round_trips(&encoded, value, QuoteStyle::Double) {
        return encoded;
    }
    if let Some(encoded) = unquoted_if_round_trips(value) {
        return encoded;
    }
    // Last resort: do not invent backslash escapes the parser will keep.
    value.to_string()
}

fn unquoted_if_round_trips(value: &str) -> Option<String> {
    if round_trips(value, value, QuoteStyle::Unquoted) {
        Some(value.to_string())
    } else {
        None
    }
}

fn needs_quotes(value: &str) -> bool {
    value.contains([' ', '#', '\n', '\r'])
        || value.starts_with(char::is_whitespace)
        || value.ends_with(char::is_whitespace)
}

fn wrap(value: &str, delimiter: char) -> String {
    let mut encoded = String::new();
    encoded.push(delimiter);
    encoded.push_str(value);
    encoded.push(delimiter);
    encoded
}

fn round_trips(encoded: &str, value: &str, style: QuoteStyle) -> bool {
    let document = neotales_dotenv::parse(&format!("K={encoded}\n"));
    if document.first_invalid().is_some() {
        return false;
    }
    let mut assignments = document.assignments();
    let Some(assignment) = assignments.next() else {
        return false;
    };
    assignments.next().is_none()
        && assignment.value == value
        && assignment.quote == style
        && assignment.key == "K"
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
    fn dotenv_values_stay_literal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        fs::write(
            &path,
            "BAR=there\nFOO=${BAR}\nKEEP=\"a $b\"\nLIT='${BAR}'\n",
        )
        .unwrap();
        let env = build_env(&[], &[path], &[], &[], dir.path(), None).unwrap();
        assert_eq!(env.get("FOO"), Some("${BAR}"));
        assert_eq!(env.get("KEEP"), Some("a $b"));
        assert_eq!(env.get("LIT"), Some("${BAR}"));
    }

    #[test]
    fn serializer_round_trips_quote_style() {
        let mut env = Environment::new(Platform::Unix);
        env.insert("PLAIN", "ok");
        env.insert("SPACE", "a b");
        env.insert("DOLLAR", "x$y");
        env.insert("QUOTE", "say \"hi\"");
        env.insert("LIT", "${HOME}");
        env.insert("EMBED", "it's");
        let mut quotes = HashMap::new();
        quotes.insert("LIT".to_string(), QuoteStyle::Single);
        quotes.insert("EMBED".to_string(), QuoteStyle::Single);
        let text = serialize(&env, &quotes);
        assert!(text.contains("DOLLAR=x$y\n"), "{text}");
        assert!(!text.contains("\\\""), "{text}");
        let document = neotales_dotenv::parse(&text);
        assert!(document.first_invalid().is_none(), "{text}");
        let lit = document
            .assignments()
            .find(|item| item.key == "LIT")
            .unwrap();
        assert_eq!(lit.value, "${HOME}");
        assert_eq!(lit.quote, QuoteStyle::Single);
        let embed = document
            .assignments()
            .find(|item| item.key == "EMBED")
            .unwrap();
        assert_eq!(embed.value, "it's");
        assert_eq!(embed.quote, QuoteStyle::Single);
        let dollar = document
            .assignments()
            .find(|item| item.key == "DOLLAR")
            .unwrap();
        assert_eq!(dollar.value, "x$y");
        assert_ne!(dollar.quote, QuoteStyle::Single);
    }

    #[test]
    fn shell_single_quote_escape_does_not_round_trip() {
        let document = neotales_dotenv::parse("K='it'\\''s'\n");
        let assignment = document.assignments().next().unwrap();
        assert_ne!(assignment.value, "it's");
    }
}
