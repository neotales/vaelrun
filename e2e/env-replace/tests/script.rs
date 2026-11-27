use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Once;

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

/// Installs missing hosts once, then puts mise's shim directory on `PATH`.
fn prepare() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // uv must not fetch another Python. The dotnet CLI must stay quiet.
        set_env("UV_PYTHON_PREFERENCE", "only-system");
        set_env("DOTNET_CLI_TELEMETRY_OPTOUT", "1");
        set_env("DOTNET_NOLOGO", "1");
        if !on_path("mise") {
            panic!("mise is required");
        }
        let root = repo_root();
        let config = root.join("mise.toml");
        for (bin, tool) in TOOLS {
            if !on_path(bin) {
                mise_install(&root, &config, tool);
            }
        }
        let shims = shim_dir();
        prepend_path(&shims);
        for (bin, _) in TOOLS {
            check_version(bin);
        }
    });
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
    expect_success(&output, "mise doctor");
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
/// `prepare` is the only caller, and its `Once` keeps the other script tests from
/// reading the environment until these writes finish.
fn set_env(key: &str, value: impl AsRef<std::ffi::OsStr>) {
    // SAFETY: see the function comment. No other thread in this binary writes
    // the environment, and the other script tests are blocked in `call_once`.
    unsafe {
        std::env::set_var(key, value);
    }
}

fn on_path(name: &str) -> bool {
    !matches!(
        Command::new(name).arg("--version").output(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound
    )
}

fn check_version(bin: &str) {
    let mut cmd = Command::new(bin);
    if bin == "go" {
        cmd.arg("version");
    } else {
        cmd.arg("--version");
    }
    let output = cmd
        .output()
        .unwrap_or_else(|err| panic!("{bin} version check failed: {err}"));
    if !output.status.success() {
        panic!(
            "{bin} version check failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
