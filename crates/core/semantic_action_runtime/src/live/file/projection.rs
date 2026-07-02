#[path = "projection/access.rs"]
mod access;
#[path = "projection/bulk_read.rs"]
mod bulk_read;
#[path = "projection/enumerate.rs"]
mod enumerate;
#[path = "projection/io_action.rs"]
mod io_action;
#[path = "projection/summary.rs"]
mod summary;
#[path = "projection/tty.rs"]
mod tty;

pub(in crate::live) use access::FileAccessProjector;
