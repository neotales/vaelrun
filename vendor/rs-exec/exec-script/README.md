# neotales-exec-script

Builds `neotales-exec` commands for script runtimes.

By [x0craft](https://github.com/x0craft).

[![Neotales](https://github.com/neotales/brand/raw/main/assets/logo.png)](https://github.com/neotales/rs-exec)

<!-- badges: crates.io, docs.rs, CI. Add once the first version exists. -->

## Install 📦

```sh
cargo add neotales-exec-script
```

## What you can do 🧰

**Get the arguments the runtime expects.** Each module knows how its runtime wants a file or a
piece of source, so the caller never spells out `-c`, `--file`, or `run`. `bash::inline` and
`sh::inline` pass a `$0` placeholder before your arguments, so the first one reaches the script as
`$1`.

```rust
use neotales_exec_script::{bash, node};

let command = bash::inline("echo hi", []);
assert_eq!(command.arguments()[4], "-c");

let command = node::file("app.js", ["--watch"]);
assert_eq!(command.arguments(), ["app.js", "--watch"]);
```

**Give it a string and let it choose.** `script` treats a single line that ends in a known extension
as a file and runs anything else inline. A line with a `\n` in it is always source.

```rust
use neotales_exec_script::{bash, cmd, golang};

assert_eq!(bash::script("build.sh", []).arguments()[4], "build.sh");
assert_eq!(bash::script("echo hi", []).arguments()[5], "echo hi");
assert_eq!(cmd::script("setup.bat", []).arguments()[2], "setup.bat");
assert_eq!(golang::script("main.go", []).unwrap().arguments()[1], "main.go");
```

**Run it in the terminal, or capture it.** `run` inherits stdin, stdout, and stderr; `output` is the
constructor that captures.

```rust,no_run
use neotales_exec_script::bash;

bash::inline("echo hi", [])
    .current_dir(".")
    .env("MODE", "test")
    .run()
    .unwrap();
```

**Replace the default shell flags.** `inline_with_shell_args` and `file_with_shell_args` swap the
prefix that `bash`, `sh`, `zsh`, `powershell`, `pwsh`, `cmd`, and `deno` add by default. The rest of
the argument shape stays.

```rust
use neotales_exec_script::bash;

let command = bash::inline_with_shell_args("echo hi", ["-e"], []);
assert!(!command.arguments().iter().any(|argument| argument == "--noprofile"));
assert!(command.arguments().iter().any(|argument| argument == "-e"));
```

**Chain into another command.** A command from this crate is a `neotales_exec::Command`, so it pipes
without leaving the type.

```rust,no_run
use neotales_exec::Command;
use neotales_exec_script::bash;

let output = bash::inline("echo Hello World", [])
    .pipe(Command::new("grep").arg("Hello"))
    .output()
    .unwrap();
assert_eq!(output.text(), "Hello World\n");
```

**Pass source on the runtime's own flag.** Python, Ruby, Deno, Lua, Julia, Elixir, Groovy, Racket,
and zsh take it there, and Perl keeps a `--` so a leading dash reaches `@ARGV` instead of perl
itself.

```rust
use neotales_exec_script::{perl, python};

let command = python::inline("print(1)", ["a"]);
assert_eq!(command.arguments(), ["-c", "print(1)", "a"]);

let command = perl::inline("print 1", ["-n"]);
assert_eq!(command.arguments(), ["-e", "print 1", "--", "-n"]);
```

**Run inline source through a private file.** `golang`, `dotnet`, `uv`, `php`, `fish`, `nushell`,
`dart`, `r`, and `tcl` cannot take the source on a flag without losing the caller's arguments, so
they write a fresh private file in the user's directory and run that. The command records the path,
the file stays there, and every other module writes nothing.

On Windows, privacy relies on the per-user `LocalAppData` directory and reparse-point refusal, not a custom ACL.

```rust
use neotales_exec_script::{dotnet, golang};

let go = golang::inline("package main\nfunc main() {}\n", []).unwrap();
assert!(go.temp_file().is_some());

let cs = dotnet::inline("System.Console.WriteLine(1);", []).unwrap();
assert!(cs.temp_file().is_some());
```

**Runtime modules.** `bash`, `sh`, `zsh`, `fish`, `node`, `bun`, `deno`, `golang`, `dotnet`,
`powershell`, `pwsh`, `cmd`, `python`, `ruby`, `perl`, `lua`, `uv`, `php`, `julia`, `elixir`,
`groovy`, `racket`, `dart`, `r`, `tcl`, `awk`, and `nushell`. Each one resolves its executable
through `neotales_exec::find`, so an environment variable such as `RUN_NODE_EXE` overrides the
search, and a missing runtime falls back to the bare program name. Three modules run an executable
with a different name: `nushell` runs `nu`, `r` runs `Rscript`, and `tcl` runs `tclsh`.

## Licence

[MIT](../LICENSE.md)
