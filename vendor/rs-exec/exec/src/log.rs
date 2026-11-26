//! The logging hook that fires once per process, immediately before it starts.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// A snapshot of a command, handed to a logging hook.
///
/// The view deliberately carries no environment: a hook is a log point, and environment values are
/// the usual place a secret sits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandView {
    /// The program as the caller set it, before any `PATH` resolution.
    pub program: OsString,
    /// The arguments, in order, with the environment left out.
    pub args: Vec<OsString>,
    /// The working directory, when the caller set one.
    pub current_dir: Option<PathBuf>,
}

/// The hook a command or the package holds.
pub(crate) type Hook = Arc<dyn Fn(&CommandView) + Send + Sync>;

static LOGGER: Mutex<Option<Hook>> = Mutex::new(None);

fn logger() -> MutexGuard<'static, Option<Hook>> {
    LOGGER.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Installs the package logger, replacing the previous one.
///
/// The hook runs for every process started through this crate, before it starts, unless that
/// command disabled logging. It is safe to call from any thread; a concurrent spawn either sees the
/// old hook or the new one.
pub fn set_logger(hook: impl Fn(&CommandView) + Send + Sync + 'static) {
    *logger() = Some(Arc::new(hook));
}

/// Removes the package logger.
///
/// A command that set its own hook still runs that hook.
pub fn clear_logger() {
    *logger() = None;
}

pub(crate) fn global_hook() -> Option<Hook> {
    logger().clone()
}
