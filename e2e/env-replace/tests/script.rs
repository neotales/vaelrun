use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Mutex;

use vaelrun::RunOptions;

/// Bare name the tests look up, and the mise tool id that installs it.
const TOOLS: &[(&str, &str)] = &[
    ("python", "python"),
    ("node", "node"),
    ("bun", "bun"),
    ("deno", "deno"),
    ("go", "go"),
    ("uv", "uv"),
    ("dotnet", "dotnet"),
    ("pwsh", "powershell"),
    ("bash", "bash"),
];

const PYTHON: &str = "import sys\nopen(sys.argv[1], 'w').write('from-python')\n";
const NODE: &str =
    "#!/usr/bin/env node\nrequire('fs').writeFileSync(process.argv[2], 'from-node')\n";
const BUN: &str = "require('fs').writeFileSync(process.argv[2], 'from-bun')\n";
const DENO: &str = "#!/usr/bin/env deno\nDeno.writeTextFileSync(Deno.args[0], 'from-deno')\n";
const GO: &str = r#"package main

import "os"

func main() {
	os.WriteFile(os.Args[1], []byte("from-go"), 0o644)
}
"#;
const GO_MOD: &str = "module example\n\ngo 1.22\n";
const DOTNET: &str = "System.IO.File.WriteAllText(args[0], \"from-dotnet\");\n";
const BASH: &str = "printf '%s' 'from-bash' > \"$1\"\n";
const PWSH: &str = "[System.IO.File]::WriteAllText($args[0], 'from-pwsh')\n";
const UV: &str = "#!/usr/bin/env uv\nimport sys\nopen(sys.argv[1], 'w').write('from-uv')\n";

#[test]
fn python_script_runs_through_vrun() {
    run_host("hello.py", PYTHON, "from-python");
}

#[test]
fn node_script_runs_through_vrun() {
    run_host("hello.js", NODE, "from-node");
}

#[test]
fn bun_script_runs_through_vrun() {
    run_host("hello.ts", BUN, "from-bun");
}

#[test]
fn deno_script_runs_through_vrun() {
    run_host("hello.js", DENO, "from-deno");
}

#[test]
fn go_script_runs_through_vrun() {
    prepare();
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran.txt");
    let script = dir.path().join("main.go");
    std::fs::write(&script, GO).unwrap();
    std::fs::write(dir.path().join("go.mod"), GO_MOD).unwrap();
    run_marker(&script, &marker, "from-go");
}

#[test]
fn dotnet_script_runs_through_vrun() {
    run_host("app.cs", DOTNET, "from-dotnet");
}

#[test]
fn bash_script_runs_through_vrun() {
    run_host("hello.sh", BASH, "from-bash");
}

#[test]
fn pwsh_script_runs_through_vrun() {
    run_host("hello.ps1", PWSH, "from-pwsh");
}

#[test]
fn uv_script_runs_through_vrun() {
    run_host("hello.py", UV, "from-uv");
}

fn run_host(name: &str, source: &str, text: &str) {
    prepare();
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran.txt");
    let script = dir.path().join(name);
    std::fs::write(&script, source).unwrap();
    run_marker(&script, &marker, text);
}

fn run_marker(script: &Path, marker: &Path, text: &str) {
    // --no-trust: mise shims are PATH hits, and trust would refuse them.
    let code = vaelrun::run_with(RunOptions {
        args: Some(vec![
            "vrun".into(),
            "--no-trust".into(),
            script.as_os_str().to_os_string(),
            marker.as_os_str().to_os_string(),
        ]),
        ..Default::default()
    })
    .unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(code, 0, "vrun status {code}");
    assert_eq!(std::fs::read_to_string(marker).unwrap(), text);
}

/// Installs a missing host once. Hosts already on `PATH` skip mise entirely.
fn prepare() {
    static SLOT: Mutex<Option<Result<(), String>>> = Mutex::new(None);
    let mut slot = SLOT.lock().unwrap_or_else(|err| err.into_inner());
    if let Some(result) = slot.clone() {
        drop(slot);
        if let Err(message) = result {
            panic!("{message}");
        }
        return;
    }
    let stored = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(prepare_hosts)) {
        Ok(()) => Ok(()),
        Err(payload) => Err(panic_message(payload)),
    };
    let failed = stored.clone().err();
    *slot = Some(stored);
    drop(slot);
    if let Some(message) = failed {
        panic!("{message}");
    }
}

fn prepare_hosts() {
    // uv must not fetch another Python. The dotnet CLI must stay quiet.
    set_env("UV_PYTHON_PREFERENCE", "only-system");
    set_env("DOTNET_CLI_TELEMETRY_OPTOUT", "1");
    set_env("DOTNET_NOLOGO", "1");
    prefer_git_bash();
    let missing: Vec<&str> = TOOLS
        .iter()
        .filter(|(bin, _)| !on_path(bin))
        .map(|(_, tool)| *tool)
        .collect();
    // mise has no bash tool. A missing bash stays missing.
    let installable: Vec<&str> = missing.into_iter().filter(|tool| *tool != "bash").collect();
    if installable.is_empty() {
        for (bin, _) in TOOLS {
            check_version(bin);
        }
        return;
    }
    if !on_path("mise") {
        panic!("mise is required");
    }
    let root = repo_root();
    let config = root.join("mise.toml");
    for tool in installable {
        mise_install(&root, &config, tool);
    }
    prepend_path(&shim_dir());
    for (bin, _) in TOOLS {
        check_version(bin);
    }
}

/// `System32\bash.exe` is the WSL launcher and fails `--version`. Git bash does not.
#[cfg(windows)]
fn prefer_git_bash() {
    if on_path("bash") {
        return;
    }
    let dir = Path::new(r"C:\Program Files\Git\bin");
    if dir.join("bash.exe").is_file() {
        prepend_path(dir);
    }
}

#[cfg(not(windows))]
fn prefer_git_bash() {}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    "script host setup failed".to_string()
}

fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if dir.join("mise.toml").is_file() {
            return dir;
        }
        if !dir.pop() {
            panic!("mise.toml not found above {}", env!("CARGO_MANIFEST_DIR"));
        }
    }
}

fn mise_install(root: &Path, config: &Path, tool: &str) {
    let output = mise_at(root, &["install", tool]);
    if untrusted(&output) {
        trust_config(config);
        expect_success(&mise_at(root, &["install", tool]), "mise install");
        return;
    }
    expect_success(&output, "mise install");
}

fn mise_at(root: &Path, args: &[&str]) -> Output {
    Command::new("mise")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("mise is required: {err}"))
}

fn trust_config(config: &Path) {
    let output = Command::new("mise")
        .arg("trust")
        .arg(config)
        .output()
        .unwrap_or_else(|err| panic!("mise is required: {err}"));
    expect_success(&output, "mise trust");
}

fn untrusted(output: &Output) -> bool {
    !output.status.success() && String::from_utf8_lossy(&output.stderr).contains("not trusted")
}

fn expect_success(output: &Output, what: &str) {
    if output.status.success() {
        return;
    }
    panic!("{what} failed: {}", String::from_utf8_lossy(&output.stderr));
}

/// `mise activate --shims` prints nothing when the shim dir is already first.
/// `mise doctor -J` reports `dirs.shims` without a home path.
fn shim_dir() -> PathBuf {
    let output = Command::new("mise")
        .args(["--no-config", "doctor", "-J"])
        .output()
        .unwrap_or_else(|err| panic!("mise is required: {err}"));
    if !output.status.success() {
        panic!(
            "mise doctor failed: stdout: {} stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let json = String::from_utf8_lossy(&output.stdout);
    let dir = json_path(&json, "shims").unwrap_or_else(|| {
        panic!("mise doctor did not report a shim directory: {json}");
    });
    if !dir.is_dir() {
        panic!("mise shim directory {} is missing", dir.display());
    }
    dir
}

fn json_path(json: &str, key: &str) -> Option<PathBuf> {
    let marker = format!("\"{key}\":");
    let rest = json.get(json.find(&marker)? + marker.len()..)?.trim_start();
    let mut chars = rest.strip_prefix('"')?.chars();
    let mut value = String::new();
    while let Some(c) = chars.next() {
        if c == '"' {
            break;
        }
        if c == '\\' {
            if let Some(next) = chars.next() {
                value.push(next);
            }
            continue;
        }
        value.push(c);
    }
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(value))
    }
}

fn prepend_path(dir: &Path) {
    let mut dirs = vec![dir.to_path_buf()];
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    let joined = std::env::join_paths(dirs).unwrap_or_else(|err| panic!("PATH: {err}"));
    set_env("PATH", joined);
}

/// `set_var` is unsafe because a concurrent environment read is undefined.
///
/// `prepare` is the only caller. Its mutex keeps the other script tests from
/// reading the environment until these writes finish.
fn set_env(key: &str, value: impl AsRef<std::ffi::OsStr>) {
    // SAFETY: see the function comment. No other thread in this binary writes
    // the environment, and the other script tests wait on `prepare`'s mutex.
    unsafe {
        std::env::set_var(key, value);
    }
}

fn on_path(name: &str) -> bool {
    version_output(name).is_ok_and(|output| output.status.success())
}

fn version_output(bin: &str) -> std::io::Result<Output> {
    let mut cmd = Command::new(bin);
    if bin == "go" {
        cmd.arg("version");
    } else {
        cmd.arg("--version");
    }
    cmd.output()
}

fn check_version(bin: &str) {
    let output = version_output(bin).unwrap_or_else(|err| {
        panic!("{bin} version check failed: exit none stdout: stderr: {err}");
    });
    if output.status.success() {
        return;
    }
    let code = output
        .status
        .code()
        .map_or_else(|| "none".to_string(), |code| code.to_string());
    panic!(
        "{bin} version check failed: exit {code} stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
