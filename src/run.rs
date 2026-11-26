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
    Start,
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
        let name = invocation_name(args.first().map(OsString::as_os_str));
        write_stdout(&help_text(&name))?;
        return Ok(0);
    }
    if inv.version {
        write_stdout(&format!("vaelrun {}\n", env!("CARGO_PKG_VERSION")))?;
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
    let extra = opts.config.clone().or_else(|| env_path("VAELRUN_CONFIG"));
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
            Ok(Some(open_target(&inv.command[1], inv.detach, ctx, false)?))
        }
        Some("start") => {
            if inv.command.len() < 2 {
                return Err(Error::usage("start needs a command"));
            }
            Ok(Some(dispatch(
                &inv.command[1],
                &inv.command[2..],
                Mode::Start,
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
    dispatch_inner(token, args, mode, detach, ctx, true)
}

fn dispatch_inner(
    token: &OsStr,
    args: &[OsString],
    mode: Mode,
    detach: bool,
    ctx: &Ctx<'_>,
    follow_alias: bool,
) -> Result<i32, Error> {
    if cli::is_url(token) && !ctx.config.urls_enabled() {
        return Err(Error::new(1, "URL opening is disabled"));
    }

    let path = cli::resolve_path(ctx.cwd, token);
    let unqualified = cli::is_unqualified(token);
    let suffix = cli::extension_suffix(token);
    let implicit = !matches!(mode, Mode::Run);

    if implicit && path.is_dir() {
        return open_target(
            path.as_os_str(),
            detach,
            ctx,
            starting_app(token, ctx, mode),
        );
    }
    if implicit && cli::is_url(token) {
        return open_target(token, detach, ctx, false);
    }

    if let Some(suffix) = suffix.as_deref() {
        if suffix == ".vrun" || is_mapped(suffix, ctx) {
            if path.is_file() {
                return dispatch_file(&path, Some(suffix), args, mode, detach, token, ctx);
            }
            if unqualified
                && follow_alias
                && let Some(code) = try_alias(token, args, mode, detach, ctx)?
            {
                return Ok(code);
            }
            return Err(Error::not_found(&token.to_string_lossy()));
        }
        if is_script_suffix(suffix, ctx) && path.is_file() {
            return dispatch_file(&path, Some(suffix), args, mode, detach, token, ctx);
        }
    }

    if path.is_file() && !unqualified {
        return dispatch_file(&path, suffix.as_deref(), args, mode, detach, token, ctx);
    }
    if unqualified && follow_alias {
        if let Some(code) = try_alias(token, args, mode, detach, ctx)? {
            return Ok(code);
        }
        if let Some(hit) = ctx.finder.lookup(&token.to_string_lossy()) {
            if !hit.allowed(ctx.trust_on) {
                return Err(Error::trust(&token.to_string_lossy()));
            }
            return run_program(&hit.into_path(), args, detach, token, ctx);
        }
        if implicit && suffix.is_some() {
            return open_target(token, detach, ctx, false);
        }
        return Err(Error::not_found(&token.to_string_lossy()));
    }
    if implicit {
        return open_target(token, detach, ctx, false);
    }
    if path.exists() {
        Err(Error::cannot_execute(&path.display().to_string()))
    } else {
        Err(Error::not_found(&token.to_string_lossy()))
    }
}

/// Shebang wins over an extension executor. A file that is neither is opened,
/// unless `run` was asked to execute it.
fn dispatch_file(
    path: &Path,
    suffix: Option<&str>,
    args: &[OsString],
    mode: Mode,
    detach: bool,
    original: &OsStr,
    ctx: &Ctx<'_>,
) -> Result<i32, Error> {
    if suffix == Some(".vrun") {
        return run_script(path, detach, ctx);
    }
    let probed = if may_sniff(suffix, ctx) {
        Some(shebang::probe(path)?)
    } else {
        None
    };
    if let Some(shebang) = probed.as_ref().and_then(|item| item.shebang.as_ref()) {
        return run_shebang(shebang, path, args, detach, original, ctx);
    }
    if let Some(suffix) = suffix
        && is_mapped(suffix, ctx)
    {
        return run_extension(suffix, path, args, detach, original, ctx);
    }
    if suffix.is_none()
        && probed.as_ref().is_some_and(|item| item.binary)
        && resolve::is_executable(path)
    {
        return exec_path(path, args, detach, original, ctx);
    }
    if matches!(mode, Mode::Run) {
        return Err(Error::cannot_execute(&path.display().to_string()));
    }
    open_target(
        path.as_os_str(),
        detach,
        ctx,
        starting_app(original, ctx, mode),
    )
}

fn run_program(
    path: &Path,
    args: &[OsString],
    detach: bool,
    original: &OsStr,
    ctx: &Ctx<'_>,
) -> Result<i32, Error> {
    let suffix = cli::extension_suffix(path.as_os_str());
    if may_sniff(suffix.as_deref(), ctx)
        && let Some(shebang) = shebang::probe(path)?.shebang
    {
        return run_shebang(&shebang, path, args, detach, original, ctx);
    }
    exec_path(path, args, detach, original, ctx)
}

/// Alias targets use the same file rules as a typed path. They are not expanded again.
fn try_alias(
    token: &OsStr,
    args: &[OsString],
    mode: Mode,
    detach: bool,
    ctx: &Ctx<'_>,
) -> Result<Option<i32>, Error> {
    let Some(name) = token.to_str() else {
        return Ok(None);
    };
    let Some(alias) = ctx.config.aliases.get(name).cloned() else {
        return Ok(None);
    };
    if alias.is_empty() {
        return Ok(None);
    }
    let target = OsString::from(&alias[0]);
    let mut forwarded: Vec<OsString> = alias[1..].iter().map(OsString::from).collect();
    forwarded.extend(args.iter().cloned());
    Ok(Some(dispatch_inner(
        &target, &forwarded, mode, detach, ctx, false,
    )?))
}

fn may_sniff(suffix: Option<&str>, ctx: &Ctx<'_>) -> bool {
    match suffix {
        None => true,
        Some(suffix) => {
            is_builtin(suffix)
                || ctx.config.extensions.contains_key(suffix)
                || is_script_suffix(suffix, ctx)
        }
    }
}

fn is_script_suffix(suffix: &str, ctx: &Ctx<'_>) -> bool {
    ctx.config.scripts.iter().any(|have| have == suffix)
}

fn starting_app(token: &OsStr, ctx: &Ctx<'_>, mode: Mode) -> bool {
    matches!(mode, Mode::Start) && resolve_app(token, ctx).is_some()
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
    if let Some(name) = runtime_name(&shebang.interpreter) {
        let script_text = script.to_string_lossy();
        let extra = owned_args(args);
        let extra_refs = refs_of(&extra);
        let shell_args = shebang.args.clone();
        let shell_refs = refs_of(&shell_args);
        if let Some(cmd) = runtime_shebang(name, &script_text, &shell_refs, &extra_refs) {
            let argv = authorize_builder(cmd, ctx)?;
            return finish(argv, detach, original, ctx);
        }
    }
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
            | ".csx"
            | ".dll"
            | ".exe"
            | ".csproj"
            | ".fsproj"
            | ".vbproj"
            | ".py"
            | ".rb"
            | ".nu"
            | ".exs"
            | ".lua"
            | ".php"
            | ".tcl"
            | ".fish"
            | ".groovy"
            | ".awk"
            | ".dart"
            | ".jl"
            | ".pl"
            | ".rkt"
            | ".zsh"
            | ".bat"
            | ".cmd"
            | ".r"
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

fn builtin_argv(
    suffix: &str,
    file: &Path,
    extras: &[OsString],
    ctx: &Ctx<'_>,
) -> Result<Vec<OsString>, Error> {
    let path = file.to_string_lossy();
    let extra = owned_args(extras);
    let extra_refs = refs_of(&extra);
    let Some(cmd) = script_builder(suffix, &path, &extra_refs) else {
        return Err(Error::not_found(suffix));
    };
    authorize_builder(cmd, ctx)
}

fn script_builder(suffix: &str, path: &str, args: &[&str]) -> Option<neotales_exec::Command> {
    Some(match suffix {
        ".sh" => shell_script(path, args),
        ".ps1" => powershell_script(path, args),
        ".ts" | ".tsx" => neotales_exec_script::bun::file(path, args.iter().copied()),
        ".js" | ".mjs" | ".cjs" => javascript_script(path, args),
        ".go" => neotales_exec_script::golang::file(path, args.iter().copied()),
        ".cs" | ".csx" | ".dll" | ".exe" | ".csproj" | ".fsproj" | ".vbproj" => {
            neotales_exec_script::dotnet::file(path, args.iter().copied())
        }
        ".py" => python_script(path, args),
        ".rb" => neotales_exec_script::ruby::file(path, args.iter().copied()),
        ".nu" => neotales_exec_script::nushell::file(path, args.iter().copied()),
        ".exs" => neotales_exec_script::elixir::file(path, args.iter().copied()),
        ".lua" => neotales_exec_script::lua::file(path, args.iter().copied()),
        ".php" => neotales_exec_script::php::file(path, args.iter().copied()),
        ".tcl" => neotales_exec_script::tcl::file(path, args.iter().copied()),
        ".fish" => neotales_exec_script::fish::file(path, args.iter().copied()),
        ".groovy" => neotales_exec_script::groovy::file(path, args.iter().copied()),
        ".awk" => neotales_exec_script::awk::file(path, args.iter().copied()),
        ".dart" => neotales_exec_script::dart::file(path, args.iter().copied()),
        ".jl" => neotales_exec_script::julia::file(path, args.iter().copied()),
        ".pl" => neotales_exec_script::perl::file(path, args.iter().copied()),
        ".rkt" => neotales_exec_script::racket::file(path, args.iter().copied()),
        ".zsh" => neotales_exec_script::zsh::file(path, args.iter().copied()),
        ".bat" | ".cmd" => neotales_exec_script::cmd::file(path, args.iter().copied()),
        ".r" => neotales_exec_script::r::file(path, args.iter().copied()),
        _ => return None,
    })
}

/// `find` only accepts a registered absolute candidate. A bare name left by the
/// builder is resolved here, so trust still refuses a PATH-only hit.
fn authorize_builder(cmd: neotales_exec::Command, ctx: &Ctx<'_>) -> Result<Vec<OsString>, Error> {
    let resolved = authorize_runtime(cmd.program(), ctx)?;
    let mut argv = vec![resolved.into()];
    argv.extend(cmd.arguments().iter().cloned());
    Ok(argv)
}

fn authorize_runtime(program: &OsStr, ctx: &Ctx<'_>) -> Result<PathBuf, Error> {
    let path = Path::new(program);
    if path.is_absolute() {
        if path.is_file() {
            return Ok(path.to_path_buf());
        }
        return Err(Error::not_found(&program.to_string_lossy()));
    }
    resolve::resolve_program(program, ctx.finder, ctx.cwd, ctx.trust_on)
}

fn runtime_resolved(cmd: &neotales_exec::Command, name: &str) -> bool {
    let program = cmd.program();
    program != OsStr::new(name) && Path::new(program).is_absolute()
}

fn shell_script(path: &str, args: &[&str]) -> neotales_exec::Command {
    let bash = neotales_exec_script::bash::file(path, args.iter().copied());
    if runtime_resolved(&bash, "bash") || neotales_exec::find("bash", None).is_ok() {
        bash
    } else {
        neotales_exec_script::sh::file(path, args.iter().copied())
    }
}

const PS_FLAGS: [&str; 1] = ["-NoLogo"];

fn powershell_script(path: &str, args: &[&str]) -> neotales_exec::Command {
    let pwsh =
        neotales_exec_script::pwsh::file_with_shell_args(path, PS_FLAGS, args.iter().copied());
    if runtime_resolved(&pwsh, "pwsh") || neotales_exec::find("pwsh", None).is_ok() {
        pwsh
    } else {
        neotales_exec_script::powershell::file_with_shell_args(path, PS_FLAGS, args.iter().copied())
    }
}

fn python_script(path: &str, args: &[&str]) -> neotales_exec::Command {
    let python = neotales_exec_script::python::file(path, args.iter().copied());
    if runtime_resolved(&python, "python") || neotales_exec::find("python", None).is_ok() {
        python
    } else {
        neotales_exec_script::uv::file(path, args.iter().copied())
    }
}

fn javascript_script(path: &str, args: &[&str]) -> neotales_exec::Command {
    let bun = neotales_exec_script::bun::file(path, args.iter().copied());
    if runtime_resolved(&bun, "bun") || neotales_exec::find("bun", None).is_ok() {
        bun
    } else {
        neotales_exec_script::node::file(path, args.iter().copied())
    }
}

fn runtime_name(interpreter: &str) -> Option<&'static str> {
    let base = Path::new(interpreter)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(interpreter);
    let base = base.strip_suffix(".exe").unwrap_or(base);
    match base {
        "bash" => Some("bash"),
        "sh" => Some("sh"),
        "zsh" => Some("zsh"),
        "pwsh" => Some("pwsh"),
        "powershell" => Some("powershell"),
        "python" | "python3" => Some("python"),
        "ruby" => Some("ruby"),
        "node" => Some("node"),
        "bun" => Some("bun"),
        "deno" => Some("deno"),
        "nu" => Some("nu"),
        "elixir" => Some("elixir"),
        "uv" => Some("uv"),
        "lua" => Some("lua"),
        "php" => Some("php"),
        "tclsh" => Some("tclsh"),
        "fish" => Some("fish"),
        "groovy" => Some("groovy"),
        "awk" => Some("awk"),
        "dart" => Some("dart"),
        "julia" => Some("julia"),
        "perl" => Some("perl"),
        "racket" => Some("racket"),
        "Rscript" | "rscript" => Some("r"),
        _ => None,
    }
}

fn runtime_shebang(
    name: &str,
    script: &str,
    shell_args: &[&str],
    extras: &[&str],
) -> Option<neotales_exec::Command> {
    use neotales_exec_script::{
        awk, bash, bun, dart, deno, elixir, fish, groovy, julia, lua, node, nushell, perl, php,
        powershell, pwsh, python, r, racket, ruby, sh, tcl, uv, zsh,
    };
    let cmd = match name {
        "bash" if shell_args.is_empty() => bash::file(script, extras.iter().copied()),
        "bash" => {
            bash::file_with_shell_args(script, shell_args.iter().copied(), extras.iter().copied())
        }
        "sh" if shell_args.is_empty() => sh::file(script, extras.iter().copied()),
        "sh" => {
            sh::file_with_shell_args(script, shell_args.iter().copied(), extras.iter().copied())
        }
        "zsh" if shell_args.is_empty() => zsh::file(script, extras.iter().copied()),
        "zsh" => {
            zsh::file_with_shell_args(script, shell_args.iter().copied(), extras.iter().copied())
        }
        "pwsh" if shell_args.is_empty() => {
            pwsh::file_with_shell_args(script, PS_FLAGS, extras.iter().copied())
        }
        "pwsh" => {
            pwsh::file_with_shell_args(script, shell_args.iter().copied(), extras.iter().copied())
        }
        "powershell" if shell_args.is_empty() => {
            powershell::file_with_shell_args(script, PS_FLAGS, extras.iter().copied())
        }
        "powershell" => powershell::file_with_shell_args(
            script,
            shell_args.iter().copied(),
            extras.iter().copied(),
        ),
        "deno" if shell_args.is_empty() => deno::file(script, extras.iter().copied()),
        "deno" => {
            deno::file_with_shell_args(script, shell_args.iter().copied(), extras.iter().copied())
        }
        "python" if shell_args.is_empty() => python::file(script, extras.iter().copied()),
        "python" => python::new(std::iter::empty())
            .args(shell_args.iter().copied())
            .arg(script)
            .args(extras.iter().copied()),
        "node" if shell_args.is_empty() => node::file(script, extras.iter().copied()),
        "node" => node::new(std::iter::empty())
            .args(shell_args.iter().copied())
            .arg(script)
            .args(extras.iter().copied()),
        "bun" if shell_args.is_empty() => bun::file(script, extras.iter().copied()),
        "bun" => bun::new(std::iter::empty())
            .args(shell_args.iter().copied())
            .arg(script)
            .args(extras.iter().copied()),
        "ruby" if shell_args.is_empty() => ruby::file(script, extras.iter().copied()),
        "ruby" => ruby::new(std::iter::empty())
            .args(shell_args.iter().copied())
            .arg(script)
            .args(extras.iter().copied()),
        "nu" if shell_args.is_empty() => nushell::file(script, extras.iter().copied()),
        "nu" => nushell::new(std::iter::empty())
            .args(shell_args.iter().copied())
            .arg(script)
            .args(extras.iter().copied()),
        "elixir" if shell_args.is_empty() => elixir::file(script, extras.iter().copied()),
        "elixir" => elixir::new(std::iter::empty())
            .args(shell_args.iter().copied())
            .arg(script)
            .args(extras.iter().copied()),
        "uv" if shell_args.is_empty() => uv::file(script, extras.iter().copied()),
        "uv" => uv::new(std::iter::empty())
            .args(shell_args.iter().copied())
            .arg(script)
            .args(extras.iter().copied()),
        "lua" if shell_args.is_empty() => lua::file(script, extras.iter().copied()),
        "lua" => lua::new(std::iter::empty())
            .args(shell_args.iter().copied())
            .arg(script)
            .args(extras.iter().copied()),
        "php" if shell_args.is_empty() => php::file(script, extras.iter().copied()),
        "php" => php::new(std::iter::empty())
            .args(shell_args.iter().copied())
            .arg(script)
            .args(extras.iter().copied()),
        "tclsh" if shell_args.is_empty() => tcl::file(script, extras.iter().copied()),
        "tclsh" => tcl::new(std::iter::empty())
            .args(shell_args.iter().copied())
            .arg(script)
            .args(extras.iter().copied()),
        "fish" if shell_args.is_empty() => fish::file(script, extras.iter().copied()),
        "fish" => fish::new(std::iter::empty())
            .args(shell_args.iter().copied())
            .arg(script)
            .args(extras.iter().copied()),
        "groovy" if shell_args.is_empty() => groovy::file(script, extras.iter().copied()),
        "groovy" => groovy::new(std::iter::empty())
            .args(shell_args.iter().copied())
            .arg(script)
            .args(extras.iter().copied()),
        "awk" if shell_args.is_empty() => awk::file(script, extras.iter().copied()),
        "awk" => awk::new(std::iter::empty())
            .args(shell_args.iter().copied())
            .arg(script)
            .args(extras.iter().copied()),
        "dart" if shell_args.is_empty() => dart::file(script, extras.iter().copied()),
        "dart" => dart::new(std::iter::empty())
            .args(shell_args.iter().copied())
            .arg(script)
            .args(extras.iter().copied()),
        "julia" if shell_args.is_empty() => julia::file(script, extras.iter().copied()),
        "julia" => julia::new(std::iter::empty())
            .args(shell_args.iter().copied())
            .arg(script)
            .args(extras.iter().copied()),
        "perl" if shell_args.is_empty() => perl::file(script, extras.iter().copied()),
        "perl" => perl::new(std::iter::empty())
            .args(shell_args.iter().copied())
            .arg(script)
            .args(extras.iter().copied()),
        "racket" if shell_args.is_empty() => racket::file(script, extras.iter().copied()),
        "racket" => racket::new(std::iter::empty())
            .args(shell_args.iter().copied())
            .arg(script)
            .args(extras.iter().copied()),
        "r" if shell_args.is_empty() => r::file(script, extras.iter().copied()),
        "r" => r::new(std::iter::empty())
            .args(shell_args.iter().copied())
            .arg(script)
            .args(extras.iter().copied()),
        _ => return None,
    };
    Some(cmd)
}

fn owned_args(args: &[OsString]) -> Vec<String> {
    args.iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn refs_of(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
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

fn open_target(
    target: &OsStr,
    detach: bool,
    ctx: &Ctx<'_>,
    new_instance: bool,
) -> Result<i32, Error> {
    if cli::is_url(target) && !ctx.config.urls_enabled() {
        return Err(Error::new(1, "URL opening is disabled"));
    }
    if let Some(opener) = ctx.opener {
        return spawn(&[opener.to_os_string(), target.to_os_string()], ctx, detach);
    }
    let facts = OpenerFacts {
        host: host(),
        wsl: wsl_detected(),
        wsl_xdg_open: ctx.config.wsl_xdg_open_on(),
        wslview_found: ctx.finder.lookup("wslview").is_some(),
        directory: Path::new(target).is_dir(),
        new_instance,
    };
    // Linux: xdg-open. macOS: open. Windows documents and URLs: ShellExecuteW.
    // `cmd /c start` splits on `&` and treats the first quoted string as a title.
    match choose_opener(&facts) {
        OpenerChoice::ShellExecute => {
            #[cfg(windows)]
            {
                shell_execute(target)?;
                return Ok(0);
            }
            #[cfg(not(windows))]
            Err(Error::new(1, "ShellExecuteW is only available on Windows"))
        }
        OpenerChoice::OpenNew => {
            let open = resolve::resolve_lenient("open", ctx.finder, ctx.cwd)?;
            let bundle = resolve_app(target, ctx).unwrap_or_else(|| PathBuf::from(target));
            // `open -n` returns immediately. Do not setsid the bundle.
            spawn(
                &[
                    open.into(),
                    OsString::from("-n"),
                    bundle.as_os_str().to_os_string(),
                ],
                ctx,
                false,
            )
        }
        OpenerChoice::XdgOpen => spawn_opener("xdg-open", target, detach, ctx),
        OpenerChoice::Open => spawn_opener("open", target, detach, ctx),
        OpenerChoice::WslView => spawn_opener("wslview", target, detach, ctx),
        OpenerChoice::Explorer => spawn_opener("explorer.exe", target, detach, ctx),
    }
}

fn spawn_opener(name: &str, target: &OsStr, detach: bool, ctx: &Ctx<'_>) -> Result<i32, Error> {
    let program = resolve::resolve_lenient(name, ctx.finder, ctx.cwd)?;
    spawn(&[program.into(), target.to_os_string()], ctx, detach)
}

/// Which platform tool opens a document, directory, or URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenerChoice {
    XdgOpen,
    Open,
    OpenNew,
    WslView,
    Explorer,
    ShellExecute,
}

#[derive(Clone, Copy, Debug)]
struct OpenerFacts {
    host: HostOs,
    wsl: bool,
    wsl_xdg_open: bool,
    wslview_found: bool,
    directory: bool,
    new_instance: bool,
}

/// Pick the opener from host, WSL, and `[open] wsl-xdg-open`. Outside WSL the key is ignored.
fn choose_opener(facts: &OpenerFacts) -> OpenerChoice {
    if facts.host == HostOs::Macos && facts.new_instance {
        return OpenerChoice::OpenNew;
    }
    if facts.wsl {
        if facts.wsl_xdg_open {
            return OpenerChoice::XdgOpen;
        }
        return if facts.wslview_found {
            OpenerChoice::WslView
        } else {
            OpenerChoice::Explorer
        };
    }
    match facts.host {
        HostOs::Windows if facts.directory => OpenerChoice::Explorer,
        HostOs::Windows => OpenerChoice::ShellExecute,
        HostOs::Macos => OpenerChoice::Open,
        HostOs::Linux => OpenerChoice::XdgOpen,
    }
}

fn wsl_detected() -> bool {
    std::env::var_os("WSL_DISTRO_NAME").is_some_and(|value| !value.is_empty())
        || Path::new("/proc/sys/fs/binfmt_misc/WSLInterop").exists()
}

fn resolve_app(target: &OsStr, ctx: &Ctx<'_>) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let path = cli::resolve_path(ctx.cwd, target);
    if is_app_dir(&path) {
        return Some(path);
    }
    if !cli::is_unqualified(target) {
        return None;
    }
    let name = target.to_str()?;
    let stem = name.strip_suffix(".app").unwrap_or(name);
    let mut candidates = vec![
        ctx.cwd.join(format!("{stem}.app")),
        PathBuf::from("/Applications").join(format!("{stem}.app")),
    ];
    if let Some(home) = ctx.env.get("HOME") {
        candidates.push(
            PathBuf::from(home)
                .join("Applications")
                .join(format!("{stem}.app")),
        );
    }
    candidates.into_iter().find(|path| is_app_dir(path))
}

fn is_app_dir(path: &Path) -> bool {
    path.is_dir()
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
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

/// File name of argv0, without a trailing `.exe`. Missing or empty names fall back to `vrun`.
fn invocation_name(argv0: Option<&OsStr>) -> String {
    let Some(argv0) = argv0 else {
        return "vrun".to_string();
    };
    let Some(file) = Path::new(argv0).file_name().and_then(|name| name.to_str()) else {
        return "vrun".to_string();
    };
    let lower = file.to_ascii_lowercase();
    let name = lower.strip_suffix(".exe").unwrap_or(&lower);
    if name.is_empty() {
        "vrun".to_string()
    } else {
        name.to_string()
    }
}

fn help_text(name: &str) -> String {
    format!(
        "\
vaelrun {version} — cross-platform run, open, and start

Usage:
  {name} [options] [NAME=value]... [dotenv]... <command> [args]...
  {name} [options] open <target>
  {name} [options] start <command> [args]...
  {name} [options] run <command> [args]...
  {name} [options] -e <path>
  {name} complete bash|zsh|fish|pwsh

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
",
        version = env!("CARGO_PKG_VERSION"),
    )
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;

    use super::{
        HostOs, OpenerChoice, OpenerFacts, choose_opener, help_text, invocation_name, is_builtin,
        script_builder,
    };
    use crate::cli::extension_suffix;

    #[test]
    fn remaining_script_suffixes_use_exec_script() {
        for suffix in [
            ".nu", ".exs", ".lua", ".php", ".tcl", ".fish", ".groovy", ".awk", ".dart", ".jl",
            ".pl", ".rkt", ".zsh", ".bat", ".cmd", ".r",
        ] {
            assert!(is_builtin(suffix), "{suffix}");
        }
        assert_eq!(
            extension_suffix(OsStr::new("plot.R")).as_deref(),
            Some(".r")
        );
        let ps1 = script_builder(".ps1", "app.ps1", &["-c", "Release"]).unwrap();
        let flags: Vec<String> = ps1
            .arguments()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(flags.iter().any(|arg| arg == "-NoLogo"), "{flags:?}");
        assert!(flags.iter().any(|arg| arg == "-File"), "{flags:?}");
        assert!(
            flags.iter().all(|arg| !arg.contains("ExecutionPolicy")),
            "{flags:?}"
        );
        assert!(!flags.iter().any(|arg| arg == "-NoProfile"), "{flags:?}");

        let cmd = script_builder(".nu", "app.nu", &[]).unwrap();
        let program = cmd.program().to_string_lossy();
        let name = Path::new(program.as_ref())
            .file_name()
            .unwrap()
            .to_string_lossy();
        assert!(name == "nu" || name == "nu.exe", "{program}");
    }

    #[test]
    fn wsl_opener_follows_the_config_flag() {
        let mut facts = OpenerFacts {
            host: HostOs::Linux,
            wsl: true,
            wsl_xdg_open: true,
            wslview_found: true,
            directory: false,
            new_instance: false,
        };
        assert_eq!(choose_opener(&facts), OpenerChoice::XdgOpen);
        facts.wsl_xdg_open = false;
        assert_eq!(choose_opener(&facts), OpenerChoice::WslView);
        facts.wslview_found = false;
        assert_eq!(choose_opener(&facts), OpenerChoice::Explorer);
        facts.wsl = false;
        facts.wsl_xdg_open = true;
        assert_eq!(choose_opener(&facts), OpenerChoice::XdgOpen);
        facts.host = HostOs::Macos;
        assert_eq!(choose_opener(&facts), OpenerChoice::Open);
        facts.host = HostOs::Windows;
        assert_eq!(choose_opener(&facts), OpenerChoice::ShellExecute);
        facts.directory = true;
        assert_eq!(choose_opener(&facts), OpenerChoice::Explorer);
    }

    #[test]
    fn help_names_the_product_and_the_invoked_file() {
        let version = env!("CARGO_PKG_VERSION");
        let text = help_text("vr");
        assert!(text.starts_with(&format!("vaelrun {version} — ")), "{text}");
        assert!(
            text.lines()
                .any(|line| line.trim_start().starts_with("vr "))
        );
        assert!(!text.contains("vrun"));

        assert_eq!(invocation_name(Some(OsStr::new("vr.exe"))), "vr");
        assert_eq!(invocation_name(Some(OsStr::new("VR.EXE"))), "vr");
        assert_eq!(invocation_name(Some(OsStr::new("/usr/local/bin/vr"))), "vr");
        assert_eq!(invocation_name(None), "vrun");
        assert!(!help_text("vr").contains("vr.exe"));
    }
}
