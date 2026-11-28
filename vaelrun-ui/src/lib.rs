//! State the task window binds. The Rune view is in the binary; these tests do not open one.

#![deny(unsafe_code)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use vaelrun_core::{
    Environment, EnvironmentInput, Error, Move, Scope, Task, TaskStore, add_icon,
    build_environment, filter_tasks, launch_request, load_config, move_env_file, move_env_var,
};

fn failure(message: &str) -> Error {
    Error {
        code: 1,
        message: message.to_string(),
    }
}

/// Lines captured from a task run. A VT widget can replace the scroll that draws these.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskOutput {
    lines: Vec<String>,
    partial: String,
}

impl TaskOutput {
    /// An empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Completed lines, in the order they were captured.
    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Drops every line and any unfinished tail.
    pub fn clear(&mut self) {
        self.lines.clear();
        self.partial.clear();
    }

    /// Appends one finished line.
    pub fn push_line(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }

    /// Appends a chunk that may end mid-line. A newline finishes a line; the rest waits.
    pub fn append_chunk(&mut self, chunk: &str) {
        let mut rest = chunk;
        while let Some(index) = rest.find('\n') {
            let mut line = std::mem::take(&mut self.partial);
            line.push_str(&rest[..index]);
            if line.ends_with('\r') {
                line.pop();
            }
            self.lines.push(line);
            rest = &rest[index + 1..];
        }
        self.partial.push_str(rest);
    }

    /// Stores an unfinished tail as its own line.
    pub fn finish(&mut self) {
        if !self.partial.is_empty() {
            self.lines.push(std::mem::take(&mut self.partial));
        }
    }
}

/// The working directory, filter, and resolved tasks the window shows.
pub struct WindowState {
    cwd: String,
    filter: String,
    store: TaskStore,
    tasks: Vec<Task>,
    selected: Option<usize>,
    output: TaskOutput,
    status: String,
    icon_dir: PathBuf,
}

impl WindowState {
    /// Loads global tasks. `icon_dir` is where [`WindowState::add_icon_file`] copies files.
    ///
    /// # Errors
    ///
    /// Returns an IO or parse error from the global task file.
    pub fn open(mut store: TaskStore, icon_dir: PathBuf) -> Result<Self, Error> {
        let tasks = store.load_global()?;
        Ok(Self {
            cwd: String::new(),
            filter: String::new(),
            store,
            tasks,
            selected: None,
            output: TaskOutput::new(),
            status: String::new(),
            icon_dir,
        })
    }

    /// The working directory the list was resolved for. Empty until [`WindowState::set_cwd`].
    #[must_use]
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    /// Resolves tasks for `cwd`. A folder task of the same name replaces the global one.
    ///
    /// # Errors
    ///
    /// Returns an IO or parse error from either task file. The global file is not written.
    pub fn set_cwd(&mut self, cwd: impl Into<String>) -> Result<(), Error> {
        let previous = self.selected().map(|task| task.name.clone());
        self.cwd = cwd.into();
        self.reload()?;
        self.selected =
            previous.and_then(|name| self.tasks.iter().position(|task| task.name == name));
        Ok(())
    }

    /// The current filter. Empty matches every task.
    #[must_use]
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Sets the filter. Matching is case-insensitive on name and description.
    pub fn set_filter(&mut self, query: impl Into<String>) {
        self.filter = query.into();
    }

    /// Resolved tasks, before the filter.
    #[must_use]
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// Resolved tasks that match the filter, in input order.
    #[must_use]
    pub fn visible(&self) -> Vec<&Task> {
        filter_tasks(&self.tasks, &self.filter)
    }

    /// Selects a visible task by its index in [`WindowState::visible`].
    pub fn select(&mut self, visible_index: usize) {
        if let Some(task) = self.visible().get(visible_index) {
            let name = task.name.clone();
            self.select_name(&name);
        }
    }

    /// Selects the resolved task named `name`, if it is in the list.
    pub fn select_name(&mut self, name: &str) {
        self.selected = self.tasks.iter().position(|task| task.name == name);
    }

    /// The selected task, if one is selected.
    #[must_use]
    pub fn selected(&self) -> Option<&Task> {
        self.selected.and_then(|index| self.tasks.get(index))
    }

    /// The line log from the last run.
    #[must_use]
    pub fn output(&self) -> &TaskOutput {
        &self.output
    }

    /// A short status from the last run or icon add. Empty when nothing has failed.
    #[must_use]
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Replaces the status line the window shows under the log.
    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status = status.into();
    }

    /// Moves an env file of the selected task and saves that task's own file.
    ///
    /// # Errors
    ///
    /// Returns an error when nothing is selected, or the task file cannot be written.
    pub fn move_file(&mut self, index: usize, direction: Move) -> Result<(), Error> {
        self.reorder(index, direction, true)
    }

    /// Moves an env variable of the selected task and saves that task's own file.
    ///
    /// # Errors
    ///
    /// Returns an error when nothing is selected, or the task file cannot be written.
    pub fn move_var(&mut self, index: usize, direction: Move) -> Result<(), Error> {
        self.reorder(index, direction, false)
    }

    /// Copies `source` into the icon directory and stores the id on the selected task.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is empty, nothing is selected, or the copy fails.
    pub fn add_icon_file(&mut self, source: &Path) -> Result<String, Error> {
        let Some(name) = source.file_name().and_then(|name| name.to_str()) else {
            return Err(failure("icon name is empty"));
        };
        let id = add_icon(&self.icon_dir, name, source)?;
        let Some(index) = self.selected else {
            return Err(failure("no task selected"));
        };
        self.tasks[index].icon = Some(id.clone());
        self.persist(index)?;
        Ok(id)
    }

    /// The child environment the selected task would receive. Does not start a process.
    ///
    /// # Errors
    ///
    /// Returns an error when nothing is selected, or an env file cannot be read.
    pub fn child_env(&self) -> Result<Environment, Error> {
        let task = self.selected().ok_or_else(|| failure("no task selected"))?;
        let cwd = self.cwd_path();
        environment(task, &cwd, None)
    }

    /// Builds launch args and starts the selected command in the working directory.
    ///
    /// The environment comes from [`build_environment`]. Stdout and stderr are appended
    /// to [`WindowState::output`]. This does not pass `--no-trust` and does not spawn `vrun`.
    ///
    /// # Errors
    ///
    /// Returns an error when nothing is selected, the working directory is missing, or the
    /// command cannot start.
    pub fn run_selected(&mut self) -> Result<i32, Error> {
        let task = self
            .selected()
            .cloned()
            .ok_or_else(|| failure("no task selected"))?;
        let cwd = self.cwd_path();
        if !cwd.is_dir() {
            return Err(failure("working directory is not a directory"));
        }
        let request = launch_request(&self.store, &task);
        let skip = task.env_files.len() + task.env.len();
        let command: Vec<OsString> = request.args.into_iter().skip(skip).collect();
        let Some(program) = command.first() else {
            return Err(failure("task command is empty"));
        };
        let env = environment(&task, &cwd, request.injector.as_deref())?;
        let output = Command::new(program)
            .args(&command[1..])
            .current_dir(&cwd)
            .env_clear()
            .envs(env.pairs().iter().cloned())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|_| failure("cannot start command"))?;
        self.output.clear();
        self.output
            .append_chunk(&String::from_utf8_lossy(&output.stdout));
        self.output
            .append_chunk(&String::from_utf8_lossy(&output.stderr));
        self.output.finish();
        self.status.clear();
        Ok(output.status.code().unwrap_or(1))
    }

    fn reload(&mut self) -> Result<(), Error> {
        if self.cwd.is_empty() {
            self.tasks = self.store.load_global()?;
            return Ok(());
        }
        self.tasks = self.store.resolve(Path::new(&self.cwd))?;
        Ok(())
    }

    fn cwd_path(&self) -> PathBuf {
        PathBuf::from(&self.cwd)
    }

    fn reorder(&mut self, index: usize, direction: Move, file: bool) -> Result<(), Error> {
        let Some(selected) = self.selected else {
            return Err(failure("no task selected"));
        };
        if file {
            move_env_file(&mut self.tasks[selected], index, direction);
        } else {
            move_env_var(&mut self.tasks[selected], index, direction);
        }
        self.persist(selected)
    }

    fn persist(&mut self, index: usize) -> Result<(), Error> {
        let task = self.tasks[index].clone();
        match &task.scope {
            Scope::Global => {
                let mut tasks = self.store.load_global()?;
                upsert(&mut tasks, &task);
                self.store.save_global(&tasks)?;
            }
            Scope::Folder(folder) => {
                let folder = folder.clone();
                let mut tasks = self.store.load_folder(&folder)?;
                upsert(&mut tasks, &task);
                self.store.save_folder(&folder, &tasks)?;
            }
        }
        Ok(())
    }
}

fn upsert(tasks: &mut Vec<Task>, task: &Task) {
    if let Some(existing) = tasks.iter_mut().find(|item| item.name == task.name) {
        *existing = task.clone();
    } else {
        tasks.push(task.clone());
    }
}

fn environment(task: &Task, cwd: &Path, injector: Option<&[String]>) -> Result<Environment, Error> {
    let config = load_config(&[])?;
    let files = resolved_files(task, cwd);
    let inline: Vec<(String, OsString)> = task
        .env
        .iter()
        .map(|(key, value)| (key.clone(), OsString::from(value)))
        .collect();
    let parent: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    build_environment(&EnvironmentInput {
        parent: &parent,
        dotenv: &files,
        unset: &[],
        inline: &inline,
        cwd,
        injector,
        config: &config,
        trust: None,
    })
}

fn resolved_files(task: &Task, cwd: &Path) -> Vec<PathBuf> {
    task.env_files
        .iter()
        .map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                cwd.join(path)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn task(name: &str, command: &str) -> Task {
        Task::new(name, command).unwrap()
    }

    #[test]
    fn set_cwd_filter_and_folder_replace() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("proj");
        fs::create_dir_all(&folder).unwrap();
        let store = TaskStore::at(dir.path().join("tasks.json"));
        store
            .save_global(&[
                task("build", "cargo build").with_description("from global"),
                task("test", "cargo test"),
            ])
            .unwrap();
        store
            .save_folder(
                &folder,
                &[task("build", "cargo build --release").with_description("from folder")],
            )
            .unwrap();
        let mut state = WindowState::open(store, dir.path().join("icons")).unwrap();
        assert!(state.cwd().is_empty());
        state.set_cwd(folder.to_str().unwrap()).unwrap();
        assert_eq!(state.cwd(), folder.to_str().unwrap());
        assert_eq!(state.tasks()[0].command, "cargo build --release");
        assert_eq!(state.tasks()[1].name, "test");

        state.set_filter("FOLD");
        assert_eq!(state.filter(), "FOLD");
        let visible = state.visible();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "build");
        state.set_filter("");
        assert_eq!(state.visible().len(), 2);
        assert_eq!(state.visible()[0].name, "build");
    }

    #[test]
    fn reorder_env_and_variable_overrides_files() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("proj");
        fs::create_dir_all(&folder).unwrap();
        let early = folder.join("early.env");
        let late = folder.join("late.env");
        fs::write(&early, "COLOR='red'\nSHARED='from-file'\n").unwrap();
        fs::write(&late, "COLOR='blue'\n").unwrap();
        let store = TaskStore::at(dir.path().join("tasks.json"));
        store
            .save_folder(
                &folder,
                &[task("paint", "true")
                    .with_env_file(&early)
                    .with_env_file(&late)
                    .with_env("COLOR", "green")
                    .unwrap()
                    .with_env("MODE", "1")
                    .unwrap()],
            )
            .unwrap();
        let mut state = WindowState::open(store, dir.path().join("icons")).unwrap();
        state.set_cwd(folder.to_str().unwrap()).unwrap();
        state.select(0);
        let env = state.child_env().unwrap();
        assert_eq!(env.get("COLOR"), Some("green"));
        assert_eq!(env.get("SHARED"), Some("from-file"));

        state.move_file(0, Move::Down).unwrap();
        assert_eq!(state.selected().unwrap().env_files[0], late);
        state.move_var(1, Move::Up).unwrap();
        assert_eq!(state.selected().unwrap().env[0].0, "MODE");
        state.move_file(0, Move::Up).unwrap();
        assert_eq!(state.selected().unwrap().env_files[0], late);
    }

    #[test]
    fn output_keeps_a_partial_line() {
        let mut output = TaskOutput::new();
        output.append_chunk("one\ntwo");
        assert_eq!(output.lines(), ["one".to_string()]);
        output.finish();
        assert_eq!(output.lines(), ["one".to_string(), "two".to_string()]);
    }
}
