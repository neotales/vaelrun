use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::error::Error;

/// A parsed `#!` line after `env` and `-S` have been consumed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Shebang {
    pub interpreter: String,
    pub args: Vec<String>,
}

/// How to turn a shebang interpreter token into something trust can check.
#[derive(Debug)]
pub(crate) enum Interp {
    Absolute(PathBuf),
    Name(String),
    Missing(String),
}

pub(crate) fn read_shebang(path: &Path) -> Result<Option<Shebang>, Error> {
    let mut file = File::open(path).map_err(|err| Error::io(path, &err))?;
    let mut buf = [0u8; 1024];
    let n = file.read(&mut buf).map_err(|err| Error::io(path, &err))?;
    if n < 2 || &buf[..2] != b"#!" {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&buf[..n]);
    let line = text.lines().next().unwrap_or("");
    parse_shebang_line(line)
        .map(Some)
        .map_err(|_| Error::at(path, 1, "invalid shebang"))
}

/// Accept `#!interpreter`, `#!/usr/bin/env interpreter`, and `#!/usr/bin/env -S interpreter arg...`.
pub(crate) fn parse_shebang_line(line: &str) -> Result<Shebang, ()> {
    let rest = line.trim().strip_prefix("#!").ok_or(())?.trim();
    if rest.is_empty() {
        return Err(());
    }
    let tokens: Vec<String> = rest.split_whitespace().map(str::to_string).collect();
    if tokens.is_empty() {
        return Err(());
    }
    let (interpreter, args) = if is_env(&tokens[0]) {
        let mut tail = &tokens[1..];
        if tail.first().is_some_and(|token| token == "-S") {
            tail = &tail[1..];
        }
        if tail.is_empty() {
            return Err(());
        }
        (tail[0].clone(), tail[1..].to_vec())
    } else {
        (tokens[0].clone(), tokens[1..].to_vec())
    };
    Ok(Shebang { interpreter, args })
}

fn is_env(token: &str) -> bool {
    token == "env" || token == "/usr/bin/env" || token == "/bin/env"
}

/// Use an absolute interpreter that exists. Otherwise strip a leading `/usr/bin/` and resolve the name.
pub(crate) fn classify_interp(interpreter: &str) -> Interp {
    let path = Path::new(interpreter);
    if path.is_absolute() && path.is_file() {
        return Interp::Absolute(path.to_path_buf());
    }
    if let Some(name) = interpreter.strip_prefix("/usr/bin/")
        && !name.is_empty()
        && !name.contains('/')
    {
        return Interp::Name(name.to_string());
    }
    if path.is_absolute() {
        return Interp::Missing(interpreter.to_string());
    }
    Interp::Name(interpreter.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_s_splits_interpreter_and_args() {
        let shebang = parse_shebang_line("#!/usr/bin/env -S python3 -u").unwrap();
        assert_eq!(shebang.interpreter, "python3");
        assert_eq!(shebang.args, vec!["-u".to_string()]);

        let plain = parse_shebang_line("#!/usr/bin/env python3").unwrap();
        assert_eq!(plain.interpreter, "python3");
        assert!(plain.args.is_empty());

        let direct = parse_shebang_line("#!/bin/bash -e").unwrap();
        assert_eq!(direct.interpreter, "/bin/bash");
        assert_eq!(direct.args, vec!["-e".to_string()]);
    }
}
