use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};

use crate::RunOptions;
use crate::cli::{self, Invocation};
use crate::config::{self, Config, HostOs};
use crate::dotenv::{self, EnvMap};
use crate::error::Error;
use crate::resolve::{self, Finder};
use crate::script::{self, Graph, Pipeline};
use crate::shebang::{self, Shebang};
use crate::template;

struct Ctx<'a> {
    cwd: &'a Path,
    env: &'a EnvMap,
    config: &'a Config,
    finder: &'a Finder,
    trust_on: bool,
    sudo: bool,
    opener: Option<&'a OsStr>,
    skip_detach: bool,
    args: &'a [OsString],
}

enum Mode {
    Implicit,
    Run,
}

pub(crate) fn run(opts: RunOptions) -> Result<i32, Error> {
    let args = opts.args.unwrap_or_else(|| std::env::args_os().collect());
    if args.is_empty() {
        return Err(Error::usage("missing command"));
    }
    let start = std::env::current_dir()
        .map_err(|err| Error::new(1, format!("current directory: {err}")))?;
    let inv = cli::parse(&args, start)?;
    if inv.help {
        write_stdout(HELP)?;
        return Ok(0);
    }
    if inv.version {
        write_stdout(&format!("vrun {}\n", env!("CARGO_PKG_VERSION")))?;
        return Ok(0);
    }
    if !inv.cwd.is_dir() {
        return Err(Error::new(
            1,
            format!("not a directory: {}", inv.cwd.display()),
        ));
    }

    let home = env_path("HOME");
    let profile = env_path("USERPROFILE");
    let extra = env_path("VAELRUN_CONFIG");
    let paths = config::config_file_paths(
        host(),
        home.as_deref(),
        profile.as_deref(),
        extra.as_deref(),
    );
    let config = config::load_config(&paths)?;
    let trust_on = inv.trust.unwrap_or_else(|| config.trust_on());
    let bin_dirs = expand_bins(&config, home.as_deref(), &inv.cwd);
    let parent: Vec<_> = std::env::vars_os().collect();
    let parent_finder = build_finder(&bin_dirs, &dotenv::parent_map(&parent));
    let injector = resolve_injector(&config, &parent_finder, &inv.cwd, trust_on)?;
    let env = dotenv::build_env(
        &parent,
        &inv.dotenv,
        &inv.unset,
        &inv.inline,
        &inv.cwd,
        injector.as_ref().map(|item| dotenv::Injector {
            argv: &item.argv,
            program: &item.program,
        }),
    )?;
    let finder = build_finder(&bin_dirs, &env);
    let skip_detach = std::env::var_os("VAELRUN_DETACHED").as_deref() == Some(OsStr::new("1"));
    let ctx = Ctx {
        cwd: &inv.cwd,
        env: &env,
        config: &config,
        finder: &finder,
        trust_on,
        sudo: inv.sudo,
        opener: opts.opener.as_deref(),
        skip_detach,
        args: &args,
    };

    if let Some(path) = &inv.edit {
        return edit(path, &ctx);
    }
    if inv.command.is_empty() {
        return Err(Error::usage("missing command"));
    }
    if !inv.force_run
        && let Some(code) = subcommand(&inv, &bin_dirs, &ctx)?
    {
        return Ok(code);
    }
    let mode = if inv.force_run {
        Mode::Run
    } else {
        Mode::Implicit
    };
    dispatch(&inv.command[0], &inv.command[1..], mode, inv.detach, &ctx)
}

fn subcommand(inv: &Invocation, bin_dirs: &[PathBuf], ctx: &Ctx<'_>) -> Result<Option<i32>, Error> {
    let name = inv.command[0].to_str();
    match name {
        Some("open") => {
            if inv.command.len() != 2 {
                return Err(Error::usage("open needs a target"));
            }
            Ok(Some(open_target(&inv.command[1], inv.detach, ctx)?))
        }
        Some("start") => {
            if inv.command.len() < 2 {
                return Err(Error::usage("start needs a command"));
            }
            Ok(Some(dispatch(
                &inv.command[1],
                &inv.command[2..],
                Mode::Implicit,
                true,
                ctx,
            )?))
        }
        Some("run") => {
            if inv.command.len() < 2 {
                return Err(Error::usage("run needs a command"));
            }
            Ok(Some(dispatch(
                &inv.command[1],
                &inv.command[2..],
                Mode::Run,
                inv.detach,
                ctx,
            )?))
        }
        Some("complete") => {
            if inv.command.len() != 2 {
                return Err(Error::usage("complete needs a shell"));
            }
            let Some(shell) = inv.command[1].to_str() else {
                return Err(Error::usage("complete needs a shell"));
            };
            Ok(Some(crate::complete::print_complete(
                shell, ctx.config, bin_dirs,
            )?))
        }
        _ => Ok(None),
    }
}

fn dispatch(
    token: &OsStr,
    args: &[OsString],
    mode: Mode,
    detach: bool,
    ctx: &Ctx<'_>,
) -> Result<i32, Error> {
    let path = cli::resolve_path(ctx.cwd, token);
    let unqualified = cli::is_unqualified(token);
    let suffix = cli::extension_suffix(token);
    let implicit = matches!(mode, Mode::Implicit);

    if implicit && path.is_dir() {
        return open_target(path.as_os_str(), detach, ctx);
    }
    if implicit && cli::is_url(token) {
        return open_target(token, detach, ctx);
    }

    if let Some(suffix) = suffix.as_deref()
        && (suffix == ".vrun" || is_mapped(suffix, ctx))
    {
        if path.is_file() {
            return if suffix == ".vrun" {
                run_script(&path, detach, ctx)
            } else {
                run_extension(suffix, &path, args, detach, token, ctx)
            };
        }
        if unqualified && let Some(code) = try_alias(token, args, detach, ctx)? {
            return Ok(code);
        }
        return Err(Error::not_found(&token.to_string_lossy()));
    }

    if path.is_file() && !unqualified {
        return run_existing(&path, args, mode, detach, token, ctx);
    }
    if unqualified {
        if let Some(code) = try_alias(token, args, detach, ctx)? {
            return Ok(code);
        }
        if let Some(hit) = ctx.finder.lookup(&token.to_string_lossy()) {
            if !hit.allowed(ctx.trust_on) {
                return Err(Error::trust(&token.to_string_lossy()));
            }
            return run_program(&hit.into_path(), args, detach, token, ctx);
        }
        if implicit && suffix.is_some() {
            return open_target(token, detach, ctx);
        }
        return Err(Error::not_found(&token.to_string_lossy()));
    }
    if implicit {
        return open_target(token, detach, ctx);
    }
    if path.exists() {
        Err(Error::cannot_execute(&path.display().to_string()))
    } else {
        Err(Error::not_found(&token.to_string_lossy()))
    }
}

fn run_existing(
    path: &Path,
    args: &[OsString],
    mode: Mode,
    detach: bool,
    original: &OsStr,
    ctx: &Ctx<'_>,
) -> Result<i32, Error> {
    if let Some(shebang) = shebang::read_shebang(path)? {
        return run_shebang(&shebang, path, args, detach, original, ctx);
    }
    if resolve::is_executable(path) {
        return exec_path(path, args, detach, original, ctx);
    }
    if matches!(mode, Mode::Run) {
        return Err(Error::cannot_execute(&path.display().to_string()));
    }
    open_target(path.as_os_str(), detach, ctx)
}

fn run_program(
    path: &Path,
    args: &[OsString],
    detach: bool,
    original: &OsStr,
    ctx: &Ctx<'_>,
) -> Result<i32, Error> {
    if let Some(shebang) = shebang::read_shebang(path)? {
        return run_shebang(&shebang, path, args, detach, original, ctx);
    }
    exec_path(path, args, detach, original, ctx)
}

fn try_alias(
    token: &OsStr,
    args: &[OsString],
    detach: bool,
    ctx: &Ctx<'_>,
) -> Result<Option<i32>, Error> {
    let Some(name) = token.to_str() else {
        return Ok(None);
    };
    let Some(alias) = ctx.config.aliases.get(name) else {
        return Ok(None);
    };
    let mut argv: Vec<OsString> = alias.iter().map(OsString::from).collect();
    argv.extend(args.iter().cloned());
    let program = resolve::resolve_program(&argv[0], ctx.finder, ctx.cwd, ctx.trust_on)?;
    argv[0] = program.into();
    Ok(Some(finish(argv, detach, token, ctx)?))
}

fn exec_path(
    path: &Path,
    args: &[OsString],
    detach: bool,
    original: &OsStr,
    ctx: &Ctx<'_>,
) -> Result<i32, Error> {
    let mut argv = vec![path.as_os_str().to_os_string()];
    argv.extend(args.iter().cloned());
    finish(argv, detach, original, ctx)
}

fn finish(
    mut argv: Vec<OsString>,
    detach_flag: bool,
    original: &OsStr,
    ctx: &Ctx<'_>,
) -> Result<i32, Error> {
    if argv.is_empty() {
        return Err(Error::usage("missing command"));
    }
    let program = PathBuf::from(&argv[0]);
    let detach = detach_flag || listed(&program, original, &ctx.config.detach);
    if let Some(sudo) = resolve_sudo(ctx)? {
        argv.insert(0, sudo.into());
    }
    spawn(&argv, ctx, detach)
}

/// Resolve `sudo` once. A PATH-only hit is refused with 126 when trust is on.
fn resolve_sudo(ctx: &Ctx<'_>) -> Result<Option<PathBuf>, Error> {
    if !ctx.sudo {
        return Ok(None);
    }
    resolve::resolve_program(OsStr::new("sudo"), ctx.finder, ctx.cwd, ctx.trust_on).map(Some)
}

fn run_shebang(
    shebang: &Shebang,
    script: &Path,
    args: &[OsString],
    detach: bool,
    original: &OsStr,
    ctx: &Ctx<'_>,
) -> Result<i32, Error> {
    let interp = match shebang::classify_interp(&shebang.interpreter) {
        shebang::Interp::Absolute(path) => path,
        shebang::Interp::Name(name) => {
            resolve::resolve_program(OsStr::new(&name), ctx.finder, ctx.cwd, ctx.trust_on)?
        }
        shebang::Interp::Missing(name) => return Err(Error::not_found(&name)),
    };
    let mut argv = vec![interp.into_os_string()];
    argv.extend(shebang.args.iter().map(OsString::from));
    argv.push(script.as_os_str().to_os_string());
    argv.extend(args.iter().cloned());
    finish(argv, detach, original, ctx)
}

fn is_mapped(suffix: &str, ctx: &Ctx<'_>) -> bool {
    ctx.config.extensions.contains_key(suffix) || is_builtin(suffix)
}

fn is_builtin(suffix: &str) -> bool {
    matches!(
        suffix,
        ".ts"
            | ".tsx"
            | ".js"
            | ".mjs"
            | ".cjs"
            | ".ps1"
            | ".sh"
            | ".go"
            | ".cs"
            | ".csproj"
            | ".py"
            | ".rb"
    )
}

fn run_extension(
    suffix: &str,
    file: &Path,
    extras: &[OsString],
    detach: bool,
    original: &OsStr,
    ctx: &Ctx<'_>,
) -> Result<i32, Error> {
    let argv = if let Some(tmpl) = ctx.config.extensions.get(suffix) {
        let mut argv = template::apply_template(tmpl, file.as_os_str())?;
        if argv.is_empty() {
            return Err(Error::new(1, "invalid extension template"));
        }
        let program = resolve::resolve_program(&argv[0], ctx.finder, ctx.cwd, ctx.trust_on)?;
        argv[0] = program.into();
        argv.extend(extras.iter().cloned());
        argv
    } else {
        builtin_argv(suffix, file, extras, ctx)?
    };
    finish(argv, detach, original, ctx)
}

enum Found {
    Path(PathBuf),
    Refused,
    Missing,
}

fn classify(name: &str, ctx: &Ctx<'_>) -> Found {
    match ctx.finder.lookup(name) {
        None => Found::Missing,
        Some(hit) if !hit.allowed(ctx.trust_on) => Found::Refused,
        Some(hit) => Found::Path(hit.into_path()),
    }
}

fn pick(primary: &str, fallback: Option<&str>, ctx: &Ctx<'_>) -> Result<PathBuf, Error> {
    match classify(primary, ctx) {
        Found::Path(path) => Ok(path),
        Found::Refused => Err(Error::trust(primary)),
        Found::Missing => match fallback {
            None => Err(Error::not_found(primary)),
            Some(alt) => match classify(alt, ctx) {
                Found::Path(path) => Ok(path),
                Found::Refused => Err(Error::trust(alt)),
                Found::Missing => Err(Error::not_found(primary)),
            },
        },
    }
}

fn builtin_argv(
    suffix: &str,
    file: &Path,
    extras: &[OsString],
    ctx: &Ctx<'_>,
) -> Result<Vec<OsString>, Error> {
    let file_arg = file.as_os_str().to_os_string();
    let mut argv = match suffix {
        ".ts" | ".tsx" => vec![pick("bun", None, ctx)?.into(), file_arg],
        ".js" | ".mjs" | ".cjs" => vec![pick("bun", Some("node"), ctx)?.into(), file_arg],
        ".ps1" => {
            let prog = pick("pwsh", Some("powershell"), ctx)?;
            vec![
                prog.into(),
                OsString::from("-ExecutionPolicy"),
                OsString::from("RemoteSigned"),
                OsString::from("-NoLogo"),
                OsString::from("-File"),
                file_arg,
            ]
        }
        ".sh" => vec![pick("bash", Some("sh"), ctx)?.into(), file_arg],
        ".go" => vec![
            pick("go", None, ctx)?.into(),
            OsString::from("run"),
            file_arg,
        ],
        ".cs" => vec![
            pick("dotnet", None, ctx)?.into(),
            OsString::from("run"),
            file_arg,
            OsString::from("--"),
        ],
        ".csproj" => vec![
            pick("dotnet", None, ctx)?.into(),
            OsString::from("run"),
            OsString::from("--project"),
            file_arg,
            OsString::from("--"),
        ],
        ".py" => vec![pick("python3", Some("python"), ctx)?.into(), file_arg],
        ".rb" => vec![pick("ruby", None, ctx)?.into(), file_arg],
        _ => return Err(Error::not_found(suffix)),
    };
    argv.extend(extras.iter().cloned());
    Ok(argv)
}

fn edit(path: &Path, ctx: &Ctx<'_>) -> Result<i32, Error> {
    let name = ctx
        .env
        .get("VISUAL")
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            ctx.env
                .get("EDITOR")
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });
    // The editor is chosen by the user or is the built-in default. A PATH hit
    // is allowed so `vrun -e` still works when trust would refuse `vi`.
    let editor = resolve::resolve_lenient(&name, ctx.finder, ctx.cwd)?;
    spawn(
        &[editor.into(), path.as_os_str().to_os_string()],
        ctx,
        false,
    )
}

fn open_target(target: &OsStr, detach: bool, ctx: &Ctx<'_>) -> Result<i32, Error> {
    if let Some(opener) = ctx.opener {
        return spawn(&[opener.to_os_string(), target.to_os_string()], ctx, detach);
    }
    // Linux: xdg-open. macOS: open. Windows directories: explorer.exe.
    // Documents and URLs on Windows use ShellExecuteW. `cmd /c start` is not
    // used because it splits URLs on `&`.
    #[cfg(windows)]
    {
        if Path::new(target).is_dir() {
            let explorer = resolve::resolve_lenient("explorer", ctx.finder, ctx.cwd)?;
            return spawn(&[explorer.into(), target.to_os_string()], ctx, detach);
        }
        shell_execute(target)?;
        return Ok(0);
    }
    #[cfg(not(windows))]
    {
        let bin = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let opener = resolve::resolve_lenient(bin, ctx.finder, ctx.cwd)?;
        spawn(&[opener.into(), target.to_os_string()], ctx, detach)
    }
}

fn run_script(path: &Path, detach: bool, ctx: &Ctx<'_>) -> Result<i32, Error> {
    if detach && !ctx.skip_detach {
        return reexec_detached(ctx);
    }
    let text = std::fs::read_to_string(path).map_err(|err| Error::io(path, &err))?;
    let mut status = 0;
    let mut first = true;
    for (idx, line) in text.lines().enumerate() {
        if first {
            first = false;
            if line.starts_with("#!") {
                continue;
            }
        }
        if line.trim().is_empty() {
            continue;
        }
        let line_no = idx + 1;
        let expanded = script::expand_templates(line, &ctx.env.utf8)
            .map_err(|_| Error::at(path, line_no, "unknown template"))?;
        let graph = match script::parse_script_line(&expanded) {
            Ok(Some(graph)) => graph,
            Ok(None) => continue,
            Err(reason) => return Err(Error::at(path, line_no, &reason)),
        };
        status = run_graph(&graph, ctx)?;
        if status != 0 {
            return Ok(status);
        }
    }
    Ok(status)
}

fn run_graph(graph: &Graph, ctx: &Ctx<'_>) -> Result<i32, Error> {
    let mut status = run_pipeline(&graph.first, ctx)?;
    for (sep, pipe) in &graph.rest {
        let go = match sep {
            script::Sep::And => status == 0,
            script::Sep::Or => status != 0,
            script::Sep::Semi => true,
        };
        if go {
            status = run_pipeline(pipe, ctx)?;
        }
    }
    Ok(status)
}

fn run_pipeline(pipe: &Pipeline, ctx: &Ctx<'_>) -> Result<i32, Error> {
    if pipe.commands.len() == 1 {
        let cmd = &pipe.commands[0];
        if cmd.stdin.is_none() && cmd.stdout.is_none() && cmd.stderr.is_none() {
            let program = resolve::resolve_program(
                OsStr::new(&cmd.argv[0]),
                ctx.finder,
                ctx.cwd,
                ctx.trust_on,
            )?;
            let args: Vec<OsString> = cmd.argv[1..].iter().map(OsString::from).collect();
            return exec_path(&program, &args, false, OsStr::new(&cmd.argv[0]), ctx);
        }
    }
    let mut prepared = Vec::new();
    for cmd in &pipe.commands {
        if cmd.argv.is_empty() {
            return Err(Error::new(1, "empty command"));
        }
        let program =
            resolve::resolve_program(OsStr::new(&cmd.argv[0]), ctx.finder, ctx.cwd, ctx.trust_on)?;
        prepared.push(Prepared {
            program,
            args: cmd.argv[1..].iter().map(OsString::from).collect(),
            stdin: cmd.stdin.clone(),
            stdout: cmd.stdout.clone(),
            stderr: cmd.stderr.clone(),
        });
    }
    spawn_pipeline(&prepared, ctx)
}

struct Prepared {
    program: PathBuf,
    args: Vec<OsString>,
    stdin: Option<String>,
    stdout: Option<(String, bool)>,
    stderr: Option<(String, bool)>,
}

fn spawn_pipeline(cmds: &[Prepared], ctx: &Ctx<'_>) -> Result<i32, Error> {
    // One resolution for the whole pipeline. Each stage is prefixed; the
    // script interpreter is not wrapped.
    let sudo = resolve_sudo(ctx)?;
    let mut children = Vec::new();
    let mut previous: Option<std::process::ChildStdout> = None;
    for (idx, spec) in cmds.iter().enumerate() {
        let last = idx + 1 == cmds.len();
        let launched = sudo.as_deref().unwrap_or(spec.program.as_path());
        let mut cmd = match &sudo {
            Some(sudo) => {
                let mut cmd = Command::new(sudo);
                cmd.arg(&spec.program);
                cmd.args(&spec.args);
                cmd
            }
            None => {
                let mut cmd = Command::new(&spec.program);
                cmd.args(&spec.args);
                cmd
            }
        };
        apply_child(&mut cmd, ctx);
        if let Some(out) = previous.take() {
            if last && spec.stdin.is_some() {
                drop(out);
                cmd.stdin(open_read(ctx.cwd, spec.stdin.as_deref().unwrap_or(""))?);
            } else {
                cmd.stdin(Stdio::from(out));
            }
        } else if let Some(path) = &spec.stdin {
            cmd.stdin(open_read(ctx.cwd, path)?);
        }
        if !last {
            if spec.stdout.is_some() {
                kill_children(&mut children);
                return Err(Error::new(
                    1,
                    "stdout redirection is only supported on the last command",
                ));
            }
            cmd.stdout(Stdio::piped());
        } else if let Some((path, append)) = &spec.stdout {
            cmd.stdout(open_write(ctx.cwd, path, *append)?);
        }
        if let Some((path, append)) = &spec.stderr {
            cmd.stderr(open_write(ctx.cwd, path, *append)?);
        }
        match cmd.spawn() {
            Ok(mut child) => {
                previous = if last { None } else { child.stdout.take() };
                children.push(child);
            }
            Err(err) => {
                kill_children(&mut children);
                return Err(map_spawn(launched, err));
            }
        }
    }
    let mut status = 0;
    for (idx, child) in children.iter_mut().enumerate() {
        let waited = child
            .wait()
            .map_err(|err| Error::new(1, format!("wait: {err}")))?;
        if idx + 1 == cmds.len() {
            status = exit_code(waited);
        }
    }
    Ok(status)
}

fn spawn(argv: &[OsString], ctx: &Ctx<'_>, detach: bool) -> Result<i32, Error> {
    if argv.is_empty() {
        return Err(Error::usage("missing command"));
    }
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    apply_child(&mut cmd, ctx);
    if detach && !ctx.skip_detach {
        configure_detach(&mut cmd);
        cmd.spawn()
            .map_err(|err| map_spawn(Path::new(&argv[0]), err))?;
        return Ok(0);
    }
    let status = cmd
        .status()
        .map_err(|err| map_spawn(Path::new(&argv[0]), err))?;
    Ok(exit_code(status))
}

fn apply_child(cmd: &mut Command, ctx: &Ctx<'_>) {
    cmd.env_clear();
    cmd.envs(ctx.env.pairs.iter().cloned());
    cmd.current_dir(ctx.cwd);
}

fn reexec_detached(ctx: &Ctx<'_>) -> Result<i32, Error> {
    let exe =
        std::env::current_exe().map_err(|err| Error::new(1, format!("current exe: {err}")))?;
    let mut cmd = Command::new(exe);
    if ctx.args.len() > 1 {
        cmd.args(&ctx.args[1..]);
    }
    cmd.env_clear();
    cmd.envs(ctx.env.pairs.iter().cloned());
    cmd.env("VAELRUN_DETACHED", "1");
    cmd.current_dir(ctx.cwd);
    configure_detach(&mut cmd);
    cmd.spawn()
        .map_err(|err| Error::new(1, format!("failed to detach: {err}")))?;
    Ok(0)
}

fn configure_detach(cmd: &mut Command) {
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());
    #[cfg(unix)]
    detach_unix(cmd);
    #[cfg(windows)]
    detach_windows(cmd);
}

/// `process_group(0)` only calls `setpgid`. The child stays in this session, so
/// the terminal still delivers SIGHUP. ghostty and `[detach]` names must call
/// `setsid`. std does not expose that safely; this is the one Unix unsafe call.
#[cfg(unix)]
#[allow(unsafe_code)]
fn detach_unix(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        cmd.pre_exec(|| {
            if setsid() < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
unsafe extern "C" {
    fn setsid() -> i32;
}

#[cfg(windows)]
fn detach_windows(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS | CREATE_NO_WINDOW);
}

/// `cmd /c start` splits on `&`. `ShellExecuteW` is the documented opener.
/// Kept in this function so the unsafe call does not spread.
#[cfg(windows)]
#[allow(unsafe_code)]
fn shell_execute(target: &OsStr) -> Result<(), Error> {
    use std::os::windows::ffi::OsStrExt;
    let op: Vec<u16> = OsStr::new("open")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let file: Vec<u16> = target.encode_wide().chain(std::iter::once(0)).collect();
    let rc = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    };
    if rc <= 32 {
        Err(Error::new(
            1,
            format!(
                "failed to open {}: ShellExecuteW error {rc}",
                target.to_string_lossy()
            ),
        ))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
#[link(name = "shell32")]
unsafe extern "system" {
    fn ShellExecuteW(
        hwnd: *mut std::ffi::c_void,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> isize;
}

fn listed(program: &Path, original: &OsStr, names: &[String]) -> bool {
    let file = program
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let stem = file.strip_suffix(".exe").unwrap_or(file);
    let orig = original.to_string_lossy();
    if stem.eq_ignore_ascii_case("ghostty") || orig.eq_ignore_ascii_case("ghostty") {
        return true;
    }
    names.iter().any(|name| {
        name.eq_ignore_ascii_case(stem)
            || name.eq_ignore_ascii_case(file)
            || name.eq_ignore_ascii_case(&orig)
    })
}

fn open_read(cwd: &Path, raw: &str) -> Result<File, Error> {
    let path = cli::resolve_path(cwd, OsStr::new(raw));
    File::open(&path).map_err(|err| Error::io(&path, &err))
}

fn open_write(cwd: &Path, raw: &str, append: bool) -> Result<File, Error> {
    let path = cli::resolve_path(cwd, OsStr::new(raw));
    let mut opts = OpenOptions::new();
    opts.write(true).create(true);
    if append {
        opts.append(true);
    } else {
        opts.truncate(true);
    }
    opts.open(&path).map_err(|err| Error::io(&path, &err))
}

fn kill_children(children: &mut [Child]) {
    for child in children.iter_mut() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn map_spawn(program: &Path, err: io::Error) -> Error {
    if err.kind() == io::ErrorKind::NotFound {
        Error::not_found(&program.display().to_string())
    } else if err.kind() == io::ErrorKind::PermissionDenied {
        Error::cannot_execute(&program.display().to_string())
    } else {
        Error::io(program, &err)
    }
}

fn exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return 128 + sig;
        }
    }
    1
}

struct ResolvedInjector {
    argv: Vec<String>,
    program: PathBuf,
}

fn resolve_injector(
    config: &Config,
    finder: &Finder,
    cwd: &Path,
    trust_on: bool,
) -> Result<Option<ResolvedInjector>, Error> {
    let Some(argv) = config.injector_argv() else {
        return Ok(None);
    };
    let program = resolve::resolve_program(OsStr::new(&argv[0]), finder, cwd, trust_on)?;
    Ok(Some(ResolvedInjector {
        argv: argv.to_vec(),
        program,
    }))
}

fn expand_bins(config: &Config, home: Option<&Path>, cwd: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for raw in &config.bin_dirs {
        let path = config::expand_bin_dir(raw, home, cwd);
        if !dirs.contains(&path) {
            dirs.push(path);
        }
    }
    dirs
}

fn build_finder(bin_dirs: &[PathBuf], env: &EnvMap) -> Finder {
    let home = env.get("HOME").map(PathBuf::from).or_else(|| {
        if cfg!(windows) {
            env.get("USERPROFILE").map(PathBuf::from)
        } else {
            None
        }
    });
    Finder::new(
        bin_dirs.to_vec(),
        env.get("PATH").map(OsStr::new),
        env.get("PATHEXT").map(OsStr::new),
        home,
        env.get("DOTNET_ROOT").map(PathBuf::from),
        env.get("GOROOT").map(PathBuf::from),
        env.get("ProgramFiles").map(PathBuf::from),
    )
}

fn host() -> HostOs {
    if cfg!(windows) {
        HostOs::Windows
    } else if cfg!(target_os = "macos") {
        HostOs::Macos
    } else {
        HostOs::Linux
    }
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn write_stdout(text: &str) -> Result<(), Error> {
    let mut out = io::stdout().lock();
    out.write_all(text.as_bytes())
        .map_err(|err| Error::new(1, format!("stdout: {err}")))?;
    out.flush()
        .map_err(|err| Error::new(1, format!("stdout: {err}")))?;
    Ok(())
}

const HELP: &str = "\
vrun — cross-platform run, open, and start

Usage:
  vrun [options] [NAME=value]... [dotenv]... <command> [args]...
  vrun [options] open <target>
  vrun [options] start <command> [args]...
  vrun [options] run <command> [args]...
  vrun [options] -e <path>
  vrun complete bash|zsh|fish|pwsh

Options:
  -e, --edit <path>     Open path in $VISUAL, $EDITOR, or vi/notepad
  --env-file <path>     Load a dotenv file
  -C, --chdir <dir>     Working directory
  -u, --unset <NAME>    Remove a variable
  --trust, --no-trust   Override trust
  --detach              Do not wait
  --sudo                Prefix sudo after resolution
  -h, --help            Show this help
  --version             Show version
";
