//! Read-only web UI for AcTrail SQLite stores.

#[path = "args.rs"]
mod args;
#[path = "http.rs"]
mod http;
#[path = "json.rs"]
mod json;
#[path = "render.rs"]
mod render;
#[path = "view.rs"]
mod view;

pub use args::{HELP_TEXT, WebConfig, is_help_request, parse_args};
pub use http::{RequestBudget, run_server, serve_listener};
