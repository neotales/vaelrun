use std::io::{self, Write};
use std::path::Path;

use crate::config::Config;
use crate::error::Error;
use crate::resolve::is_executable;

/// Print a completion script. Does not edit the user's shell rc.
pub(crate) fn print_complete(
    shell: &str,
    config: &Config,
    bin_dirs: &[std::path::PathBuf],
) -> Result<i32, Error> {
    let words = completion_words(config, bin_dirs);
    let script = match shell {
        "bash" => bash_script(&words),
        "zsh" => zsh_script(&words),
        "fish" => fish_script(&words),
        "pwsh" => pwsh_script(&words),
        _ => return Err(Error::usage(format!("unknown shell `{shell}`"))),
    };
    let mut out = io::stdout().lock();
    out.write_all(script.as_bytes())
        .map_err(|err| Error::new(1, format!("stdout: {err}")))?;
    out.flush()
        .map_err(|err| Error::new(1, format!("stdout: {err}")))?;
    Ok(0)
}

fn completion_words(config: &Config, bin_dirs: &[std::path::PathBuf]) -> Vec<String> {
    let mut words = vec![
        "open".to_string(),
        "start".to_string(),
        "run".to_string(),
        "complete".to_string(),
        "-h".to_string(),
        "--help".to_string(),
        "--version".to_string(),
        "-e".to_string(),
        "--edit".to_string(),
        "--env-file".to_string(),
        "-C".to_string(),
        "--chdir".to_string(),
        "-u".to_string(),
        "--unset".to_string(),
        "--trust".to_string(),
        "--no-trust".to_string(),
        "--detach".to_string(),
        "--sudo".to_string(),
    ];
    for name in config.aliases.keys() {
        if safe_word(name) {
            words.push(name.clone());
        }
    }
    for dir in bin_dirs {
        push_bin_names(dir, &mut words);
    }
    words.sort();
    words.dedup();
    words
}

fn push_bin_names(dir: &Path, words: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_executable(&path) {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|name| name.to_str())
            && safe_word(name)
        {
            words.push(name.to_string());
        }
    }
}

fn safe_word(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

fn bash_script(words: &[String]) -> String {
    format!(
        "# vrun bash completion\n_vrun() {{\n  local cur=\"${{COMP_WORDS[COMP_CWORD]}}\"\n  COMPREPLY=( $(compgen -W {list} -- \"${{cur}}\") )\n}}\ncomplete -F _vrun vrun\n",
        list = shell_quote(&words.join(" ")),
    )
}

fn zsh_script(words: &[String]) -> String {
    let list = words
        .iter()
        .map(|word| shell_quote(word))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "#compdef vrun\n_vrun() {{\n  local -a cmds\n  cmds=({list})\n  _describe 'vrun' cmds\n}}\ncompdef _vrun vrun\n"
    )
}

fn fish_script(words: &[String]) -> String {
    format!(
        "# vrun fish completion\ncomplete -c vrun -f -a {}\n",
        shell_quote(&words.join(" "))
    )
}

fn pwsh_script(words: &[String]) -> String {
    let list = words
        .iter()
        .map(|word| format!("'{}'", word.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "Register-ArgumentCompleter -CommandName vrun -ScriptBlock {{\n  param($commandName, $wordToComplete)\n  @({list}) | Where-Object {{ $_ -like \"$wordToComplete*\" }}\n}}\n"
    )
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}
