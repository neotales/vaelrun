#![cfg(unix)]

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
        ..Default::default()
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
fn pass_through_injector_keeps_dotenv_literal() {
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
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "${HOME}|${HOME}");
}

#[test]
fn dotenv_braced_refs_stay_literal() {
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
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "${BAR}");
}

#[test]
fn single_quoted_dotenv_value_stays_literal() {
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
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "${BAR}");
}

#[test]
fn inline_assignment_expands_but_dotenv_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let envfile = dir.path().join("file.env");
    fs::write(&envfile, "MY_VAR=${HOME}\n").unwrap();
    let from_file = Command::new(vrun())
        .arg(&envfile)
        .args(["/bin/sh", "-c", "printf '%s' \"$MY_VAR\""])
        .env("HOME", "/from/shell")
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        from_file.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&from_file.stderr)
    );
    assert_eq!(String::from_utf8(from_file.stdout).unwrap(), "${HOME}");

    let from_argv = Command::new(vrun())
        .arg("MY_VAR=${HOME}")
        .args(["/bin/sh", "-c", "printf '%s' \"$MY_VAR\""])
        .env("HOME", "/from/shell")
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        from_argv.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&from_argv.stderr)
    );
    assert_eq!(String::from_utf8(from_argv.stdout).unwrap(), "/from/shell");
}

#[test]
fn inline_unquoted_dollar_expands() {
    let out = Command::new(vrun())
        .arg("MY_VAR=$HOME")
        .args(["/bin/sh", "-c", "printf '%s' \"$MY_VAR\""])
        .env("HOME", "/from/shell")
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "/from/shell");
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
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "${HOME}|${HOME}|");
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

fn opener_seam(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let log = dir.join("opened");
    let opener = dir.join("opener.sh");
    write_exe(
        &opener,
        &format!("#!/bin/sh\nprintf '%s' \"$1\" > {}\n", log.display()),
    );
    (log, opener)
}

#[test]
fn ps1_shebang_runs_the_interpreter_not_pwsh() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ran");
    let interp = dir.path().join("interp.sh");
    write_exe(
        &interp,
        &format!("#!/bin/sh\nprintf '%s\\n' \"$1\" > {}\n", log.display()),
    );
    let script = dir.path().join("app.ps1");
    fs::write(&script, format!("#!{}\nWrite-Host hi\n", interp.display())).unwrap();
    let out = Command::new(vrun())
        .arg(&script)
        .env_remove("VAELRUN_CONFIG")
        .env_remove("RUN_PWSH_EXE")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(&log).unwrap().trim(),
        script.display().to_string()
    );
}

#[test]
fn extensionless_text_is_opened_not_executed() {
    let dir = tempfile::tempdir().unwrap();
    let notes = dir.path().join("notes");
    fs::write(&notes, "hello\n").unwrap();
    let mut perms = fs::metadata(&notes).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&notes, perms).unwrap();
    let (log, opener) = opener_seam(dir.path());
    let code = vaelrun::run_with(vaelrun::RunOptions {
        opener: Some(opener.into()),
        args: Some(vec!["vrun".into(), notes.as_os_str().to_os_string()]),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(code, 0);
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        notes.display().to_string()
    );
}

#[test]
fn extensionless_nul_executable_is_not_opened() {
    let dir = tempfile::tempdir().unwrap();
    let blob = dir.path().join("blob");
    fs::write(&blob, [0u8, 1, 2, 3]).unwrap();
    let mut perms = fs::metadata(&blob).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&blob, perms).unwrap();
    let (log, opener) = opener_seam(dir.path());
    let result = vaelrun::run_with(vaelrun::RunOptions {
        opener: Some(opener.into()),
        args: Some(vec!["vrun".into(), blob.as_os_str().to_os_string()]),
        ..Default::default()
    });
    assert!(result.is_err(), "a binary should be executed, not opened");
    assert!(!log.exists());
}

#[test]
fn pdf_with_shebang_is_opened_not_interpreted() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("interp");
    let interp = dir.path().join("interp.sh");
    write_exe(
        &interp,
        &format!("#!/bin/sh\nprintf ran > {}\n", log.display()),
    );
    let pdf = dir.path().join("doc.pdf");
    fs::write(&pdf, format!("#!{}\n", interp.display())).unwrap();
    let (opened, opener) = opener_seam(dir.path());
    let code = vaelrun::run_with(vaelrun::RunOptions {
        opener: Some(opener.into()),
        args: Some(vec!["vrun".into(), pdf.as_os_str().to_os_string()]),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(code, 0);
    assert_eq!(
        fs::read_to_string(&opened).unwrap(),
        pdf.display().to_string()
    );
    assert!(!log.exists());
}

#[test]
fn start_hands_a_pdf_to_the_opener() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("doc.pdf");
    fs::write(&pdf, "#!/bin/sh\necho no\n").unwrap();
    let (opened, opener) = opener_seam(dir.path());
    let code = vaelrun::run_with(vaelrun::RunOptions {
        opener: Some(opener.into()),
        args: Some(vec![
            "vrun".into(),
            "start".into(),
            pdf.as_os_str().to_os_string(),
        ]),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(code, 0);
    let logged = wait_file(&opened);
    assert_eq!(logged, pdf.display().to_string());
}

#[test]
fn scripts_suffix_runs_a_shebang_and_opens_without_one() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.ini");
    fs::write(&cfg, "[scripts]\n.txt\n").unwrap();
    let log = dir.path().join("ran");
    let interp = dir.path().join("interp.sh");
    write_exe(
        &interp,
        &format!("#!/bin/sh\nprintf '%s\\n' \"$1\" > {}\n", log.display()),
    );
    let scripted = dir.path().join("note.txt");
    fs::write(&scripted, format!("#!{}\n", interp.display())).unwrap();
    let out = Command::new(vrun())
        .arg(&scripted)
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
        fs::read_to_string(&log).unwrap().trim(),
        scripted.display().to_string()
    );

    let plain = dir.path().join("plain.txt");
    fs::write(&plain, "just text\n").unwrap();
    let (opened, opener) = opener_seam(dir.path());
    let code = vaelrun::run_with(vaelrun::RunOptions {
        opener: Some(opener.into()),
        args: Some(vec!["vrun".into(), plain.as_os_str().to_os_string()]),
        config: Some(cfg),
    })
    .unwrap();
    assert_eq!(code, 0);
    assert_eq!(
        fs::read_to_string(&opened).unwrap(),
        plain.display().to_string()
    );
}

#[test]
fn urls_off_does_not_open_and_urls_on_does() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.ini");
    fs::write(&cfg, "[open]\nurls = off\n").unwrap();
    let (log, opener) = opener_seam(dir.path());
    let err = vaelrun::run_with(vaelrun::RunOptions {
        opener: Some(opener.as_os_str().to_os_string()),
        args: Some(vec![
            "vrun".into(),
            "open".into(),
            "https://example.com".into(),
        ]),
        config: Some(cfg.clone()),
    })
    .unwrap_err();
    assert_eq!(err.code, 1);
    assert!(
        err.message.contains("URL opening is disabled"),
        "{}",
        err.message
    );
    assert!(!log.exists());

    let err = vaelrun::run_with(vaelrun::RunOptions {
        opener: Some(opener.as_os_str().to_os_string()),
        args: Some(vec![
            "vrun".into(),
            "start".into(),
            "https://example.com".into(),
        ]),
        config: Some(cfg.clone()),
    })
    .unwrap_err();
    assert_eq!(err.code, 1);
    assert!(!log.exists());

    fs::write(&cfg, "[open]\nurls = on\n").unwrap();
    let code = vaelrun::run_with(vaelrun::RunOptions {
        opener: Some(opener.into()),
        args: Some(vec![
            "vrun".into(),
            "open".into(),
            "https://example.com".into(),
        ]),
        config: Some(cfg),
    })
    .unwrap();
    assert_eq!(code, 0);
    assert_eq!(fs::read_to_string(&log).unwrap(), "https://example.com");
}

#[test]
fn alias_to_ps1_uses_the_builder_and_appends_args() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let log = dir.path().join("argv");
    let pwsh = bin.join("pwsh");
    write_exe(
        &pwsh,
        &format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\n", log.display()),
    );
    let scripts = dir.path().join("scripts");
    fs::create_dir(&scripts).unwrap();
    let script = scripts.join("build.ps1");
    fs::write(&script, "Write-Host hi\n").unwrap();
    let cfg = dir.path().join("config.ini");
    fs::write(&cfg, "[alias]\nbuild = ./scripts/build.ps1 -c Release\n").unwrap();
    let out = Command::new(vrun())
        .args(["build", "extra"])
        .current_dir(dir.path())
        .env("VAELRUN_CONFIG", &cfg)
        .env("RUN_PWSH_EXE", &pwsh)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let argv = fs::read_to_string(&log).unwrap();
    assert!(argv.contains("-NoLogo"), "{argv}");
    assert!(argv.contains("-File"), "{argv}");
    assert!(argv.contains("build.ps1"), "{argv}");
    assert!(argv.contains("-c"), "{argv}");
    assert!(argv.contains("Release"), "{argv}");
    assert!(argv.contains("extra"), "{argv}");
    assert!(!argv.contains("ExecutionPolicy"), "{argv}");
}

#[test]
fn alias_to_vrun_file_runs_the_line() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("cmd.vrun");
    fs::write(&script, "/bin/echo aliased\n").unwrap();
    let cfg = dir.path().join("config.ini");
    fs::write(&cfg, "[alias]\nship = ./cmd.vrun\n").unwrap();
    let out = Command::new(vrun())
        .arg("ship")
        .current_dir(dir.path())
        .env("VAELRUN_CONFIG", &cfg)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "aliased\n");
}

#[test]
fn help_uses_the_invoked_name() {
    let (code, text) = capture_stdout(|| {
        vaelrun::run_with(vaelrun::RunOptions {
            args: Some(vec!["vr".into(), "-h".into()]),
            ..Default::default()
        })
    });
    assert_eq!(code.unwrap(), 0);
    assert!(text.contains("vaelrun"), "{text}");
    assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
    assert!(
        text.lines()
            .any(|line| line.trim_start().starts_with("vr ")),
        "{text}"
    );
    assert!(!text.contains("vrun"), "{text}");

    let (code, text) = capture_stdout(|| {
        vaelrun::run_with(vaelrun::RunOptions {
            args: Some(vec!["vr.exe".into(), "--help".into()]),
            ..Default::default()
        })
    });
    assert_eq!(code.unwrap(), 0);
    assert!(
        text.lines()
            .any(|line| line.trim_start().starts_with("vr ")),
        "{text}"
    );
    assert!(!text.contains("vr.exe"), "{text}");
}

#[test]
fn help_before_the_command_prints_usage() {
    let out = Command::new(vrun())
        .arg("-h")
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("Usage"), "{text}");
}

#[test]
fn help_after_the_command_is_an_argument() {
    let out = Command::new(vrun())
        .args(["/bin/echo", "-h"])
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("-h"), "{text}");
    assert!(!text.contains("Usage:"), "{text}");
}

fn capture_stdout<T>(f: impl FnOnce() -> T) -> (T, String) {
    use std::io::{Read, Write};
    use std::os::unix::io::FromRawFd;
    use std::sync::Mutex;

    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap();

    unsafe extern "C" {
        fn pipe(fds: *mut i32) -> i32;
        fn dup(fd: i32) -> i32;
        fn dup2(oldfd: i32, newfd: i32) -> i32;
        fn close(fd: i32) -> i32;
    }

    let mut fds = [0i32; 2];
    assert_eq!(unsafe { pipe(fds.as_mut_ptr()) }, 0);
    let saved = unsafe { dup(1) };
    assert!(saved >= 0);
    assert_eq!(unsafe { dup2(fds[1], 1) }, 1);
    let result = f();
    let _ = std::io::stdout().flush();
    assert_eq!(unsafe { dup2(saved, 1) }, 1);
    unsafe {
        close(saved);
        close(fds[1]);
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fds[0]) };
    let mut text = String::new();
    file.read_to_string(&mut text).unwrap();
    (result, text)
}

fn wait_file(path: &Path) -> String {
    for _ in 0..50 {
        if let Ok(text) = fs::read_to_string(path)
            && !text.is_empty()
        {
            return text;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("timed out waiting for {}", path.display());
}
