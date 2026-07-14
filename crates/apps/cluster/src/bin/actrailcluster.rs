fn main() {
    if let Err(error) = cluster::run(std::env::args()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
