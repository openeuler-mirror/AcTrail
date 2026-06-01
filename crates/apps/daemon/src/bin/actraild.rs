#[path = "actraild/args.rs"]
mod args;
#[path = "actraild/entry.rs"]
mod entry;
#[path = "actraild/process.rs"]
mod process;
#[path = "actraild/signals.rs"]
mod signals;

fn main() {
    if let Err(error) = entry::run_from_env() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
