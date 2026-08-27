//! Trace-scoped INET connect policy enforcement.

mod audit;
mod descriptor;
mod policy;
mod request;
mod rules;
mod service;
mod worker;

pub(crate) use service::NetworkControlService;
