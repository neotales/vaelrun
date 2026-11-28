//! Named tasks stored globally and per folder.
//!
//! A folder task with the same name replaces the global one in [`TaskStore::resolve`].
//! That merge does not write the global file. [`launch_args`] is the argv prefix `vrun`
//! would take: env files, then `KEY=value` assignments, then the command.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use crate::error::Error;

/// Where a loaded task came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// `~/.config/vaelrun/tasks.json`, or the explicit path a test passed.
    Global,
    /// `<folder>/.vaelrun/tasks.json`.
    Folder(PathBuf),
}

/// Which way [`move_env_file`] and [`move_env_var`] shift an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    /// Toward index 0. A move at the start does nothing.
    Up,
    /// Toward the last index. A move at the end does nothing.
    Down,
}

/// A named command, plus the env files and assignments that precede it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    /// Non-empty. Compared exactly when a folder task replaces a global one.
    pub name: String,
    /// Shown beside the name. Absent when the file omitted it.
    pub description: Option<String>,
    /// Program and arguments, split on whitespace by [`launch_args`].
    pub command: String,
    /// Dotenv files, in the order `vrun` would apply them.
    pub env_files: Vec<PathBuf>,
    /// `KEY=value` assignments, applied after the files so they override.
    pub env: Vec<(String, String)>,
    /// Id returned by [`add_icon`]. The file lives in the icon directory.
    pub icon: Option<String>,
    /// Display shortcut. Not interpreted here.
    pub shortcut: Option<String>,
    /// Injector argv for this task. Overrides the store default when set.
    pub injector: Option<Vec<String>>,
    /// Set when the task is loaded. Not written back; the file path implies it.
    pub scope: Scope,
}

impl Task {
    /// A task with no env, icon, or shortcut. Scope starts as [`Scope::Global`].
    ///
    /// # Errors
    ///
    /// Returns an error when `name` or `command` is empty or only whitespace.
    pub fn new(name: impl Into<String>, command: impl Into<String>) -> Result<Self, Error> {
        let task = Self {
            name: name.into(),
            description: None,
            command: command.into(),
            env_files: Vec::new(),
            env: Vec::new(),
            icon: None,
            shortcut: None,
            injector: None,
            scope: Scope::Global,
        };
        validate_task(&task)?;
        Ok(task)
    }

    /// Sets the optional description. An empty string clears it.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = optional_text(description.into());
        self
    }

    /// Appends a dotenv file. Later files override earlier ones.
    #[must_use]
    pub fn with_env_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.env_files.push(path.into());
        self
    }

    /// Appends an assignment. It is applied after every env file.
    ///
    /// # Errors
    ///
    /// Returns an error when `key` is empty or contains `=`.
    pub fn with_env(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, Error> {
        let key = key.into();
        validate_env_key(&key)?;
        self.env.push((key, value.into()));
        Ok(self)
    }

    /// Stores an icon id. An empty id is rejected.
    ///
    /// # Errors
    ///
    /// Returns an error when `id` is empty.
    pub fn with_icon(mut self, id: impl Into<String>) -> Result<Self, Error> {
        let id = id.into();
        if id.is_empty() {
            return Err(Error::new(1, "task icon id is empty"));
        }
        self.icon = Some(id);
        Ok(self)
    }

    /// Sets the display shortcut. An empty string clears it.
    #[must_use]
    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = optional_text(shortcut.into());
        self
    }

    /// Injector program and arguments for this task. An empty list clears it.
    #[must_use]
    pub fn with_injector(mut self, argv: Vec<String>) -> Self {
        self.injector = optional_argv(argv);
        self
    }
}

/// Global and folder task files.
///
/// Tests pass [`TaskStore::at`]. Production uses [`TaskStore::global`], which is
/// `~/.config/vaelrun/tasks.json` when a home directory is known.
#[derive(Clone, Debug)]
pub struct TaskStore {
    global_path: PathBuf,
    injector: Option<Vec<String>>,
}

impl TaskStore {
    /// A store whose global file is `path`. The file need not exist yet.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            global_path: path.into(),
            injector: None,
        }
    }

    /// The store at `~/.config/vaelrun/tasks.json`.
    ///
    /// # Errors
    ///
    /// Returns an error when neither `HOME` nor, on Windows, `USERPROFILE` is set.
    pub fn global() -> Result<Self, Error> {
        let path = global_tasks_path().ok_or_else(|| Error::new(1, "home directory is not set"))?;
        Ok(Self::at(path))
    }

    /// The global file this store reads and writes.
    #[must_use]
    pub fn global_path(&self) -> &Path {
        &self.global_path
    }

    /// Default injector argv, used when a task does not name one.
    #[must_use]
    pub fn injector(&self) -> Option<&[String]> {
        self.injector.as_deref()
    }

    /// Sets the default injector. An empty list clears it. [`TaskStore::save_global`] writes it.
    pub fn set_injector(&mut self, argv: Option<Vec<String>>) {
        self.injector = argv.and_then(optional_argv);
    }

    /// Loads the global file. A missing file is an empty list and clears the store injector.
    ///
    /// # Errors
    ///
    /// Returns an IO or parse error. The message names the file, not secret values.
    pub fn load_global(&mut self) -> Result<Vec<Task>, Error> {
        let loaded = read_file(&self.global_path, Scope::Global)?;
        self.injector = loaded.injector;
        Ok(loaded.tasks)
    }

    /// Writes global tasks and the store injector. Does not read the folder file.
    ///
    /// # Errors
    ///
    /// Returns an error when a task is invalid, or the file cannot be written.
    pub fn save_global(&self, tasks: &[Task]) -> Result<(), Error> {
        write_file(&self.global_path, tasks, self.injector.as_deref())
    }

    /// Loads `<folder>/.vaelrun/tasks.json`. A missing file is an empty list.
    ///
    /// # Errors
    ///
    /// Returns an IO or parse error. The message names the file.
    pub fn load_folder(&self, folder: &Path) -> Result<Vec<Task>, Error> {
        let loaded = read_file(
            &folder_tasks_path(folder),
            Scope::Folder(folder.to_path_buf()),
        )?;
        Ok(loaded.tasks)
    }

    /// Writes folder tasks. Does not touch the global file.
    ///
    /// # Errors
    ///
    /// Returns an error when a task is invalid, or the file cannot be written.
    pub fn save_folder(&self, folder: &Path, tasks: &[Task]) -> Result<(), Error> {
        write_file(&folder_tasks_path(folder), tasks, None)
    }

    /// Global tasks, with a folder task of the same name replacing the global one.
    ///
    /// A folder task whose name is not global is appended, in file order. The global
    /// file is not created or modified.
    ///
    /// # Errors
    ///
    /// Returns an IO or parse error from either file.
    pub fn resolve(&self, folder: &Path) -> Result<Vec<Task>, Error> {
        let global = read_file(&self.global_path, Scope::Global)?;
        let folder_tasks = read_file(
            &folder_tasks_path(folder),
            Scope::Folder(folder.to_path_buf()),
        )?;
        Ok(merge_tasks(global.tasks, folder_tasks.tasks))
    }
}

/// Argv `vrun` would take for `task`, and the injector that request would name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchRequest {
    /// Env files, then `KEY=value`, then the command split on whitespace.
    pub args: Vec<std::ffi::OsString>,
    /// The task injector, or the store default when the task does not set one.
    pub injector: Option<Vec<String>>,
}

/// The argv prefix `vrun` would use for `task`.
///
/// Env files come first, then `KEY=value` assignments, then `command` split on
/// whitespace. Assignments follow files so they override. Quote parsing is not done.
#[must_use]
pub fn launch_args(task: &Task) -> Vec<std::ffi::OsString> {
    let mut args = Vec::with_capacity(task.env_files.len() + task.env.len() + 1);
    for path in &task.env_files {
        args.push(path.as_os_str().to_os_string());
    }
    for (key, value) in &task.env {
        args.push(std::ffi::OsString::from(format!("{key}={value}")));
    }
    for token in task.command.split_whitespace() {
        args.push(std::ffi::OsString::from(token));
    }
    args
}

/// [`launch_args`] plus the injector the request would name.
///
/// The task injector wins. Otherwise the store default is used. This does not run it.
#[must_use]
pub fn launch_request(store: &TaskStore, task: &Task) -> LaunchRequest {
    LaunchRequest {
        args: launch_args(task),
        injector: task
            .injector
            .clone()
            .or_else(|| store.injector().map(<[String]>::to_vec)),
    }
}

/// `~/.config/vaelrun/tasks.json` when a home directory is known.
#[must_use]
pub fn global_tasks_path() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".config").join("vaelrun").join("tasks.json"))
}

/// `~/.config/vaelrun/icons` when a home directory is known.
///
/// [`add_icon`] does not assume this path. The caller passes the directory.
#[must_use]
pub fn icons_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".config").join("vaelrun").join("icons"))
}

/// `<folder>/.vaelrun/tasks.json`.
#[must_use]
pub fn folder_tasks_path(folder: &Path) -> PathBuf {
    folder.join(".vaelrun").join("tasks.json")
}

/// Copies `source` into `dir` under `name` and returns that id.
///
/// Production passes [`icons_dir`]. The task stores the returned id; this function
/// does not edit the task.
///
/// # Errors
///
/// Returns an error when `name` is empty, is not a single file name, or the copy fails.
pub fn add_icon(dir: &Path, name: &str, source: &Path) -> Result<String, Error> {
    let id = name.trim();
    if id.is_empty() {
        return Err(Error::new(1, "icon name is empty"));
    }
    if id.contains(['/', '\\']) || id == "." || id == ".." || id.contains('\0') {
        return Err(Error::new(1, "icon name is not a file name"));
    }
    fs::create_dir_all(dir).map_err(|err| Error::io(dir, &err))?;
    let dest = dir.join(id);
    fs::copy(source, &dest).map_err(|err| Error::io(&dest, &err))?;
    Ok(id.to_string())
}

/// Tasks whose name or description contains `query`, case-insensitive.
///
/// An empty or whitespace-only query returns `tasks` in input order.
#[must_use]
pub fn filter_tasks<'a>(tasks: &'a [Task], query: &str) -> Vec<&'a Task> {
    let query = query.trim();
    if query.is_empty() {
        return tasks.iter().collect();
    }
    let needle = query.to_ascii_lowercase();
    tasks
        .iter()
        .filter(|task| {
            contains_ignore_ascii(&task.name, &needle)
                || task
                    .description
                    .as_deref()
                    .is_some_and(|text| contains_ignore_ascii(text, &needle))
        })
        .collect()
}

/// Moves an env file one step. Stops at either end, and ignores an index past the end.
pub fn move_env_file(task: &mut Task, index: usize, direction: Move) {
    move_entry(&mut task.env_files, index, direction);
}

/// Moves an env variable one step. Stops at either end, and ignores an index past the end.
pub fn move_env_var(task: &mut Task, index: usize, direction: Move) {
    move_entry(&mut task.env, index, direction);
}

fn move_entry<T>(items: &mut [T], index: usize, direction: Move) {
    let target = match direction {
        Move::Up if index > 0 && index < items.len() => index - 1,
        Move::Down if index + 1 < items.len() => index + 1,
        _ => return,
    };
    items.swap(index, target);
}

fn contains_ignore_ascii(haystack: &str, needle_lower: &str) -> bool {
    haystack.to_ascii_lowercase().contains(needle_lower)
}

fn optional_text(text: String) -> Option<String> {
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn optional_argv(argv: Vec<String>) -> Option<Vec<String>> {
    if argv.is_empty() { None } else { Some(argv) }
}

fn validate_task(task: &Task) -> Result<(), Error> {
    if task.name.trim().is_empty() {
        return Err(Error::new(1, "task name is empty"));
    }
    if task.command.trim().is_empty() {
        return Err(Error::new(1, "task command is empty"));
    }
    if task.icon.as_deref().is_some_and(str::is_empty) {
        return Err(Error::new(1, "task icon id is empty"));
    }
    for (key, _) in &task.env {
        validate_env_key(key)?;
    }
    Ok(())
}

fn validate_env_key(key: &str) -> Result<(), Error> {
    if key.is_empty() || key.contains('=') || key.contains('\0') {
        return Err(Error::new(1, "env variable name is empty or contains '='"));
    }
    Ok(())
}

fn home_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|value| !value.is_empty());
    let home = home.or_else(|| {
        if cfg!(windows) {
            std::env::var_os("USERPROFILE").filter(|value| !value.is_empty())
        } else {
            None
        }
    });
    home.map(PathBuf::from)
}

fn merge_tasks(global: Vec<Task>, folder: Vec<Task>) -> Vec<Task> {
    let mut replaced = std::collections::BTreeSet::new();
    let mut out = Vec::with_capacity(global.len() + folder.len());
    for task in global {
        if let Some(over) = folder.iter().rev().find(|item| item.name == task.name) {
            if replaced.insert(over.name.clone()) {
                out.push(over.clone());
            }
        } else {
            out.push(task);
        }
    }
    for task in folder {
        if replaced.insert(task.name.clone()) {
            out.push(task);
        }
    }
    out
}

struct Loaded {
    tasks: Vec<Task>,
    injector: Option<Vec<String>>,
}

fn read_file(path: &Path, scope: Scope) -> Result<Loaded, Error> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok(Loaded {
                tasks: Vec::new(),
                injector: None,
            });
        }
        Err(err) => return Err(Error::io(path, &err)),
    };
    parse_document(&text, scope)
        .map_err(|reason| Error::new(1, format!("{}: {reason}", path.display())))
}

fn write_file(path: &Path, tasks: &[Task], injector: Option<&[String]>) -> Result<(), Error> {
    for task in tasks {
        validate_task(task)?;
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| Error::io(parent, &err))?;
    }
    let body = render_document(tasks, injector)?;
    let mut file = fs::File::create(path).map_err(|err| Error::io(path, &err))?;
    file.write_all(body.as_bytes())
        .map_err(|err| Error::io(path, &err))?;
    Ok(())
}

fn render_document(tasks: &[Task], injector: Option<&[String]>) -> Result<String, Error> {
    let mut out = String::from("{\n");
    if let Some(argv) = injector {
        out.push_str("  \"injector\": ");
        push_string_array(&mut out, argv);
        out.push_str(",\n");
    }
    out.push_str("  \"tasks\": [\n");
    for (index, task) in tasks.iter().enumerate() {
        render_task(&mut out, task)?;
        if index + 1 != tasks.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]\n}\n");
    Ok(out)
}

fn render_task(out: &mut String, task: &Task) -> Result<(), Error> {
    out.push_str("    {\n");
    push_field(out, "name", &task.name, true);
    if let Some(description) = &task.description {
        push_field(out, "description", description, true);
    }
    push_field(out, "command", &task.command, true);
    out.push_str("      \"env_files\": ");
    push_path_array(out, &task.env_files)?;
    out.push_str(",\n      \"env\": ");
    push_pairs(out, &task.env)?;
    if let Some(icon) = &task.icon {
        out.push_str(",\n");
        push_field(out, "icon", icon, false);
    }
    if let Some(shortcut) = &task.shortcut {
        out.push_str(",\n");
        push_field(out, "shortcut", shortcut, false);
    }
    if let Some(argv) = &task.injector {
        out.push_str(",\n      \"injector\": ");
        push_string_array(out, argv);
    }
    out.push_str("\n    }");
    Ok(())
}

fn push_field(out: &mut String, key: &str, value: &str, trailing_comma: bool) {
    out.push_str("      \"");
    out.push_str(key);
    out.push_str("\": ");
    push_string(out, value);
    if trailing_comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_string_array(out: &mut String, values: &[String]) {
    out.push('[');
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        push_string(out, value);
    }
    out.push(']');
}

fn push_path_array(out: &mut String, paths: &[PathBuf]) -> Result<(), Error> {
    out.push('[');
    for (index, path) in paths.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        let text = path
            .to_str()
            .ok_or_else(|| Error::new(1, "env file path is not utf-8"))?;
        push_string(out, text);
    }
    out.push(']');
    Ok(())
}

fn push_pairs(out: &mut String, pairs: &[(String, String)]) -> Result<(), Error> {
    out.push('[');
    for (index, (key, value)) in pairs.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push('[');
        push_string(out, key);
        out.push_str(", ");
        push_string(out, value);
        out.push(']');
    }
    out.push(']');
    Ok(())
}

fn push_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                out.push_str(&format!("\\u{:04x}", u32::from(c)));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn parse_document(text: &str, scope: Scope) -> Result<Loaded, String> {
    let mut parser = Parser { text, index: 0 };
    parser.skip_ws();
    let object = parser.parse_object()?;
    parser.skip_ws();
    if parser.peek().is_some() {
        return Err("trailing data after the task file".to_string());
    }
    let mut injector = None;
    let mut tasks = None;
    for (key, value) in object {
        match key.as_str() {
            "injector" => injector = Some(expect_strings(value, "injector")?),
            "tasks" => tasks = Some(expect_tasks(value, &scope)?),
            _ => {}
        }
    }
    Ok(Loaded {
        tasks: tasks.unwrap_or_default(),
        injector: injector.and_then(optional_argv),
    })
}

fn expect_tasks(value: Value, scope: &Scope) -> Result<Vec<Task>, String> {
    let Value::Array(items) = value else {
        return Err("tasks must be an array".to_string());
    };
    let mut tasks = Vec::with_capacity(items.len());
    for item in items {
        tasks.push(expect_task(item, scope)?);
    }
    Ok(tasks)
}

fn expect_task(value: Value, scope: &Scope) -> Result<Task, String> {
    let Value::Object(fields) = value else {
        return Err("a task must be an object".to_string());
    };
    let mut name = None;
    let mut description = None;
    let mut command = None;
    let mut env_files = Vec::new();
    let mut env = Vec::new();
    let mut icon = None;
    let mut shortcut = None;
    let mut injector = None;
    for (key, value) in fields {
        match key.as_str() {
            "name" => name = Some(expect_string(value, "name")?),
            "description" => description = optional_text(expect_string(value, "description")?),
            "command" => command = Some(expect_string(value, "command")?),
            "env_files" => env_files = expect_paths(value)?,
            "env" => env = expect_pairs(value)?,
            "icon" => icon = Some(expect_string(value, "icon")?),
            "shortcut" => shortcut = optional_text(expect_string(value, "shortcut")?),
            "injector" => injector = Some(expect_strings(value, "injector")?),
            _ => {}
        }
    }
    let name = name.ok_or_else(|| "task name is missing".to_string())?;
    let command = command.ok_or_else(|| "task command is missing".to_string())?;
    if name.trim().is_empty() {
        return Err("task name is empty".to_string());
    }
    if command.trim().is_empty() {
        return Err("task command is empty".to_string());
    }
    if icon.as_deref().is_some_and(str::is_empty) {
        return Err("task icon id is empty".to_string());
    }
    for (key, _) in &env {
        if key.is_empty() || key.contains('=') {
            return Err("env variable name is empty or contains '='".to_string());
        }
    }
    Ok(Task {
        name,
        description,
        command,
        env_files,
        env,
        icon,
        shortcut,
        injector: injector.and_then(optional_argv),
        scope: scope.clone(),
    })
}

fn expect_string(value: Value, field: &str) -> Result<String, String> {
    match value {
        Value::String(text) => Ok(text),
        Value::Null => Err(format!("{field} is null")),
        _ => Err(format!("{field} must be a string")),
    }
}

fn expect_strings(value: Value, field: &str) -> Result<Vec<String>, String> {
    let Value::Array(items) = value else {
        return Err(format!("{field} must be an array"));
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        out.push(expect_string(item, field)?);
    }
    Ok(out)
}

fn expect_paths(value: Value) -> Result<Vec<PathBuf>, String> {
    Ok(expect_strings(value, "env_files")?
        .into_iter()
        .map(PathBuf::from)
        .collect())
}

fn expect_pairs(value: Value) -> Result<Vec<(String, String)>, String> {
    let Value::Array(items) = value else {
        return Err("env must be an array".to_string());
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let Value::Array(pair) = item else {
            return Err("env entry must be a pair".to_string());
        };
        if pair.len() != 2 {
            return Err("env entry must be a pair".to_string());
        }
        let mut pair = pair.into_iter();
        let key = expect_string(pair.next().unwrap_or(Value::Null), "env")?;
        let value = expect_string(pair.next().unwrap_or(Value::Null), "env")?;
        out.push((key, value));
    }
    Ok(out)
}

enum Value {
    Null,
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

struct Parser<'a> {
    text: &'a str,
    index: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while self.peek().is_some_and(|ch| ch.is_ascii_whitespace()) {
            self.bump();
        }
    }

    fn peek(&self) -> Option<char> {
        self.text[self.index..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.index += ch.len_utf8();
        Some(ch)
    }

    fn parse_value(&mut self) -> Result<Value, String> {
        self.skip_ws();
        match self.peek() {
            Some('"') => Ok(Value::String(self.parse_string()?)),
            Some('[') => Ok(Value::Array(self.parse_array()?)),
            Some('{') => Ok(Value::Object(self.parse_object()?)),
            Some('n') => self.parse_null(),
            Some(other) => Err(format!("unexpected `{other}`")),
            None => Err("unexpected end of task file".to_string()),
        }
    }

    fn parse_null(&mut self) -> Result<Value, String> {
        for expected in ['n', 'u', 'l', 'l'] {
            if self.bump() != Some(expected) {
                return Err("invalid null".to_string());
            }
        }
        Ok(Value::Null)
    }

    fn parse_array(&mut self) -> Result<Vec<Value>, String> {
        self.bump();
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(']') {
                self.bump();
                return Ok(items);
            }
            if !items.is_empty() {
                self.expect(',')?;
                self.skip_ws();
            }
            items.push(self.parse_value()?);
        }
    }

    fn parse_object(&mut self) -> Result<Vec<(String, Value)>, String> {
        self.bump();
        let mut fields = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some('}') {
                self.bump();
                return Ok(fields);
            }
            if !fields.is_empty() {
                self.expect(',')?;
                self.skip_ws();
            }
            if self.peek() != Some('"') {
                return Err("object key must be a string".to_string());
            }
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(':')?;
            fields.push((key, self.parse_value()?));
        }
    }

    fn expect(&mut self, expected: char) -> Result<(), String> {
        if self.bump() == Some(expected) {
            Ok(())
        } else {
            Err(format!("expected `{expected}`"))
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.bump();
        let mut out = String::new();
        loop {
            match self.bump() {
                Some('"') => return Ok(out),
                Some('\\') => out.push(self.parse_escape()?),
                Some(ch) => out.push(ch),
                None => return Err("unterminated string".to_string()),
            }
        }
    }

    fn parse_escape(&mut self) -> Result<char, String> {
        match self.bump() {
            Some('"') => Ok('"'),
            Some('\\') => Ok('\\'),
            Some('/') => Ok('/'),
            Some('b') => Ok('\u{0008}'),
            Some('f') => Ok('\u{000c}'),
            Some('n') => Ok('\n'),
            Some('r') => Ok('\r'),
            Some('t') => Ok('\t'),
            Some('u') => self.parse_hex_char(),
            _ => Err("invalid string escape".to_string()),
        }
    }

    fn parse_hex_char(&mut self) -> Result<char, String> {
        let mut code = 0;
        for _ in 0..4 {
            let Some(ch) = self.bump() else {
                return Err("short unicode escape".to_string());
            };
            let digit = ch
                .to_digit(16)
                .ok_or_else(|| "invalid unicode escape".to_string())?;
            code = (code << 4) | digit;
        }
        char::from_u32(code).ok_or_else(|| "invalid unicode escape".to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;

    use super::*;
    use crate::{EnvironmentInput, build_environment, load_config};

    fn sample(name: &str, command: &str) -> Task {
        Task::new(name, command).unwrap()
    }

    fn apply(task: &Task, cwd: &Path) -> crate::Environment {
        let config = load_config(&[]).unwrap();
        let inline: Vec<(String, OsString)> = task
            .env
            .iter()
            .map(|(key, value)| (key.clone(), OsString::from(value)))
            .collect();
        build_environment(&EnvironmentInput {
            parent: &[],
            dotenv: &task.env_files,
            unset: &[],
            inline: &inline,
            cwd,
            injector: None,
            config: &config,
            trust: Some(false),
        })
        .unwrap()
    }

    #[test]
    fn folder_task_replaces_global_and_does_not_write_it() {
        let dir = tempfile::tempdir().unwrap();
        let global_path = dir.path().join("tasks.json");
        let folder = dir.path().join("app");
        fs::create_dir_all(&folder).unwrap();
        let store = TaskStore::at(&global_path);
        let global = vec![
            sample("build", "cargo build").with_description("from global"),
            sample("test", "cargo test"),
        ];
        store.save_global(&global).unwrap();
        let before = fs::read(&global_path).unwrap();
        let folder_tasks = vec![
            sample("build", "cargo build --release").with_description("from folder"),
            sample("lint", "cargo clippy"),
        ];
        store.save_folder(&folder, &folder_tasks).unwrap();

        let resolved = store.resolve(&folder).unwrap();
        let after = fs::read(&global_path).unwrap();
        assert_eq!(before, after);
        assert_eq!(resolved.len(), 3);
        assert_eq!(resolved[0].name, "build");
        assert_eq!(resolved[0].command, "cargo build --release");
        assert_eq!(resolved[0].scope, Scope::Folder(folder.clone()));
        assert_eq!(resolved[1].name, "test");
        assert_eq!(resolved[1].scope, Scope::Global);
        assert_eq!(resolved[2].name, "lint");
        assert_eq!(resolved[2].scope, Scope::Folder(folder));
        assert!(!folder_tasks_path(dir.path()).exists());
    }

    #[test]
    fn later_file_overrides_an_earlier_file_and_a_variable_overrides_both() {
        let dir = tempfile::tempdir().unwrap();
        let early = dir.path().join("early.env");
        let late = dir.path().join("late.env");
        fs::write(&early, "COLOR='red'\nSHARED='from-file'\n").unwrap();
        fs::write(&late, "COLOR='blue'\n").unwrap();
        let files_only = sample("paint", "true")
            .with_env_file(&early)
            .with_env_file(&late);
        let from_files = apply(&files_only, dir.path());
        assert_eq!(from_files.get("COLOR"), Some("blue"));
        assert_eq!(from_files.get("SHARED"), Some("from-file"));

        let task = files_only.with_env("COLOR", "green").unwrap();
        let args = launch_args(&task);
        assert_eq!(args[0], early.as_os_str());
        assert_eq!(args[1], late.as_os_str());
        assert_eq!(args[2], "COLOR=green");
        assert_eq!(args[3], "true");
        let env = apply(&task, dir.path());
        assert_eq!(env.get("COLOR"), Some("green"));
        assert_eq!(env.get("SHARED"), Some("from-file"));
    }

    #[test]
    fn filter_matches_name_or_description_and_keeps_order() {
        let tasks = vec![
            sample("Build", "cargo build").with_description("Release candidate"),
            sample("test", "cargo test"),
            sample("lint", "cargo clippy").with_description("Clippy warnings"),
        ];
        let all = filter_tasks(&tasks, "  ");
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].name, "Build");
        assert_eq!(all[2].name, "lint");

        let matched = filter_tasks(&tasks, "CLIP");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].name, "lint");
        let by_name = filter_tasks(&tasks, "bui");
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0].name, "Build");
    }

    #[test]
    fn reorder_stops_at_the_ends() {
        let mut task = sample("build", "true")
            .with_env_file("a.env")
            .with_env_file("b.env")
            .with_env_file("c.env")
            .with_env("ONE", "1")
            .unwrap()
            .with_env("TWO", "2")
            .unwrap();
        move_env_file(&mut task, 0, Move::Up);
        assert_eq!(task.env_files[0], Path::new("a.env"));
        move_env_file(&mut task, 2, Move::Down);
        assert_eq!(task.env_files[2], Path::new("c.env"));
        move_env_file(&mut task, 1, Move::Up);
        assert_eq!(
            task.env_files,
            vec![
                PathBuf::from("b.env"),
                PathBuf::from("a.env"),
                PathBuf::from("c.env")
            ]
        );
        move_env_var(&mut task, 1, Move::Down);
        assert_eq!(task.env[0].0, "ONE");
        move_env_var(&mut task, 0, Move::Down);
        assert_eq!(task.env[0].0, "TWO");
        assert_eq!(task.env[1].0, "ONE");
        move_env_var(&mut task, 9, Move::Up);
        assert_eq!(task.env.len(), 2);
    }

    #[test]
    fn save_and_load_round_trip_including_the_injector() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.json");
        let mut store = TaskStore::at(&path);
        store.set_injector(Some(vec!["inject".into(), "--from-store".into()]));
        let task = sample("build", "cargo build")
            .with_description("the build")
            .with_env_file("a.env")
            .with_env_file("b.env")
            .with_env("MODE", "fast")
            .unwrap()
            .with_icon("app.png")
            .unwrap()
            .with_shortcut("mod+b")
            .with_injector(vec!["inject".into(), "--from-task".into()]);
        store.save_global(std::slice::from_ref(&task)).unwrap();

        let mut loaded_store = TaskStore::at(&path);
        let loaded = loaded_store.load_global().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "build");
        assert_eq!(loaded[0].description.as_deref(), Some("the build"));
        assert_eq!(loaded[0].command, "cargo build");
        assert_eq!(
            loaded[0].env_files,
            vec![PathBuf::from("a.env"), PathBuf::from("b.env")]
        );
        assert_eq!(loaded[0].env, vec![("MODE".into(), "fast".into())]);
        assert_eq!(loaded[0].icon.as_deref(), Some("app.png"));
        assert_eq!(loaded[0].shortcut.as_deref(), Some("mod+b"));
        assert_eq!(
            loaded[0].injector.as_deref(),
            Some(["inject".to_string(), "--from-task".to_string()].as_slice())
        );
        assert_eq!(loaded[0].scope, Scope::Global);
        assert_eq!(
            loaded_store.injector(),
            Some(["inject".to_string(), "--from-store".to_string()].as_slice())
        );

        let request = launch_request(&loaded_store, &loaded[0]);
        assert_eq!(
            request.injector.as_deref(),
            Some(["inject".to_string(), "--from-task".to_string()].as_slice())
        );
        let mut plain = loaded[0].clone();
        plain.injector = None;
        let fallback = launch_request(&loaded_store, &plain);
        assert_eq!(fallback.injector.as_deref(), loaded_store.injector());
    }

    #[test]
    fn add_icon_copies_the_file_and_rejects_an_empty_name() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.png");
        fs::write(&source, b"png-bytes").unwrap();
        let icons = dir.path().join("icons");
        let err = add_icon(&icons, "  ", &source).unwrap_err();
        assert_eq!(err.code, 1);
        let id = add_icon(&icons, "app.png", &source).unwrap();
        assert_eq!(id, "app.png");
        assert_eq!(fs::read(icons.join(&id)).unwrap(), b"png-bytes");
        let task = sample("build", "true").with_icon(id).unwrap();
        assert_eq!(task.icon.as_deref(), Some("app.png"));
    }

    #[test]
    fn empty_name_is_rejected() {
        let err = Task::new("  ", "true").unwrap_err();
        assert_eq!(err.code, 1);
        assert!(Task::new("build", " ").is_err());
    }
}
