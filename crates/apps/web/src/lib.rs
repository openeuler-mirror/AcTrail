//! AcTrail storage UI and local plugin administration boundary.

#[path = "args.rs"]
mod args;
#[path = "http.rs"]
mod http;
#[path = "json.rs"]
mod json;
#[path = "plugins/mod.rs"]
mod plugins;
#[path = "render.rs"]
mod render;
#[path = "view.rs"]
mod view;

pub use args::{HELP_TEXT, WebConfig, is_help_request, parse_args};
pub use http::{RequestBudget, run_server, serve_listener};
