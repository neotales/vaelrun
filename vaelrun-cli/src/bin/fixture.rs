//! Test helper. Not part of the vrun release surface.
//!
//! `FIXTURE_LOG` writes the arguments, one per line, and exits 0.
//! `exit <code>` exits with that code. `env <KEY>` prints the variable.
//! `cwd` prints the working directory.

use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    if let Ok(path) = env::var("FIXTURE_LOG") {
        let args: Vec<_> = env::args().skip(1).collect();
        if let Err(err) = fs::write(path, args.join("\n")) {
            eprintln!("fixture: {err}");
            return ExitCode::from(1);
        }
        return ExitCode::from(0);
    }

    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("exit") => {
            let code = args.next().and_then(|text| text.parse().ok()).unwrap_or(0);
            ExitCode::from(code)
        }
        Some("env") => {
            let key = args.next().unwrap_or_default();
            print!("{}", env::var(key).unwrap_or_default());
            ExitCode::from(0)
        }
        Some("cwd") => match env::current_dir() {
            Ok(dir) => {
                print!("{}", dir.display());
                ExitCode::from(0)
            }
            Err(err) => {
                eprintln!("fixture: {err}");
                ExitCode::from(1)
            }
        },
        _ => ExitCode::from(2),
    }
}
