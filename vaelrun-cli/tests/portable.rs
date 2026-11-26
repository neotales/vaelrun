//! Exit-code and argv tests that do not assume `/bin/sh` or `/usr/bin/echo`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn vrun() -> &'static str {
    env!("CARGO_BIN_EXE_vrun")
}

fn fixture() -> &'static str {
    env!("CARGO_BIN_EXE_fixture")
}

/// A copy with no suffix, so Windows `.exe` is not sent to the dotnet builder.
fn copy_fixture(dir: &Path, name: &str) -> PathBuf {
    let dest = dir.join(name);
    fs::copy(fixture(), &dest).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&dest).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&dest, perms).unwrap();
    }
    dest
}

fn child(dir: &Path) -> PathBuf {
    copy_fixture(dir, "child")
}

#[test]
fn child_exit_code_comes_from_the_fixture() {
    let dir = tempfile::tempdir().unwrap();
    let program = child(dir.path());
    let out = Command::new(vrun())
        .args([program.as_os_str(), "exit".as_ref(), "42".as_ref()])
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
fn parent_env_reaches_the_fixture() {
    let dir = tempfile::tempdir().unwrap();
    let program = child(dir.path());
    let out = Command::new(vrun())
        .args([
            program.as_os_str(),
            "env".as_ref(),
            "VAELRUN_PORTABLE".as_ref(),
        ])
        .env("VAELRUN_PORTABLE", "seen")
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "seen");
}

#[test]
fn version_prints_vaelrun_prefix() {
    let out = Command::new(vrun())
        .arg("--version")
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
    let version = text
        .strip_prefix("vaelrun ")
        .unwrap_or_else(|| panic!("missing vaelrun prefix: {text}"))
        .trim();
    assert!(!version.is_empty(), "{text}");
}

#[test]
fn unset_removes_a_parent_variable() {
    let dir = tempfile::tempdir().unwrap();
    let program = child(dir.path());
    let cleared = Command::new(vrun())
        .args(["-u", "VAELRUN_PARENT_UNSET"])
        .arg(&program)
        .args(["env", "VAELRUN_PARENT_UNSET"])
        .env("VAELRUN_PARENT_UNSET", "present")
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        cleared.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&cleared.stderr)
    );
    assert_eq!(String::from_utf8(cleared.stdout).unwrap(), "");
}

#[test]
fn chdir_sets_the_child_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    let program = child(dir.path());
    let work = dir.path().join("work");
    fs::create_dir(&work).unwrap();
    for flag in ["-C", "--chdir"] {
        let out = Command::new(vrun())
            .arg(flag)
            .arg(&work)
            .arg(&program)
            .arg("cwd")
            .env_remove("VAELRUN_CONFIG")
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{flag}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let printed = String::from_utf8(out.stdout).unwrap();
        let actual = PathBuf::from(printed.trim())
            .canonicalize()
            .unwrap_or_else(|err| panic!("{flag}: {printed}: {err}"));
        let expected = work.canonicalize().unwrap();
        assert_eq!(actual, expected, "{flag}: {printed}");
    }
}

#[test]
fn env_file_reaches_the_child_and_inline_wins() {
    let dir = tempfile::tempdir().unwrap();
    let program = child(dir.path());
    let envfile = dir.path().join("secrets.txt");
    fs::write(
        &envfile,
        "VAELRUN_FROM_FILE=from-file\nVAELRUN_SHARED=from-file\n",
    )
    .unwrap();

    let from_file = Command::new(vrun())
        .arg("--env-file")
        .arg(&envfile)
        .arg(&program)
        .args(["env", "VAELRUN_FROM_FILE"])
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        from_file.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&from_file.stderr)
    );
    assert_eq!(String::from_utf8(from_file.stdout).unwrap(), "from-file");

    let inline = Command::new(vrun())
        .arg("--env-file")
        .arg(&envfile)
        .arg("VAELRUN_SHARED=from-inline")
        .arg(&program)
        .args(["env", "VAELRUN_SHARED"])
        .env_remove("VAELRUN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        inline.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&inline.stderr)
    );
    assert_eq!(String::from_utf8(inline.stdout).unwrap(), "from-inline");
}

#[test]
fn trust_off_allows_a_path_name_and_trust_flag_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let name = "vaelrun-trust-child";
    // Windows resolves a bare name through PATHEXT. The argument stays unsuffixed.
    #[cfg(windows)]
    let on_disk = "vaelrun-trust-child.exe";
    #[cfg(not(windows))]
    let on_disk = name;
    copy_fixture(&bin, on_disk);
    let cfg = dir.path().join("config.ini");
    fs::write(&cfg, "[trust]\nmode = off\n").unwrap();
    let log = dir.path().join("ran");
    let path = prepend_path(&bin);

    let allowed = Command::new(vrun())
        .arg(name)
        .env("PATH", &path)
        .env("FIXTURE_LOG", &log)
        .env("VAELRUN_CONFIG", &cfg)
        .output()
        .unwrap();
    assert_eq!(
        allowed.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&allowed.stderr)
    );
    assert!(log.is_file(), "child did not run");

    fs::remove_file(&log).unwrap();
    let refused = Command::new(vrun())
        .arg("--trust")
        .arg(name)
        .env("PATH", &path)
        .env("FIXTURE_LOG", &log)
        .env("VAELRUN_CONFIG", &cfg)
        .output()
        .unwrap();
    assert_eq!(
        refused.status.code(),
        Some(126),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
    let err = String::from_utf8(refused.stderr).unwrap();
    assert!(err.to_ascii_lowercase().contains("trust"), "{err}");
    assert!(!log.exists(), "child ran despite --trust");
}

fn prepend_path(dir: &Path) -> std::ffi::OsString {
    let mut dirs = vec![dir.to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(dirs).unwrap()
}

#[test]
fn complete_zsh_fish_and_pwsh_mention_vrun() {
    for shell in ["zsh", "fish", "pwsh"] {
        let out = Command::new(vrun())
            .args(["complete", shell])
            .env_remove("VAELRUN_CONFIG")
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{shell}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(text.contains("vrun"), "{shell}: {text}");
    }
}
