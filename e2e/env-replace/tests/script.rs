use std::fs;
use std::process::Command;

use vaelrun::RunOptions;

#[test]
fn python_script_runs_through_vrun() {
    require("python");
    require("node");
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran.txt");
    let script = dir.path().join("hello.py");
    fs::write(
        &script,
        format!("open({:?}, 'w').write('from-python')\n", marker),
    )
    .unwrap();
    let code = vaelrun::run_with(RunOptions {
        args: Some(vec![
            "vrun".into(),
            "--no-trust".into(),
            script.as_os_str().to_os_string(),
        ]),
        ..Default::default()
    })
    .unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(code, 0);
    assert_eq!(fs::read_to_string(&marker).unwrap(), "from-python");
}

fn require(name: &str) {
    let output = Command::new(name).arg("--version").output();
    assert!(
        output.as_ref().is_ok_and(|out| out.status.success()),
        "{name} is missing"
    );
}
