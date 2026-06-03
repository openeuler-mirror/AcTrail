//! CLI entrypoints.

mod args;
mod command;
mod config;
mod output;
mod reporter;

pub(crate) use command::main_from_env;
