# neotales-exec

Runs external programs and pipelines with owned, chainable commands and no shell.

By [x0craft](https://github.com/x0craft).

[![Neotales](https://github.com/neotales/brand/raw/main/assets/logo.png)](https://github.com/neotales/rs-exec)

<!-- badges: crates.io, docs.rs, CI. Add once the first version exists. -->

## Install 📦

```sh
cargo add neotales-exec
```

## What you can do 🚀

**Build a command, run it, read the result.** `output()` captures stdout and stderr and leaves
stdin empty. A non-zero exit is data, not an error. `is_ok` and `to_error` decide what it means.

```rust,no_run
use neotales_exec::Command;

let out = Command::new("echo").arg("hello").output().unwrap();
assert!(out.is_ok());
assert_eq!(out.text().trim(), "hello");
```

**Parse command text without a shell.** `parse` splits quoting through `neotales_args`. It never
expands `$HOME`, never runs `$(...)`, and needs no `sh -c`.

```rust,no_run
use neotales_exec::Command;

let out = Command::parse("echo 'hello world'").unwrap().output().unwrap();
assert_eq!(out.lines(), ["hello world"]);
```

**Pipe stages.** Every stage starts before the first wait, so a full pipe buffer cannot deadlock.
The result describes the last stage. A signal on an earlier stage is recorded on that stage as
`128` plus the signal number and does not fail the pipeline. The earlier stages stay in `stages`,
each with its own stderr.

```rust,no_run
use neotales_exec::Command;

let out = Command::new("echo")
    .arg("Hello World")
    .pipe(Command::new("grep").arg("Hello"))
    .pipe_parse("cat")
    .unwrap()
    .output()
    .unwrap();

assert_eq!(out.text(), "Hello World\n");
assert_eq!(out.stages.len(), 3);
```

**Choose what the process sees.** `run()` inherits stdin, stdout, and stderr, `quiet()` discards
all three, and `output()` captures. Each still reports the exit code.

```rust,no_run
use neotales_exec::Command;

let status = Command::new("make").arg("test").run().unwrap();
status.to_error().unwrap();
```

**Log every spawn.** The hook fires once per process, immediately before it starts, and gets a
snapshot with no environment in it. A command's own hook runs before the package hook.

```rust
use neotales_exec::{Command, CommandView, set_logger};

set_logger(|view: &CommandView| {
    eprintln!("run {}", view.program.to_string_lossy());
});

let command = Command::new("echo")
    .arg("hello")
    .logger(|view: &CommandView| eprintln!("about to run {:?}", view.args));

// Nothing runs until output(), run() or quiet() is called, and this one logs nothing.
let _command = command.disable_logger();
```

**Resolve a tool by name.** `find` checks the registered environment variable, then an explicit
path, then the per-platform candidates. `which` searches `PATH` and requires the executable bit on
Unix.

```rust
use neotales_exec::{Executable, find, register, which};

register(
    "mytool",
    Executable {
        name: "mytool".to_owned(),
        variable: "MYTOOL_EXE".to_owned(),
        linux: vec!["/usr/local/bin/mytool".to_owned()],
        ..Executable::default()
    },
);

// Err when nothing matches, and the caller decides what to fall back to.
let _ = find("mytool", None);
let _ = which("sh");
```

## Licence

[MIT](../LICENSE.md)
