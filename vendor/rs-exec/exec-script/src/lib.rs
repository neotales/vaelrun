//! Build [`neotales_exec::Command`] values for script runtimes.
//!
//! Every module has the same four constructors: [`bash::new`] for a bare runtime with arguments,
//! [`bash::file`] for a script path, [`bash::inline`] for source text, and [`bash::script`] for a
//! string that may be either. Each returns an owned command, so the caller sets the working
//! directory, the environment and a logger on it. A runtime that cannot forward arguments through
//! its inline flag writes the source to a private file first, so its `inline` and `script` return
//! `io::Result<Command>`; every other constructor returns a `Command`.
//!
//! [`bash`], [`sh`], [`zsh`], [`powershell`], [`pwsh`], [`cmd`], and [`deno`] also have
//! `file_with_shell_args` and `inline_with_shell_args`, which replace the default flag prefix and
//! keep the rest of the argument shape.
//!
//! [`golang`], [`dotnet`], [`uv`], [`php`], [`fish`], [`nushell`], [`dart`], [`r`], and [`tcl`]
//! write inline source to a private file, and every other module passes it directly.
//!
//! The runtime is resolved through [`neotales_exec::find`] on first use. When it is not installed,
//! the command carries the bare name (`bash`, `node`, `go`) and the spawn reports the missing
//! program.
#![doc = include_str!("../README.md")]

pub mod awk;
pub mod bash;
pub mod bun;
pub mod cmd;
mod common;
pub mod dart;
pub mod deno;
pub mod dotnet;
pub mod elixir;
pub mod fish;
pub mod golang;
pub mod groovy;
pub mod julia;
pub mod lua;
pub mod node;
pub mod nushell;
pub mod perl;
pub mod php;
pub mod powershell;
pub mod pwsh;
pub mod python;
pub mod r;
pub mod racket;
pub mod ruby;
pub mod sh;
pub mod tcl;
pub mod uv;
pub mod zsh;
