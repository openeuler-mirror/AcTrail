fn main() {
    if let Err(error) = tls_probe_point_finder::run_from_env() {
        use std::io::Write;

        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "error: {error}");
        std::process::exit(1);
    }
}
