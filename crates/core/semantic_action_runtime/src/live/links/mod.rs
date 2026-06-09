//! Cross-action links for live semantic actions.

mod agent;
mod command;
mod http;
mod llm;
mod projector;
mod shared;
mod sse;

pub(super) use projector::ActionLinkProjector;
