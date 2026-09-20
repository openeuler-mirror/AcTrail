//! Discarding observation storage with no retained history.

mod backend;
mod resource_state;
mod storage;
mod transaction;

pub use storage::NoOpStorage;
