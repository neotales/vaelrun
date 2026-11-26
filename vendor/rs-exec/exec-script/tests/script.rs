//! The argument vectors each runtime builds.

use neotales_exec::{Command, which};
use neotales_exec_script::{
    awk, bash, bun, cmd, dart, deno, dotnet, elixir, fish, golang, groovy, julia, lua, node,
    nushell, perl, php, powershell, pwsh, python, r, racket, ruby, sh, tcl, uv, zsh,
};

fn args(command: &Command) -> Vec<String> {
    command
        .arguments()
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn bash_inline_puts_script_after_dash_c() {
    let command = bash::inline("echo hi", []);

    assert!(command.program().to_string_lossy().contains("bash"));
    assert_eq!(
        args(&command),
        [
            "--noprofile",
            "--norc",
            "-eo",
            "pipefail",
            "-c",
            "echo hi",
            "bash"
        ]
    );
}

// The assertion assumes a Unix program name.
#[cfg(unix)]
#[test]
fn bash_script_treats_one_sh_line_as_a_file() {
    let command = bash::script("  script.sh  ", ["one"]);

    assert_eq!(command.arguments()[4], "script.sh");
    assert_eq!(args(&command)[5], "one");
}

#[test]
fn sh_inline_passes_dash_c() {
    let command = sh::inline("echo hi", []);

    assert_eq!(args(&command), ["-e", "-c", "echo hi", "sh"]);
}

#[test]
fn node_script_chooses_file_or_inline() {
    let file = node::script("app.js", ["--x"]);
    assert_eq!(args(&file), ["app.js", "--x"]);
    assert!(file.program().to_string_lossy().contains("node"));

    let inline = node::script("console.log(1)", []);
    assert_eq!(args(&inline), ["-e", "console.log(1)"]);
}

#[test]
fn bun_script_chooses_file_or_inline() {
    assert_eq!(args(&bun::script("app.ts", [])), ["app.ts"]);
    assert_eq!(
        args(&bun::script("console.log(1)", [])),
        ["-e", "console.log(1)"]
    );
}

#[test]
fn cmd_file_runs_through_dash_c() {
    let command = cmd::file("a.bat", ["1"]);

    assert_eq!(args(&command), ["/d", "/c", "a.bat", "1"]);
}

#[test]
fn golang_file_runs_the_file() {
    let command = golang::file("main.go", []);

    assert_eq!(args(&command), ["run", "main.go"]);
}

#[test]
fn golang_inline_writes_a_temporary_file() {
    let command = golang::inline("package main", []).expect("inline Go");

    let temp = command.temp_file().expect("temp file");
    #[cfg(unix)]
    assert!(temp.starts_with(std::env::var_os("HOME").unwrap()));
    assert!(temp.to_string_lossy().ends_with(".go"));
    assert_eq!(
        args(&command),
        vec!["run".to_owned(), temp.to_string_lossy().into_owned()]
    );
}

#[test]
fn inline_scripts_use_distinct_private_files() {
    let go = golang::inline("same source", []).expect("inline Go");
    let other = golang::script("same source", []).expect("Go script");
    let cs = dotnet::script("same source", []).expect(".NET script");
    let paths = [
        go.temp_file().unwrap(),
        other.temp_file().unwrap(),
        cs.temp_file().unwrap(),
    ];
    assert_ne!(paths[0], paths[1]);
    assert_ne!(paths[1].parent(), paths[0].parent());
    for path in paths {
        assert_eq!(std::fs::read(path).unwrap(), b"same source");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
    }
}

#[test]
fn dotnet_file_separates_program_arguments() {
    let command = dotnet::file("app.cs", ["a"]);

    assert_eq!(args(&command), ["run", "--file", "app.cs", "--", "a"]);
    assert_eq!(
        args(&dotnet::file("app.cs", [])),
        ["run", "--file", "app.cs"]
    );
    assert_eq!(args(&dotnet::file("lib.dll", ["a"])), ["lib.dll", "a"]);
    assert_eq!(
        args(&dotnet::file("app.csproj", [])),
        ["run", "--project", "app.csproj"]
    );
}

#[test]
fn powershell_and_pwsh_share_the_shape() {
    let expected = [
        "-NoLogo",
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        "a.ps1",
    ];
    assert_eq!(args(&powershell::file("a.ps1", [])), expected);
    assert_eq!(args(&pwsh::file("a.ps1", [])), expected);
    assert_eq!(
        args(&pwsh::script("Write-Output hi", [])),
        [
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "Write-Output hi"
        ]
    );
}

#[test]
fn bash_inline_passes_the_first_argument_as_dollar_one() {
    if which("bash").is_none() && !std::path::Path::new("/bin/bash").exists() {
        return;
    }

    let output = bash::inline("echo \"$1\"", ["hello"])
        .output()
        .expect("bash");
    assert_eq!(output.text().trim(), "hello");
}

#[test]
fn sh_inline_passes_the_first_argument_as_dollar_one() {
    if which("sh").is_none() && !std::path::Path::new("/bin/sh").exists() {
        return;
    }

    let output = sh::inline("echo \"$1\"", ["hello"]).output().expect("sh");
    assert_eq!(output.text().trim(), "hello");
}

#[test]
fn bash_inline_executes_when_bash_exists() {
    if which("bash").is_none() && !std::path::Path::new("/bin/bash").exists() {
        return;
    }

    let output = bash::inline("echo hi", []).output().expect("bash");
    assert_eq!(output.text().trim(), "hi");
}

#[test]
fn sh_inline_executes_when_sh_exists() {
    if which("sh").is_none() && !std::path::Path::new("/bin/sh").exists() {
        return;
    }

    let output = sh::inline("echo hi", []).output().expect("sh");
    assert_eq!(output.text().trim(), "hi");
}

#[test]
fn deno_file_allows_everything_and_deno_inline_evals() {
    assert_eq!(
        args(&deno::file("app.ts", ["--x"])),
        ["run", "-A", "app.ts", "--x"]
    );
    assert_eq!(
        args(&deno::inline("console.log(1)", [])),
        ["eval", "console.log(1)"]
    );
    assert_eq!(args(&deno::script("app.ts", [])), ["run", "-A", "app.ts"]);
}

// The assertion assumes a Unix program name.
#[cfg(unix)]
#[test]
fn python_passes_arguments_after_dash_c() {
    assert_eq!(
        args(&python::inline("print(1)", ["a"])),
        ["-c", "print(1)", "a"]
    );
    assert_eq!(args(&python::script("app.py", [])), ["app.py"]);
    assert!(
        python::file("app.py", [])
            .program()
            .to_string_lossy()
            .contains("python")
    );
}

#[test]
fn ruby_passes_arguments_after_dash_e() {
    assert_eq!(args(&ruby::inline("puts 1", ["a"])), ["-e", "puts 1", "a"]);
    assert_eq!(args(&ruby::file("a.rb", [])), ["a.rb"]);
}

#[test]
fn perl_separates_arguments_with_a_double_dash() {
    assert_eq!(
        args(&perl::inline("print 1", ["-n"])),
        ["-e", "print 1", "--", "-n"]
    );
    assert_eq!(args(&perl::inline("print 1", [])), ["-e", "print 1", "--"]);
    assert_eq!(args(&perl::script("a.pl", [])), ["a.pl"]);
}

#[test]
fn uv_runs_files_and_writes_inline_source() {
    assert_eq!(args(&uv::file("app.py", ["a"])), ["run", "app.py", "a"]);

    let command = uv::inline("print(1)\n", []).expect("inline uv");
    let arguments = args(&command);
    assert_eq!(arguments[0], "run");
    assert!(command.temp_file().is_some());
}

#[test]
fn php_runs_files_and_writes_inline_source() {
    assert_eq!(args(&php::file("a.php", ["1"])), ["a.php", "1"]);

    let command = php::inline("<?php echo 1;", []).expect("inline php");
    let temp = command.temp_file().expect("temp file");
    assert_eq!(args(&command), [temp.to_string_lossy().into_owned()]);
}

#[test]
fn php_inline_adds_the_opening_tag_once() {
    let command = php::inline("echo 1;", []).expect("inline php");
    let temp = command.temp_file().expect("temp file");
    let stored = std::fs::read_to_string(temp).expect("read the temp file");
    assert!(stored.starts_with("<?php\n"));
    assert!(stored.contains("echo 1;"));
    assert_eq!(stored.matches("<?php").count(), 1);

    let command = php::inline(" \n<?php echo 1;", []).expect("inline php");
    let temp = command.temp_file().expect("temp file");
    let stored = std::fs::read_to_string(temp).expect("read the temp file");
    assert!(stored.starts_with("<?php"));
    assert_eq!(stored.matches("<?php").count(), 1);

    let command = php::inline("<?php echo 1;", []).expect("inline php");
    let temp = command.temp_file().expect("temp file");
    let stored = std::fs::read_to_string(temp).expect("read the temp file");
    assert_eq!(stored, "<?php echo 1;");

    let trailing = " \n<?php echo 1; ?>  ";
    let command = php::inline(trailing, []).expect("inline php");
    let temp = command.temp_file().expect("temp file");
    let stored = std::fs::read_to_string(temp).expect("read the temp file");
    assert_eq!(stored, "<?php echo 1; ?>  ");
}

#[test]
fn fish_runs_files() {
    assert_eq!(args(&fish::file("a.fish", ["1"])), ["a.fish", "1"]);
    assert!(fish::inline("echo 1", []).is_ok());
}

#[test]
fn nushell_runs_files_through_nu() {
    let command = nushell::file("a.nu", ["1"]);

    assert_eq!(args(&command), ["a.nu", "1"]);
    assert!(command.program().to_string_lossy().contains("nu"));
    assert!(nushell::script("print 1", []).is_ok());
}

#[test]
fn zsh_puts_the_script_after_dash_c_and_a_placeholder() {
    let command = zsh::inline("echo $1", ["hi"]);
    let arguments = args(&command);

    assert_eq!(
        &arguments[arguments.len() - 4..],
        ["-c", "echo $1", "zsh", "hi"]
    );

    let file = zsh::file("a.zsh", ["1"]);
    assert_eq!(
        args(&file),
        ["--no-rcs", "-e", "-o", "pipefail", "a.zsh", "1"]
    );
}

#[test]
fn lua_passes_arguments_after_dash_e() {
    assert_eq!(
        args(&lua::inline("print(1)", ["a"])),
        ["-e", "print(1)", "a"]
    );
    assert_eq!(args(&lua::file("a.lua", [])), ["a.lua"]);
}

#[test]
fn r_runs_through_rscript_and_writes_inline_source() {
    let command = r::file("a.R", ["1"]);
    assert_eq!(args(&command), ["a.R", "1"]);
    assert!(command.program().to_string_lossy().contains("Rscript"));

    let command = r::inline("print(1)\n", []).expect("inline R");
    assert!(command.temp_file().is_some());
    assert!(command.program().to_string_lossy().contains("Rscript"));
}

#[test]
fn julia_separates_arguments_with_a_double_dash() {
    assert_eq!(
        args(&julia::inline("println(1)", ["--help"])),
        ["-e", "println(1)", "--", "--help"]
    );
    assert_eq!(args(&julia::script("a.jl", [])), ["a.jl"]);
}

#[test]
fn elixir_separates_arguments_with_a_double_dash() {
    assert_eq!(
        args(&elixir::inline("IO.puts(1)", [])),
        ["-e", "IO.puts(1)", "--"]
    );
}

#[test]
fn awk_uses_f_for_a_file_and_a_positional_program() {
    assert_eq!(
        args(&awk::file("a.awk", ["in.txt"])),
        ["-f", "a.awk", "in.txt"]
    );
    assert_eq!(
        args(&awk::inline("BEGIN { }", ["in.txt"])),
        ["BEGIN { }", "in.txt"]
    );
}

#[test]
fn dart_runs_files_and_writes_inline_source() {
    assert_eq!(args(&dart::file("a.dart", ["1"])), ["run", "a.dart", "1"]);

    let command = dart::inline("void main() {}\n", []).expect("inline Dart");
    let arguments = args(&command);
    assert_eq!(arguments[0], "run");
    assert!(command.temp_file().is_some());
}

#[test]
fn tcl_runs_through_tclsh() {
    let command = tcl::file("a.tcl", ["1"]);

    assert_eq!(args(&command), ["a.tcl", "1"]);
    assert!(command.program().to_string_lossy().contains("tclsh"));
    assert!(tcl::inline("puts 1", []).is_ok());
}

#[test]
fn groovy_separates_arguments_with_a_double_dash() {
    assert_eq!(
        args(&groovy::inline("println 1", ["-h"])),
        ["-e", "println 1", "--", "-h"]
    );
}

#[test]
fn racket_separates_arguments_with_a_double_dash() {
    assert_eq!(
        args(&racket::inline("(+ 1 1)", ["--help"])),
        ["-e", "(+ 1 1)", "--", "--help"]
    );
}

#[test]
fn bash_inline_with_shell_args_drops_the_default_prefix() {
    let command = bash::inline_with_shell_args("echo hi", ["-e"], ["x"]);
    let arguments = args(&command);

    assert_eq!(arguments, ["-e", "-c", "echo hi", "bash", "x"]);
    assert!(!arguments.iter().any(|argument| argument == "--noprofile"));
}

// The assertion assumes a Unix program name.
#[cfg(unix)]
#[test]
fn bash_file_with_shell_args_appends_the_path_and_arguments() {
    let command = bash::file_with_shell_args("a.sh", [], ["x"]);

    assert_eq!(args(&command), ["a.sh", "x"]);
}

#[test]
fn deno_file_with_shell_args_replaces_run_and_allow() {
    let command = deno::file_with_shell_args("app.ts", ["run"], ["--x"]);
    let arguments = args(&command);

    assert_eq!(arguments, ["run", "app.ts", "--x"]);
    assert!(!arguments.iter().any(|argument| argument == "-A"));
}
