//! File access projection from file syscall events.

#[path = "file/access.rs"]
mod access;
#[path = "file/common.rs"]
mod common;
#[path = "file/enumerate.rs"]
mod enumerate;
#[path = "file/io.rs"]
mod io;
#[path = "file/summary.rs"]
mod summary;
#[path = "file/tty.rs"]
mod tty;

pub(super) use access::FileAccessProjector;
