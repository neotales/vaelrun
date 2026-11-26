//! Pipelines: several processes with each stage's stdout wired to the next stage's stdin.

use std::io::Read;
use std::process::{Child, ChildStdout, ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::SystemTime;

use crate::command::Command;
use crate::error::Error;
use crate::output::{Output, Stage};

/// Several commands run at once, each stage's stdout handed to the next stage's stdin.
///
/// Every stage is started before the first one is waited for, so a full pipe buffer cannot
/// deadlock the chain. A three-stage pipeline is the same shape as a two-stage one; nothing
/// special-cases the first and last stage beyond where stdin and stdout go.
#[derive(Debug)]
pub struct Pipeline {
    stages: Vec<Command>,
}

impl Pipeline {
    pub(crate) fn new(stages: Vec<Command>) -> Self {
        Self { stages }
    }

    /// Appends a stage whose stdin is the previous stage's stdout.
    #[must_use]
    pub fn pipe(mut self, next: Command) -> Self {
        self.stages.push(next);
        self
    }

    /// Parses `command` and appends it as the last stage.
    ///
    /// # Errors
    ///
    /// Returns [`Error::EmptyCommand`] when `command` contains no program name.
    pub fn pipe_parse(mut self, command: &str) -> Result<Self, Error> {
        self.stages.push(Command::parse(command)?);
        Ok(self)
    }

    /// Runs every stage, capturing each stage's stderr and the last stage's stdout.
    ///
    /// The returned [`Output`] describes the last stage. Earlier stages are in
    /// [`Output::stages`], with their captured stderr and the exit code of each. A non-zero exit is
    /// not an error until the caller asks for one with [`Output::to_error`].
    ///
    /// An earlier stage that ends on a signal is recorded on that stage, as `128` plus the signal
    /// number. Only the last stage decides whether the pipeline failed, so `yes | head -n 1`
    /// returns the reader's result even though the writer was killed by `SIGPIPE`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Spawn`] when a stage cannot start; stages already started are killed and
    /// waited for first. Returns [`Error::Wait`] when a status cannot be collected, and
    /// [`Error::Terminated`] only when the last stage was killed by a signal.
    pub fn output(self) -> Result<Output, Error> {
        execute(&self.stages, Mode::Capture)
    }

    /// Runs every stage with the last stage's stdout and stderr and the first stage's stdin
    /// inherited.
    ///
    /// Earlier stages' stderr is still captured. The returned [`Output`] carries the exit code and
    /// the per-stage records, and empty captured buffers for the last stage.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Spawn`] when a stage cannot start; stages already started are killed and
    /// waited for first. Returns [`Error::Wait`] when a status cannot be collected, and
    /// [`Error::Terminated`] only when the last stage was killed by a signal.
    pub fn run(self) -> Result<Output, Error> {
        execute(&self.stages, Mode::Inherit)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Capture,
    Inherit,
}

fn execute(stages: &[Command], mode: Mode) -> Result<Output, Error> {
    let started_at = SystemTime::now();
    let last_index = stages.len().saturating_sub(1);

    let mut children: Vec<Child> = Vec::with_capacity(stages.len());
    let mut stderr_readers: Vec<JoinHandle<Vec<u8>>> = Vec::with_capacity(stages.len());
    let mut stdout_reader: Option<JoinHandle<Vec<u8>>> = None;
    let mut stdin_of_next: Option<ChildStdout> = None;

    for (index, stage) in stages.iter().enumerate() {
        stage.fire_hooks();
        let is_last = index == last_index;

        let mut command = stage.process_command();
        command.stdin(match stdin_of_next.take() {
            Some(stdout) => Stdio::from(stdout),
            None if mode == Mode::Capture => Stdio::null(),
            None => Stdio::inherit(),
        });
        command.stdout(if !is_last || mode == Mode::Capture {
            Stdio::piped()
        } else {
            Stdio::inherit()
        });
        command.stderr(if is_last && mode == Mode::Inherit {
            Stdio::inherit()
        } else {
            Stdio::piped()
        });

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(source) => {
                stop(&mut children, &mut stderr_readers, &mut stdout_reader);
                return Err(Error::Spawn {
                    program: stage.program().to_os_string(),
                    source,
                });
            }
        };

        if let Some(stderr) = child.stderr.take() {
            stderr_readers.push(read_in_background(stderr));
        }

        if is_last {
            if mode == Mode::Capture
                && let Some(stdout) = child.stdout.take()
            {
                stdout_reader = Some(read_in_background(stdout));
            }
        } else {
            stdin_of_next = child.stdout.take();
        }

        children.push(child);
    }

    let (codes, wait_error) = wait(&mut children, stages);
    let mut stage_stderr: Vec<Vec<u8>> = stderr_readers.into_iter().map(finish).collect();
    let last_stdout = stdout_reader.map_or_default(finish);

    if let Some(error) = wait_error {
        return Err(error);
    }

    let mut stage_outputs = Vec::with_capacity(stages.len());
    for (index, stage) in stages.iter().enumerate() {
        stage_outputs.push(Stage {
            program: stage.program().to_os_string(),
            args: stage.arguments().to_vec(),
            code: codes.get(index).copied().unwrap_or_default(),
            stdout: if index == last_index {
                last_stdout.clone()
            } else {
                Vec::new()
            },
            stderr: stage_stderr.get_mut(index).map_or_default(std::mem::take),
        });
    }

    let last = &stage_outputs[last_index];
    Ok(Output {
        stdout: last.stdout.clone(),
        stderr: last.stderr.clone(),
        code: last.code,
        program: last.program.clone(),
        args: last.args.clone(),
        started_at,
        ended_at: SystemTime::now(),
        stages: stage_outputs,
    })
}

fn wait(children: &mut [Child], stages: &[Command]) -> (Vec<i32>, Option<Error>) {
    let last_index = stages.len().saturating_sub(1);
    let mut codes = Vec::with_capacity(children.len());
    let mut error = None;

    for (index, (child, stage)) in children.iter_mut().zip(stages).enumerate() {
        match child.wait() {
            Ok(status) => {
                let terminated = status.code().is_none();
                codes.push(exit_code(status));
                if index == last_index && terminated && error.is_none() {
                    error = Some(Error::Terminated {
                        program: stage.program().to_os_string(),
                    });
                }
            }
            Err(source) => {
                codes.push(0);
                if error.is_none() {
                    error = Some(Error::Wait {
                        program: stage.program().to_os_string(),
                        source,
                    });
                }
            }
        }
    }

    (codes, error)
}

/// `128` plus the signal number for a signalled process, matching a shell.
fn exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }

    0
}

fn stop(
    children: &mut [Child],
    stderr_readers: &mut Vec<JoinHandle<Vec<u8>>>,
    stdout_reader: &mut Option<JoinHandle<Vec<u8>>>,
) {
    for child in children.iter_mut() {
        let _ = child.kill();
    }
    for child in children.iter_mut() {
        let _ = child.wait();
    }
    for reader in stderr_readers.drain(..) {
        let _ = reader.join();
    }
    if let Some(reader) = stdout_reader.take() {
        let _ = reader.join();
    }
}

fn read_in_background<R: Read + Send + 'static>(mut reader: R) -> JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = reader.read_to_end(&mut buffer);
        buffer
    })
}

fn finish(reader: JoinHandle<Vec<u8>>) -> Vec<u8> {
    reader.join().unwrap_or_default()
}
