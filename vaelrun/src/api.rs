//! Library entry points. The `vrun` binary calls these instead of growing its own copy.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::config;
use crate::dotenv::EnvMap;
use crate::error::Error;
use crate::run::{self, Mode};

/// INI loaded from explicit paths. Later files override the same keys the CLI overrides.
#[derive(Clone, Debug)]
pub struct Config {
    inner: config::Config,
}

impl Config {
    /// Load `paths` in order.
    ///
    /// A missing file is ignored. A file that does not parse is an error.
    ///
    /// # Errors
    ///
    /// Returns an IO or parse error. The message names the file.
    pub fn load(paths: &[PathBuf]) -> Result<Self, Error> {
        Ok(Self {
            inner: config::load_config(paths)?,
        })
    }

    /// Whether trust refuses a PATH-only program. Default is on.
    #[must_use]
    pub fn trust_on(&self) -> bool {
        self.inner.trust_on()
    }

    /// Whether `http://` and `https://` targets may be opened. Default is on.
    #[must_use]
    pub fn urls_enabled(&self) -> bool {
        self.inner.urls_enabled()
    }

    /// Injector argv from `[secrets]`, if one is configured and non-empty.
    #[must_use]
    pub fn injector(&self) -> Option<&[String]> {
        self.inner.injector_argv()
    }

    pub(crate) fn inner(&self) -> &config::Config {
        &self.inner
    }
}

/// The environment a child process receives.
#[derive(Clone, Debug)]
pub struct Environment {
    inner: EnvMap,
}

impl Environment {
    /// The value of `key`, if it is UTF-8. On Windows the name match ignores ASCII case.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.inner.get(key)
    }

    /// Every pair passed to the child, in insertion order.
    #[must_use]
    pub fn pairs(&self) -> &[(OsString, OsString)] {
        &self.inner.pairs
    }

    pub(crate) fn inner(&self) -> &EnvMap {
        &self.inner
    }
}

/// Inputs for [`build_environment`], applied in CLI order.
///
/// Parent values are copied as the caller left them. Dotenv values, including
/// single-quoted ones, stay literal. `-u` removes keys. Inline assignments are
/// expanded after an optional injector returns.
pub struct EnvironmentInput<'a> {
    /// Process environment, or a stand-in for it.
    pub parent: &'a [(OsString, OsString)],
    /// Dotenv files, in order.
    pub dotenv: &'a [PathBuf],
    /// Names removed after dotenv files are applied.
    pub unset: &'a [String],
    /// `NAME=value` assignments. Only these values are interpolated.
    pub inline: &'a [(String, OsString)],
    /// Working directory for the injector, if one runs.
    pub cwd: &'a Path,
    /// Injector program and arguments. `None` runs no injector.
    pub injector: Option<&'a [String]>,
    /// Config used to resolve the injector and to read trust.
    pub config: &'a Config,
    /// Overrides `[trust]`. `None` uses [`Config::trust_on`].
    pub trust: Option<bool>,
}

/// Build the child environment.
///
/// # Errors
///
/// Returns an IO, parse, trust, or injector error. A dotenv value is not expanded.
///
/// # Examples
///
/// ```
/// use std::ffi::OsString;
/// use vaelrun::{EnvironmentInput, build_environment, load_config};
///
/// let config = load_config(&[]).expect("empty path list");
/// let dir = std::env::temp_dir().join(format!("vaelrun-doc-{}", std::process::id()));
/// std::fs::create_dir_all(&dir).unwrap();
/// let dotenv = dir.join("sample.env");
/// std::fs::write(&dotenv, "SECRET='keep-$HOME'\n").unwrap();
/// let parent = vec![(OsString::from("HOME"), OsString::from("parent-home"))];
/// let inline = vec![("NOTE".to_string(), OsString::from("hi-$HOME"))];
/// let env = build_environment(&EnvironmentInput {
///     parent: &parent,
///     dotenv: std::slice::from_ref(&dotenv),
///     unset: &[],
///     inline: &inline,
///     cwd: &dir,
///     injector: None,
///     config: &config,
///     trust: None,
/// })
/// .expect("environment");
/// assert_eq!(env.get("SECRET"), Some("keep-$HOME"));
/// assert_eq!(env.get("NOTE"), Some("hi-parent-home"));
/// let _ = std::fs::remove_dir_all(&dir);
/// ```
pub fn build_environment(input: &EnvironmentInput<'_>) -> Result<Environment, Error> {
    let trust_on = input.trust.unwrap_or_else(|| input.config.trust_on());
    let inner = run::build_child_env(&run::ChildEnvRequest {
        parent: input.parent,
        dotenv: input.dotenv,
        unset: input.unset,
        inline: input.inline,
        cwd: input.cwd,
        config: input.config.inner(),
        trust_on,
        injector: input.injector,
    })?;
    Ok(Environment { inner })
}

/// How [`launch`] treats a file that is not a program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandMode {
    /// Run scripts and programs. Open a document.
    Implicit,
    /// Refuse a file that cannot be executed.
    Run,
    /// Like [`CommandMode::Implicit`], detached, and a macOS `.app` uses `open -n`.
    Start,
}

/// A command or an open, already split. CLI flags are not parsed.
pub struct Launch<'a> {
    /// Program and arguments.
    pub argv: &'a [OsString],
    /// Working directory.
    pub cwd: &'a Path,
    /// Child environment from [`build_environment`].
    pub env: &'a Environment,
    /// Config used for trust, extensions, and scripts.
    pub config: &'a Config,
    /// Overrides `[trust]`.
    pub trust: Option<bool>,
    /// Prefix `sudo` after the program is resolved.
    pub sudo: bool,
    /// Replaces the platform opener. The target is one argument.
    pub opener: Option<&'a OsStr>,
    /// Process argv, used when a detached `.vrun` file re-execs. Empty is fine for a library call.
    pub args: &'a [OsString],
    /// How a non-program file is handled.
    pub mode: CommandMode,
    /// Do not wait for the child.
    pub detach: bool,
}

/// Run `request.argv` with shebang, extension, and trust rules.
///
/// # Errors
///
/// Returns the same errors the CLI returns for a command: usage, trust, not found, or IO.
pub fn launch(request: &Launch<'_>) -> Result<i32, Error> {
    let trust_on = request.trust.unwrap_or_else(|| request.config.trust_on());
    run::launch_command(&run::CommandRequest {
        cwd: request.cwd,
        env: request.env.inner(),
        config: request.config.inner(),
        trust_on,
        sudo: request.sudo,
        opener: request.opener,
        args: request.args,
        command: request.argv,
        mode: mode_of(request.mode),
        detach: request.detach,
    })
}

/// Open `target` with the platform opener, or `request.opener` when set.
///
/// # Errors
///
/// Returns an error when URL opening is disabled, or when the opener cannot start.
pub fn open(target: &OsStr, request: &Launch<'_>) -> Result<i32, Error> {
    let trust_on = request.trust.unwrap_or_else(|| request.config.trust_on());
    run::open_command(
        request.cwd,
        request.env.inner(),
        request.config.inner(),
        trust_on,
        request.opener,
        target,
        request.detach,
    )
}

/// Load config files. See [`Config::load`].
///
/// # Errors
///
/// Returns an IO or parse error.
pub fn load_config(paths: &[PathBuf]) -> Result<Config, Error> {
    Config::load(paths)
}

fn mode_of(mode: CommandMode) -> Mode {
    match mode {
        CommandMode::Implicit => Mode::Implicit,
        CommandMode::Run => Mode::Run,
        CommandMode::Start => Mode::Start,
    }
}
