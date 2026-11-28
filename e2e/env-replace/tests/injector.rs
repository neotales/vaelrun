use std::ffi::OsString;
use std::fs;
use std::process::Command;

use vaelrun_core::{EnvironmentInput, build_environment, load_config};

#[test]
fn injector_replaces_the_marker_before_the_child_runs() {
    let dir = tempfile::tempdir().unwrap();
    let dotenv = dir.path().join("app.env");
    fs::write(&dotenv, "DB='secret:db'\n").unwrap();
    let cfg_path = dir.path().join("config.ini");
    let injector = std::env::var("CARGO_BIN_EXE_env-replace").unwrap();
    fs::write(
        &cfg_path,
        format!("[secrets]\ninjector = \"{injector}\" secret:db resolved\n"),
    )
    .unwrap();
    let config = load_config(std::slice::from_ref(&cfg_path)).unwrap();
    let parent = vec![(OsString::from("PATH"), OsString::from("/usr/bin"))];
    let env = build_environment(&EnvironmentInput {
        parent: &parent,
        dotenv: std::slice::from_ref(&dotenv),
        unset: &[],
        inline: &[],
        cwd: dir.path(),
        injector: config.injector(),
        config: &config,
        trust: Some(false),
    })
    .unwrap();
    assert_eq!(env.get("DB"), Some("resolved"));

    let show = std::env::var("CARGO_BIN_EXE_show-env").unwrap();
    let output = Command::new(show)
        .arg("DB")
        .env_clear()
        .envs(env.pairs().iter().cloned())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "resolved");
}
