mod admission;
mod bindings;
mod correlation;
mod http;
mod lifecycle;
mod orchestrator;
mod pending;
mod projection;

pub(in crate::llm_pipeline) use orchestrator::ProjectionCoordinator;
