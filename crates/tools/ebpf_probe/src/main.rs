fn main() {
    if let Err(error) = ebpf_probe::entry::run_from_env() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
