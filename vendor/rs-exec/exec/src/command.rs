//! A single process: owned, chainable, and started without a shell.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::Arc;
use std::time::SystemTime;

use crate::error::Error;
use crate::log::{CommandView, Hook, global_hook};
use crate::output::{Output, Stage};
use crate::pipeline::Pipeline;

/// A program and the arguments, environment, and working directory it will run with.
///
/// Every builder takes `self` and returns `Self`, so a command is a value that moves through the
/// chain. Starting it costs one spawn; nothing runs until [`Command::output`], [`Command::run`] or
/// [`Command::quiet`] is called.
///
/// The program is not resolved here. The operating system resolves it when the process starts, so
/// [`Command::program`] reports exactly what the caller set and a logging hook sees the same name.
pub struct Command {
    program: OsString,
    arguments: Vec<OsString>,
    environment: Vec<(OsString, OsString)>,
    working_directory: Option<PathBuf>,
    hook: Option<Hook>,
    logging_disabled: bool,
    temporary_file: Option<PathBuf>,
}

impl std::fmt::Debug for Command {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Command")
            .field("program", &self.program)
            .field("arguments", &self.arguments)
            .field("working_directory", &self.working_directory)
            .field("logging_disabled", &self.logging_disabled)
            .field("temporary_file", &self.temporary_file)
            .finish_non_exhaustive()
    }
}

impl Command {
    /// Creates a command for `program`.
    #[must_use]
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            program: program.as_ref().to_os_string(),
            arguments: Vec::new(),
            environment: Vec::new(),
            working_directory: None,
            hook: None,
            logging_disabled: false,
            temporary_file: None,
        }
    }

    /// Splits `command` into a program and arguments, without a shell.
    ///
    /// Quoting is handled by [`neotales_args::split()`]; `$HOME`, `$(...)`, `|` and `>` stay ordinary
    /// characters and are never expanded or interpreted.
    ///
    /// # Errors
    ///
    /// Returns [`Error::EmptyCommand`] when `command` contains no program name.
    pub fn parse(command: &str) -> Result<Self, Error> {
        let mut tokens = neotales_args::split(command).into_iter();
        let program = tokens.next().ok_or(Error::EmptyCommand)?;
        Ok(Self::new(program).args(tokens))
    }

    /// Appends one argument.
    #[must_use]
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.arguments.push(arg.as_ref().to_os_string());
        self
    }

    /// Appends several arguments, in order.
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.arguments
            .extend(args.into_iter().map(|arg| arg.as_ref().to_os_string()));
        self
    }

    /// Sets an environment variable for this process only.
    #[must_use]
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.environment
            .push((key.as_ref().to_os_string(), value.as_ref().to_os_string()));
        self
    }

    /// Sets the working directory for this process only.
    #[must_use]
    pub fn current_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.working_directory = Some(dir.as_ref().to_path_buf());
        self
    }

    /// Adds a hook that runs for this command before it starts.
    ///
    /// The command hook runs before the package hook installed by [`crate::set_logger`], and both
    /// may run. Neither is needed for the process to start. The hook receives a snapshot.
    #[must_use]
    pub fn logger(mut self, hook: impl Fn(&CommandView) + Send + Sync + 'static) -> Self {
        self.hook = Some(Arc::new(hook));
        self
    }

    /// Suppresses this command's hook and the package hook.
    #[must_use]
    pub fn disable_logger(mut self) -> Self {
        self.logging_disabled = true;
        self
    }

    /// The program, as the caller set it.
    #[must_use]
    pub fn program(&self) -> &OsStr {
        &self.program
    }

    /// The arguments, in order.
    ///
    /// Named `arguments` because [`Command::args`] is the chainable builder.
    #[must_use]
    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    /// The temporary file this command was built from, when a script runtime wrote one.
    #[must_use]
    pub fn temp_file(&self) -> Option<&Path> {
        self.temporary_file.as_deref()
    }

    /// Records the temporary file this command was built from.
    ///
    /// A script runtime that materialises source on disk sets this so the caller can name it. The
    /// crate never deletes the file.
    #[must_use]
    pub fn with_temp_file(mut self, path: PathBuf) -> Self {
        self.temporary_file = Some(path);
        self
    }

    /// Captures stdout and stderr and inherits nothing.
    ///
    /// Standard input is empty, so a program that reads it sees end of file immediately.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Spawn`] when the program cannot start, [`Error::Wait`] when its status
    /// cannot be collected, and [`Error::Terminated`] when a signal killed it.
    pub fn output(self) -> Result<Output, Error> {
        Pipeline::new(vec![self]).output()
    }

    /// Inherits stdout, stderr and stdin, and still reports the exit code.
    ///
    /// The captured buffers in the returned [`Output`] are empty, because the bytes went to the
    /// terminal rather than to this crate.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Spawn`] when the program cannot start, [`Error::Wait`] when its status
    /// cannot be collected, and [`Error::Terminated`] when a signal killed it.
    pub fn run(self) -> Result<Output, Error> {
        Pipeline::new(vec![self]).run()
    }

    /// Discards stdin, stdout and stderr, and still reports the exit code.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Spawn`] when the program cannot start, [`Error::Wait`] when its status
    /// cannot be collected, and [`Error::Terminated`] when a signal killed it.
    pub fn quiet(self) -> Result<Output, Error> {
        let started_at = SystemTime::now();
        self.fire_hooks();

        let mut command = self.process_command();
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let mut child = command.spawn().map_err(|source| Error::Spawn {
            program: self.program.clone(),
            source,
        })?;
        let status = child.wait().map_err(|source| Error::Wait {
            program: self.program.clone(),
            source,
        })?;
        let code = status.code().ok_or_else(|| Error::Terminated {
            program: self.program.clone(),
        })?;

        let stage = Stage {
            program: self.program.clone(),
            args: self.arguments.clone(),
            code,
            stdout: Vec::new(),
            stderr: Vec::new(),
        };

        Ok(Output {
            stdout: Vec::new(),
            stderr: Vec::new(),
            code,
            program: self.program.clone(),
            args: self.arguments.clone(),
            started_at,
            ended_at: SystemTime::now(),
            stages: vec![stage],
        })
    }

    /// Starts a pipeline with this command as the first stage.
    #[must_use]
    pub fn pipe(self, next: Self) -> Pipeline {
        Pipeline::new(vec![self, next])
    }

    /// Parses `command` and appends it as the second stage of a pipeline.
    ///
    /// # Errors
    ///
    /// Returns [`Error::EmptyCommand`] when `command` contains no program name.
    pub fn pipe_parse(self, command: &str) -> Result<Pipeline, Error> {
        let next = Self::parse(command)?;
        Ok(self.pipe(next))
    }

    pub(crate) fn process_command(&self) -> ProcessCommand {
        let mut command = ProcessCommand::new(&self.program);
        command.args(&self.arguments);
        for (key, value) in &self.environment {
            command.env(key, value);
        }
        if let Some(directory) = &self.working_directory {
            command.current_dir(directory);
        }
        command
    }

    pub(crate) fn fire_hooks(&self) {
        if self.logging_disabled {
            return;
        }

        let view = CommandView {
            program: self.program.clone(),
            args: self.arguments.clone(),
            current_dir: self.working_directory.clone(),
        };

        if let Some(hook) = &self.hook {
            hook(&view);
        }
        if let Some(hook) = global_hook() {
            hook(&view);
        }
    }
}
