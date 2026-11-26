use std::ffi::{OsStr, OsString};

use crate::error::Error;

/// Quote-aware split. `$s` is replaced after the split, so a path stays one argument.
pub(crate) fn apply_template(template: &str, subject: &OsStr) -> Result<Vec<OsString>, Error> {
    let tokens = split_quoted(template).map_err(|_| Error::new(1, "invalid extension template"))?;
    let subject_text = subject.to_string_lossy();
    Ok(tokens
        .into_iter()
        .map(|token| {
            if token == "$s" {
                subject.to_os_string()
            } else if token.contains("$s") {
                OsString::from(token.replace("$s", subject_text.as_ref()))
            } else {
                OsString::from(token)
            }
        })
        .collect())
}

pub(crate) fn split_quoted(input: &str) -> Result<Vec<String>, ()> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = input.chars().peekable();
    let mut in_token = false;
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
                in_token = true;
                continue;
            }
            if q == '"' && c == '\\' {
                match chars.next() {
                    Some(next @ ('\\' | '"' | '\'')) => cur.push(next),
                    Some(next) => {
                        cur.push('\\');
                        cur.push(next);
                    }
                    None => return Err(()),
                }
                in_token = true;
                continue;
            }
            cur.push(c);
            in_token = true;
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                in_token = true;
            }
            c if c.is_whitespace() => {
                if in_token {
                    out.push(std::mem::take(&mut cur));
                    in_token = false;
                }
            }
            _ => {
                cur.push(c);
                in_token = true;
            }
        }
    }
    if quote.is_some() {
        return Err(());
    }
    if in_token {
        out.push(cur);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_is_one_argument() {
        let argv = apply_template("bun $s", OsStr::new("/tmp/my file.ts")).unwrap();
        assert_eq!(
            argv,
            vec![OsString::from("bun"), OsString::from("/tmp/my file.ts"),]
        );
        let quoted = apply_template("echo \"hello world\" $s", OsStr::new("file.ts")).unwrap();
        assert_eq!(
            quoted,
            vec![
                OsString::from("echo"),
                OsString::from("hello world"),
                OsString::from("file.ts"),
            ]
        );
    }

    #[test]
    fn quoted_windows_injector_keeps_backslashes() {
        let tokens = split_quoted(r#""C:\\Tools\\env-replace.exe" secret:db resolved"#).unwrap();
        assert_eq!(
            tokens,
            vec![
                "C:\\Tools\\env-replace.exe".to_string(),
                "secret:db".to_string(),
                "resolved".to_string(),
            ]
        );
    }
}
