use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

fn vrun() -> &'static str {
    env!("CARGO_BIN_EXE_vrun")
}

fn write_exe(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

#[test]
fn complete_bash_mentions_vrun() {
    let out = Command::new(vrun())
        .args(["complete", "bash"])
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("vrun"), "{text}");
}

#[test]
fn unknown_shell_is_usage() {
    let out = Command::new(vrun())
        .args(["complete", "powershell"])
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn child_exit_code_is_propagated() {
    let out = Command::new(vrun())
        .args(["/bin/sh", "-c", "exit 42"])
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(42),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn trust_refuses_path_only_and_allows_absolute() {
    let refused = Command::new(vrun())
        .args(["true", "-c", "echo hi"])
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(126));
    let err = String::from_utf8(refused.stderr).unwrap();
    assert!(err.contains("true"), "{err}");
    assert!(err.to_ascii_lowercase().contains("trust"), "{err}");

    let allowed = Command::new(vrun())
        .args(["/bin/true"])
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        allowed.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&allowed.stderr)
    );
}

#[test]
fn inline_overrides_dotenv_and_parent_reaches_child() {
    let dir = tempfile::tempdir().unwrap();
    let envfile = dir.path().join(".env");
    fs::write(&envfile, "MY_VAR=from-file\nOTHER=from-file\n").unwrap();
    let out = Command::new(vrun())
        .arg("--no-trust")
        .arg("MY_VAR=from-inline")
        .arg(&envfile)
        .args([
            "/bin/sh",
            "-c",
            "printf '%s\\n%s\\n%s' \"$MY_VAR\" \"$OTHER\" \"$PARENT_VAR\"",
        ])
        .env("PARENT_VAR", "from-parent")
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "from-inline\nfrom-file\nfrom-parent"
    );
}

#[test]
fn dotenv_path_is_not_a_command_argument() {
    let dir = tempfile::tempdir().unwrap();
    let envfile = dir.path().join("prod.env");
    fs::write(&envfile, "OTHER=1\n").unwrap();
    let out = Command::new(vrun())
        .arg("--no-trust")
        .arg(&envfile)
        .args(["/bin/sh", "-c", "printf '%s' \"$0\""])
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "/bin/sh");
}

#[test]
fn edit_is_not_an_assignment() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let editor = dir.path().join("editor.sh");
    write_exe(
        &editor,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$#\" > {}\nprintf '%s\\n' \"$1\" >> {}\n",
            log.display(),
            log.display()
        ),
    );
    let target = dir.path().join("my file.cs");
    fs::write(&target, "").unwrap();
    let out = Command::new(vrun())
        .arg("-e")
        .arg(&target)
        .env("EDITOR", &editor)
        .env_remove("VISUAL")
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let logged = fs::read_to_string(&log).unwrap();
    assert_eq!(logged, format!("1\n{}\n", target.display()));
}

#[test]
fn directory_open_uses_the_opener_seam() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("folder");
    fs::create_dir(&target).unwrap();
    let log = dir.path().join("opened");
    let opener = dir.path().join("opener.sh");
    write_exe(
        &opener,
        &format!("#!/bin/sh\nprintf '%s' \"$1\" > {}\n", log.display()),
    );
    let code = vaelrun::run_with(vaelrun::RunOptions {
        opener: Some(opener.as_os_str().to_os_string()),
        args: Some(vec!["vrun".into(), target.as_os_str().to_os_string()]),
    })
    .unwrap();
    assert_eq!(code, 0);
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        target.display().to_string()
    );
}

#[test]
fn vrun_script_and_or_and_pipe() {
    let dir = tempfile::tempdir().unwrap();
    let ok = dir.path().join("ok.vrun");
    fs::write(&ok, "/bin/true && /bin/false || /bin/true\n").unwrap();
    let out = Command::new(vrun())
        .arg(&ok)
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let fail = dir.path().join("fail.vrun");
    fs::write(&fail, "/bin/false && /bin/true\n").unwrap();
    let out = Command::new(vrun())
        .arg(&fail)
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let piped = dir.path().join("pipe.vrun");
    fs::write(&piped, "/usr/bin/printf hello | /bin/cat\n").unwrap();
    let out = Command::new(vrun())
        .arg(&piped)
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "hello");
}

#[test]
fn sudo_prefixes_each_pipeline_stage() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let log = dir.path().join("argv");
    let sudo = bin.join("sudo");
    let log_disp = log.display().to_string();
    write_exe(
        &sudo,
        &format!(
            "#!/bin/sh\nexec 9>>'{log_disp}.lock'\nflock 9\n{{\nprintf '%s' \"$0\"\nfor arg in \"$@\"; do printf ' %s' \"$arg\"; done\nprintf '\\n'\n}} >> '{log_disp}'\nexit 0\n"
        ),
    );
    let cfg = dir.path().join("config.ini");
    fs::write(&cfg, format!("[bin]\n{}\n", bin.display())).unwrap();
    let out_file = dir.path().join("out.txt");
    let script = dir.path().join("pipe.vrun");
    fs::write(
        &script,
        format!(
            "/usr/bin/printf hello | /bin/cat\n/bin/true > {}\n",
            out_file.display()
        ),
    )
    .unwrap();
    let out = Command::new(vrun())
        .arg("--sudo")
        .arg(&script)
        .env("VAELRUN_CONFIG", &cfg)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let logged = fs::read_to_string(&log).unwrap();
    let lines: Vec<_> = logged.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(lines.len(), 3, "{logged}");
    let sudo_disp = sudo.display().to_string();
    for line in lines {
        assert!(line.starts_with(&sudo_disp), "{line}");
    }
}

#[test]
fn sudo_trust_refusal_on_a_pipeline_is_126() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("pipe.vrun");
    fs::write(&script, "/bin/true | /bin/true\n").unwrap();
    let out = Command::new(vrun())
        .arg("--sudo")
        .arg(&script)
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(126),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("sudo"), "{err}");
    assert!(err.to_ascii_lowercase().contains("trust"), "{err}");
}

#[test]
fn pass_through_injector_expands_single_quotes() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.ini");
    fs::write(&cfg, "[secrets]\ninjector = /bin/cat\n").unwrap();
    let envfile = dir.path().join("file.env");
    fs::write(&envfile, "HOME=/from/file\nLIT='${HOME}'\nVAR=${HOME}\n").unwrap();
    let out = Command::new(vrun())
        .arg(&envfile)
        .args(["/bin/sh", "-c", "printf '%s|%s' \"$LIT\" \"$VAR\""])
        .env("VAELRUN_CONFIG", &cfg)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "/from/file|/from/file"
    );
}

#[test]
fn dotenv_expands_braced_refs_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let envfile = dir.path().join(".env");
    fs::write(&envfile, "BAR=there\nFOO=${BAR}\n").unwrap();
    let out = Command::new(vrun())
        .arg(&envfile)
        .args(["/bin/sh", "-c", "printf '%s' \"$FOO\""])
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "there");
}

#[test]
fn single_quoted_dotenv_value_expands() {
    let dir = tempfile::tempdir().unwrap();
    let envfile = dir.path().join(".env");
    fs::write(&envfile, "BAR=there\nFOO='${BAR}'\n").unwrap();
    let out = Command::new(vrun())
        .arg(&envfile)
        .args(["/bin/sh", "-c", "printf '%s' \"$FOO\""])
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "there");
}

#[test]
fn injector_replaces_env_and_still_expands() {
    let dir = tempfile::tempdir().unwrap();
    let stdin_log = dir.path().join("stdin.log");
    let injector = dir.path().join("injector.sh");
    write_exe(
        &injector,
        &format!(
            "#!/bin/sh\ncat > '{log}'\ncat <<'END'\nHOME=/from-injector\nEXPANDED=${{HOME}}\nLITERAL='${{HOME}}'\nEND\n",
            log = stdin_log.display()
        ),
    );
    let cfg = dir.path().join("config.ini");
    fs::write(
        &cfg,
        format!("[secrets]\ninjector = {}\n", injector.display()),
    )
    .unwrap();
    let envfile = dir.path().join("file.env");
    fs::write(&envfile, "FILE_VAR=from-file\n").unwrap();
    let out = Command::new(vrun())
        .arg("INLINE_VAR=from-inline")
        .arg(&envfile)
        .args([
            "/bin/sh",
            "-c",
            "printf '%s|%s|%s' \"$EXPANDED\" \"$LITERAL\" \"$FILE_VAR\"",
        ])
        .env("PARENT_VAR", "from-parent")
        .env("VAELRUN_CONFIG", &cfg)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "/from-injector|/from-injector|"
    );
    let seen = fs::read_to_string(&stdin_log).unwrap();
    let parent = seen.find("PARENT_VAR=from-parent").expect(&seen);
    let file = seen.find("FILE_VAR=from-file").expect(&seen);
    let inline = seen.find("INLINE_VAR=from-inline").expect(&seen);
    assert!(parent < file && file < inline, "{seen}");
}

#[test]
fn injector_failure_does_not_run_the_child() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran");
    let injector = dir.path().join("injector.sh");
    write_exe(&injector, "#!/bin/sh\nprintf 'SECRET-CANARY'\nexit 2\n");
    let cfg = dir.path().join("config.ini");
    fs::write(
        &cfg,
        format!("[secrets]\ninjector = {}\n", injector.display()),
    )
    .unwrap();
    let out = Command::new(vrun())
        .args([
            "/bin/sh",
            "-c",
            &format!("printf x > '{}'", marker.display()),
        ])
        .env("VAELRUN_CONFIG", &cfg)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!marker.exists());
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains(&injector.display().to_string()), "{err}");
    assert!(err.contains("2"), "{err}");
    assert!(!err.contains("SECRET-CANARY"), "{err}");
}

#[test]
fn trust_refuses_path_only_injector() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let marker = dir.path().join("ran");
    write_exe(&bin.join("fakeinject"), "#!/bin/sh\nexit 0\n");
    let cfg = dir.path().join("config.ini");
    fs::write(&cfg, "[secrets]\ninjector = fakeinject\n").unwrap();
    let path = std::env::var("PATH").unwrap_or_default();
    let out = Command::new(vrun())
        .args([
            "/bin/sh",
            "-c",
            &format!("printf x > '{}'", marker.display()),
        ])
        .env("PATH", format!("{}:{path}", bin.display()))
        .env("VAELRUN_CONFIG", &cfg)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(126),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("fakeinject"), "{err}");
    assert!(err.to_ascii_lowercase().contains("trust"), "{err}");
    assert!(!marker.exists());
}
