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
fn child(dir: &Path) -> PathBuf {
    let dest = dir.join("child");
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
