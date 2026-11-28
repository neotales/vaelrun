fn main() {
    let code = match vaelrun_core::run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{err}");
            err.code
        }
    };
    std::process::exit(code);
}
