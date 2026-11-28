#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Graph {
    pub first: Pipeline,
    pub rest: Vec<(Sep, Pipeline)>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Pipeline {
    pub commands: Vec<Cmd>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Cmd {
    pub argv: Vec<String>,
    pub stdin: Option<String>,
    pub stdout: Option<(String, bool)>,
    pub stderr: Option<(String, bool)>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Sep {
    And,
    Or,
    Semi,
}

#[derive(Debug)]
enum Tok {
    Word(String),
    Pipe,
    And,
    Or,
    Semi,
    In,
    Out,
    OutAppend,
    Err,
    ErrAppend,
}

/// Expand `{{ .NAME }}` and `{{ env "NAME" }}`. Missing names become empty.
/// Any other `{{` is an error. The error does not include the line text.
pub(crate) fn expand_templates(
    line: &str,
    env: &std::collections::HashMap<String, String>,
) -> Result<String, ()> {
    let mut out = String::new();
    let mut rest = line;
    while let Some(idx) = rest.find("{{") {
        out.push_str(&rest[..idx]);
        let after = &rest[idx..];
        let Some(end) = after.find("}}") else {
            return Err(());
        };
        let inner = after[2..end].trim();
        let value = template_value(inner, env)?;
        out.push_str(&value);
        rest = &after[end + 2..];
    }
    if rest.contains("{{") {
        return Err(());
    }
    out.push_str(rest);
    Ok(out)
}

fn template_value(
    inner: &str,
    env: &std::collections::HashMap<String, String>,
) -> Result<String, ()> {
    if let Some(name) = inner.strip_prefix('.') {
        let name = name.trim();
        if !crate::cli::is_env_name(name) {
            return Err(());
        }
        return Ok(env.get(name).cloned().unwrap_or_default());
    }
    let Some(rest) = inner.strip_prefix("env") else {
        return Err(());
    };
    let rest = rest.trim_start();
    let Some(rest) = rest.strip_prefix('"') else {
        return Err(());
    };
    let Some(name) = rest.strip_suffix('"') else {
        return Err(());
    };
    if !crate::cli::is_env_name(name) {
        return Err(());
    }
    Ok(env.get(name).cloned().unwrap_or_default())
}

pub(crate) fn parse_script_line(line: &str) -> Result<Option<Graph>, String> {
    let tokens = tokenize(line)?;
    if tokens.is_empty() {
        return Ok(None);
    }
    let (first, mut i) = parse_pipeline(&tokens, 0)?;
    let mut rest = Vec::new();
    while i < tokens.len() {
        let sep = match &tokens[i] {
            Tok::And => Sep::And,
            Tok::Or => Sep::Or,
            Tok::Semi => Sep::Semi,
            _ => return Err("expected &&, ||, or ;".into()),
        };
        i += 1;
        let (pipe, next) = parse_pipeline(&tokens, i)?;
        rest.push((sep, pipe));
        i = next;
    }
    Ok(Some(Graph { first, rest }))
}

fn parse_pipeline(tokens: &[Tok], mut i: usize) -> Result<(Pipeline, usize), String> {
    let mut commands = Vec::new();
    loop {
        let (cmd, next) = parse_command(tokens, i)?;
        commands.push(cmd);
        i = next;
        if i < tokens.len() && matches!(tokens[i], Tok::Pipe) {
            i += 1;
            continue;
        }
        break;
    }
    if commands.is_empty() {
        return Err("empty command".into());
    }
    Ok((Pipeline { commands }, i))
}

fn parse_command(tokens: &[Tok], mut i: usize) -> Result<(Cmd, usize), String> {
    let mut cmd = Cmd {
        argv: Vec::new(),
        stdin: None,
        stdout: None,
        stderr: None,
    };
    let mut saw = false;
    while i < tokens.len() && !is_sep(&tokens[i]) {
        match &tokens[i] {
            Tok::Word(word) => {
                cmd.argv.push(word.clone());
                saw = true;
                i += 1;
            }
            Tok::In => {
                i += 1;
                cmd.stdin = Some(expect_word(tokens, &mut i)?);
                saw = true;
            }
            Tok::Out => {
                i += 1;
                cmd.stdout = Some((expect_word(tokens, &mut i)?, false));
                saw = true;
            }
            Tok::OutAppend => {
                i += 1;
                cmd.stdout = Some((expect_word(tokens, &mut i)?, true));
                saw = true;
            }
            Tok::Err => {
                i += 1;
                cmd.stderr = Some((expect_word(tokens, &mut i)?, false));
                saw = true;
            }
            Tok::ErrAppend => {
                i += 1;
                cmd.stderr = Some((expect_word(tokens, &mut i)?, true));
                saw = true;
            }
            _ => return Err("empty command".into()),
        }
    }
    if !saw || cmd.argv.is_empty() {
        return Err("empty command".into());
    }
    Ok((cmd, i))
}

fn expect_word(tokens: &[Tok], i: &mut usize) -> Result<String, String> {
    match tokens.get(*i) {
        Some(Tok::Word(word)) => {
            *i += 1;
            Ok(word.clone())
        }
        _ => Err("redirection needs a path".into()),
    }
}

fn is_sep(tok: &Tok) -> bool {
    matches!(tok, Tok::Pipe | Tok::And | Tok::Or | Tok::Semi)
}

fn tokenize(line: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = line.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        if chars[i] == '#' {
            break;
        }
        if chars[i] == '|' && peek(&chars, i + 1) == Some('|') {
            tokens.push(Tok::Or);
            i += 2;
            continue;
        }
        if chars[i] == '|' {
            tokens.push(Tok::Pipe);
            i += 1;
            continue;
        }
        if chars[i] == '&' {
            if peek(&chars, i + 1) == Some('&') {
                tokens.push(Tok::And);
                i += 2;
                continue;
            }
            return Err("unsupported operator".into());
        }
        if chars[i] == ';' {
            tokens.push(Tok::Semi);
            i += 1;
            continue;
        }
        if chars[i] == '2' && peek(&chars, i + 1) == Some('>') {
            if peek(&chars, i + 2) == Some('>') {
                tokens.push(Tok::ErrAppend);
                i += 3;
            } else {
                tokens.push(Tok::Err);
                i += 2;
            }
            i = take_attached(&chars, i, &mut tokens)?;
            continue;
        }
        if chars[i] == '>' {
            if peek(&chars, i + 1) == Some('>') {
                tokens.push(Tok::OutAppend);
                i += 2;
            } else {
                tokens.push(Tok::Out);
                i += 1;
            }
            i = take_attached(&chars, i, &mut tokens)?;
            continue;
        }
        if chars[i] == '<' {
            tokens.push(Tok::In);
            i += 1;
            i = take_attached(&chars, i, &mut tokens)?;
            continue;
        }
        let (word, next) = scan_word(&chars, i)?;
        tokens.push(Tok::Word(word));
        i = next;
    }
    Ok(tokens)
}

fn take_attached(chars: &[char], i: usize, tokens: &mut Vec<Tok>) -> Result<usize, String> {
    if i < chars.len() && !chars[i].is_whitespace() && !is_op_start(chars[i]) {
        let (word, next) = scan_word(chars, i)?;
        tokens.push(Tok::Word(word));
        Ok(next)
    } else {
        Ok(i)
    }
}

fn is_op_start(c: char) -> bool {
    matches!(c, '|' | '&' | ';' | '>' | '<' | '#')
}

fn peek(chars: &[char], i: usize) -> Option<char> {
    chars.get(i).copied()
}

fn scan_word(chars: &[char], mut i: usize) -> Result<(String, usize), String> {
    let mut out = String::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() || is_op_start(c) {
            break;
        }
        if c == '\'' || c == '"' {
            let quote = c;
            i += 1;
            let mut closed = false;
            while i < chars.len() {
                if chars[i] == quote {
                    closed = true;
                    i += 1;
                    break;
                }
                out.push(chars[i]);
                i += 1;
            }
            if !closed {
                return Err("unclosed quote".into());
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    if out.is_empty() {
        return Err("empty command".into());
    }
    Ok((out, i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn and_or_pipe_shape() {
        let graph = parse_script_line("true && false || true").unwrap().unwrap();
        assert!(graph.first.commands[0].argv == ["true"]);
        assert_eq!(graph.rest.len(), 2);
        assert_eq!(graph.rest[0].0, Sep::And);
        assert_eq!(graph.rest[1].0, Sep::Or);

        let piped = parse_script_line("printf hello | cat").unwrap().unwrap();
        assert_eq!(piped.first.commands.len(), 2);
        assert!(piped.rest.is_empty());
    }

    #[test]
    fn templates_expand_and_reject_unknown() {
        let mut env = std::collections::HashMap::new();
        env.insert("NAME".to_string(), "ada".to_string());
        let line = expand_templates("echo {{ .NAME }} {{ env \"NAME\" }}", &env).unwrap();
        assert_eq!(line, "echo ada ada");
        let missing = expand_templates("echo {{ .MISSING }}", &env).unwrap();
        assert_eq!(missing, "echo ");
        assert!(expand_templates("echo {{ foo }}", &env).is_err());
    }
}
