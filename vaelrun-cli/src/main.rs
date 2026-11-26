fn main() {
    let code = match vaelrun::run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{err}");
            err.code
        }
    };
    std::process::exit(code);
}
