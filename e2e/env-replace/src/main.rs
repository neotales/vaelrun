//! Replace a substring in every dotenv value. Quotes are preserved by neotales-dotenv.

use std::io::{self, Read};
use std::process::ExitCode;

use neotales_dotenv::Document;

fn main() -> ExitCode {
    let mut args = std::env::args();
    let _argv0 = args.next();
    let Some(from) = args.next() else {
        eprintln!("usage: env-replace FROM TO");
        return ExitCode::from(2);
    };
    let Some(to) = args.next() else {
        eprintln!("usage: env-replace FROM TO");
        return ExitCode::from(2);
    };

    let mut bytes = Vec::new();
    if let Err(err) = io::stdin().read_to_end(&mut bytes) {
        eprintln!("stdin: {err}");
        return ExitCode::from(1);
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return ExitCode::from(1);
    };

    let parsed = neotales_dotenv::parse(&text);
    let mut out = Document::new();
    for assignment in parsed.assignments() {
        let value = assignment.value.replace(&from, &to);
        out.add_quoted_variable(&assignment.key, &value, assignment.quote);
    }
    print!("{out}");
    ExitCode::from(0)
}
