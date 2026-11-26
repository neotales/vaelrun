//! Cross-platform run, open, and start.
//!
//! [`load_config`], [`build_environment`], [`launch`], and [`open`] are the
//! library entry points. The `vrun` binary parses argv and calls those functions.

#![deny(unsafe_code)]

mod api;
mod cli;
mod complete;
mod config;
mod dotenv;
mod error;
mod resolve;
mod run;
mod script;
mod shebang;
mod template;

pub use api::{
    CommandMode, Config, Environment, EnvironmentInput, Launch, build_environment, launch,
    load_config, open,
};
pub use error::Error;

use std::ffi::OsString;
use std::path::PathBuf;

/// Inputs for [`run_with`]. The process environment is used when a field is unset.
#[derive(Debug, Default)]
pub struct RunOptions {
    /// Replaces `xdg-open`, `open`, and `explorer`. The target is one argument.
    ///
    /// Test seam so an open can be asserted without a file manager.
    pub opener: Option<OsString>,
    /// Arguments including argv0. `None` reads the process arguments.
    pub args: Option<Vec<OsString>>,
    /// Config file loaded last, in place of `$VAELRUN_CONFIG`.
    pub config: Option<PathBuf>,
}

/// Run vrun with the process arguments.
pub fn run() -> Result<i32, Error> {
    run_with(RunOptions::default())
}

/// Run vrun with explicit arguments and an optional opener seam.
pub fn run_with(opts: RunOptions) -> Result<i32, Error> {
    run::run(opts)
}
