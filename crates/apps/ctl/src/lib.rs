//! Control application package skeleton.

pub(crate) mod args;
mod clean;
pub(crate) mod dispatch;
mod entry;
pub(crate) mod launch;
pub(crate) mod output;
pub(crate) mod process_ref;
pub(crate) mod transport;

pub use args::CtlCommand;
pub use dispatch::dispatch;
pub use entry::run_from_env;
pub use output::format_reply;
