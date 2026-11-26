# vrun

vrun runs a command, opens a path or URL, or detaches a process. `NAME=value` prefixes, dotenv files, and `#!` scripts work the same way on Linux, macOS, and Windows.

The library crate is `vaelrun/` and the CLI package is `vaelrun-cli`.

Documentation is at https://vaelrun.dev.

```sh
vrun MY_VAR=test ./path/to/.env dotnet build
```

```sh
vrun .
```

```sh
vrun -e ./path/to/file.cs
```

A `.vrun` file is one command graph per line:

```
dotnet build && dotnet test
```

## Install

`cargo build --release -p vaelrun-cli` produces `vrun`. The package is `vaelrun-cli` because `vrun` is taken on crates.io.

A `v*` tag builds the six platform binaries and uploads them as workflow artifacts. They are not signed and are not attached to a GitHub Release yet.

Path hints and script operators are implemented in this crate. It does not vendor Neotales.Exec or neotales-process.

## Config

vrun reads `/etc/vaelrun/config.ini`, then `C:\ProgramData\vaelrun\etc\config.ini` on Windows, then `~/.config/vaelrun/config.ini`.

`$VAELRUN_CONFIG` is loaded last when it is set. A missing file is ignored. A file that does not parse is an error.

## Trust

Trust is on by default. A program may run when it is a full path, a relative path that contains a separator, a name from a `[bin]` directory, or a registered tool hint that exists on disk. Anything else a bare name finds on `PATH` is refused.

## Scripts

A `.vrun` file is not a shell. `#` comments are skipped, and a first-line `#!` is ignored. `{{ .NAME }}` and `{{ env "NAME" }}` expand from the child environment before the line is parsed. `|` binds tighter than `&&`, `||`, and `;`. A line that ends non-zero stops the file unless `||` on that line handled the failure.

## Exit codes

`0` means success, including a detach that spawned. A command vrun waits for returns the child status. `2` is usage. `126` is a trust refusal or a file that cannot be executed. `127` means the program was not found. `1` is a config, template, or IO error.
