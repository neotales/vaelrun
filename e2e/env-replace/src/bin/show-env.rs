//! Print one environment variable. Used by the injector e2e test.

fn main() {
    let key = std::env::args().nth(1).unwrap_or_else(|| "DB".to_string());
    print!("{}", std::env::var(&key).unwrap_or_default());
}
