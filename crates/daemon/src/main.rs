fn main() {
    if let Err(message) = daemon::serve_forever() {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
}
