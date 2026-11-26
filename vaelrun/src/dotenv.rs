use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use neotales_dotenv::{Document, QuoteStyle};
use neotales_env::Platform;

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

    fn from_document(document: &Document, non_utf8: Vec<(OsString, OsString)>) -> Self {
        let mut pairs = Vec::new();
        let mut utf8 = HashMap::new();
        for key in document.keys() {
            if keys_equal(key, "VAELRUN_DETACHED") {
                continue;
            }
            let Some(value) = document.get(key) else {
                continue;
            };
            pairs.push((OsString::from(key), OsString::from(value)));
            utf8.insert(key.to_string(), value.to_string());
        }
        for (key, value) in non_utf8 {
            if !pairs.iter().any(|(existing, _)| existing == &key) {
                pairs.push((key, value));
            }
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
///
/// Dotenv values stay literal. Parent values stay as the shell left them. Only
/// command-line assignments are expanded, after the injector returns, against
/// values already inserted. A key only the injector added is not expanded.
pub(crate) fn build_env(
    parent: &[(OsString, OsString)],
    dotenv_files: &[PathBuf],
    unset: &[String],
    inline: &[(String, OsString)],
    cwd: &Path,
    injector: Option<Injector<'_>>,
) -> Result<EnvMap, Error> {
    let mut document = Document::new();
    let mut inline_keys: HashSet<String> = HashSet::new();
    let mut non_utf8 = Vec::new();

    for (key, value) in parent {
        if key.to_str() == Some("VAELRUN_DETACHED") {
            continue;
        }
        let (Some(key), Some(value)) = (key.to_str(), value.to_str()) else {
            non_utf8.push((key.clone(), value.clone()));
            continue;
        };
        put(&mut document, key, value, None);
    }
    for path in dotenv_files {
        let text = fs::read_to_string(path).map_err(|err| Error::io(path, &err))?;
        let parsed = neotales_dotenv::parse(&text);
        reject_invalid(path, &parsed)?;
        for assignment in parsed.assignments() {
            put(
                &mut document,
                &assignment.key,
                &assignment.value,
                Some(assignment.quote),
            );
        }
    }
    for name in unset {
        remove_key(&mut document, name);
        inline_keys.retain(|key| !keys_equal(key, name));
    }
    for (key, value) in inline {
        let Some(value) = value.to_str() else {
            continue;
        };
        put(&mut document, key, value, None);
        inline_keys.insert(stored_key(&document, key));
    }

    if let Some(injector) = injector {
        let stdout = run_injector(
            injector.program,
            injector.argv,
            &document.to_string(),
            cwd,
            parent,
        )?;
        let parsed = neotales_dotenv::parse(&stdout);
        if let Some((_, span)) = parsed.first_invalid() {
            return Err(Error::new(
                1,
                format!(
                    "injector {}:{}: invalid dotenv line",
                    injector.program.display(),
                    span.line
                ),
            ));
        }
        document = Document::new();
        for assignment in parsed.assignments() {
            put(
                &mut document,
                &assignment.key,
                &assignment.value,
                Some(assignment.quote),
            );
        }
    }

    let expanded = expand(&document, &inline_keys);
    Ok(EnvMap::from_document(&expanded, non_utf8))
}

/// Expand command-line assignments against values already inserted.
///
/// `expand_with` is documented to treat an unset name as empty. If it returns
/// an error for that case anyway, the value becomes empty so a missing name
/// does not fail the run.
fn expand(source: &Document, inline_keys: &HashSet<String>) -> Document {
    let mut out = Document::new();
    for key in source.keys() {
        if keys_equal(key, "VAELRUN_DETACHED") {
            continue;
        }
        let value = source.get(key).unwrap_or("");
        let next = if inline_keys.iter().any(|name| keys_equal(name, key)) {
            expand_inline(value, &out)
        } else {
            value.to_string()
        };
        let quote = quote_of(source, key);
        out.add_quoted_variable(key, &next, quote);
    }
    out
}

fn expand_inline(value: &str, prior: &Document) -> String {
    neotales_env::expand_with(value, |name| doc_get(prior, name)).unwrap_or_default()
}

fn put(document: &mut Document, key: &str, value: &str, quote: Option<QuoteStyle>) {
    let style = quote.unwrap_or_else(|| write_quote(value));
    if document.keys().any(|existing| keys_equal(existing, key)) {
        let stored = stored_key(document, key);
        document.add_quoted_variable(&stored, value, style);
    } else {
        document.add_quoted_variable(key, value, style);
    }
}

fn write_quote(value: &str) -> QuoteStyle {
    if value.contains([' ', '#', '\n', '\r', '"', '\''])
        || value.starts_with(char::is_whitespace)
        || value.ends_with(char::is_whitespace)
    {
        QuoteStyle::Double
    } else {
        QuoteStyle::Unquoted
    }
}

fn quote_of(document: &Document, key: &str) -> QuoteStyle {
    document
        .assignments()
        .find(|assignment| keys_equal(&assignment.key, key))
        .map(|assignment| assignment.quote)
        .unwrap_or(QuoteStyle::Unquoted)
}

fn remove_key(document: &mut Document, name: &str) {
    let stored: Vec<String> = document
        .keys()
        .filter(|key| keys_equal(key, name))
        .map(str::to_string)
        .collect();
    if stored.is_empty() {
        return;
    }
    let mut rebuilt = Document::new();
    for key in document.keys() {
        if stored.iter().any(|gone| keys_equal(gone, key)) {
            continue;
        }
        let value = document.get(key).unwrap_or("");
        rebuilt.add_quoted_variable(key, value, quote_of(document, key));
    }
    *document = rebuilt;
}

fn stored_key(document: &Document, key: &str) -> String {
    document
        .keys()
        .find(|existing| keys_equal(existing, key))
        .unwrap_or(key)
        .to_string()
}

fn doc_get<'a>(document: &'a Document, key: &str) -> Option<&'a str> {
    if let Some(value) = document.get(key) {
        return Some(value);
    }
    document
        .keys()
        .find(|existing| keys_equal(existing, key))
        .and_then(|existing| document.get(existing))
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
}
