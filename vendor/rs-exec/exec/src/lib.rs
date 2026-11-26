//! Run external commands and pipelines without a shell, and find the executables to run.
//!
//! [`Command`] is owned and chainable: each builder returns the command, and nothing starts until
//! [`Command::output`], [`Command::run`] or [`Command::quiet`] is called. [`Pipeline`] wires each
//! stage's stdout to the next stage's stdin and starts every stage before waiting for any of them,
//! so a full pipe buffer cannot deadlock the chain.
//!
//! Quoting is the only parsing this crate does, through [`neotales_args::split()`]; there is no shell
//! behind [`Command::parse`], so `$HOME`, `$(...)` and `|` are ordinary text.
//!
//! [`find`] and [`which`] locate an executable for a caller that needs the path, and
//! [`set_logger`] installs a hook that fires once per process, immediately before it starts.
#![doc = include_str!("../README.md")]

mod command;
mod error;
mod executables;
mod finder;
mod log;
mod output;
mod pipeline;

pub use command::Command;
pub use error::Error;
pub use executables::{Executable, find, register};
pub use finder::{WhichOptions, which, which_first};
pub use log::{CommandView, clear_logger, set_logger};
pub use output::{Output, Stage};
pub use pipeline::Pipeline;
