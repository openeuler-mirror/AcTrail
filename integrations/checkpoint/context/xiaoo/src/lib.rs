//! Xiaoo checkpoint integration using the shared dynamic library ABI.

mod backend;
mod client;
mod config;
mod constants;
mod error;
mod factory;
mod protocol;

pub use factory::checkpoint_module;
