#[path = "actraild/args.rs"]
mod args;
#[path = "actraild/entry.rs"]
mod entry;
#[path = "actraild/logging.rs"]
mod logging;
#[path = "actraild/plugin_registry.rs"]
mod plugin_registry;
#[path = "actraild/process.rs"]
mod process;
#[path = "actraild/signals.rs"]
mod signals;

fn main() {
    if let Err(error) = logging::install() {
        tracing::error!(error = %error, "failed to install daemon tracing subscriber");
        std::process::exit(1);
    }
    if let Err(error) = entry::run_from_env() {
        tracing::error!(error = %error, "actraild command failed");
        std::process::exit(1);
    }
}
