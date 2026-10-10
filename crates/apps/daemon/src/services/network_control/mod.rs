//! Trace-scoped INET connect policy enforcement.

mod audit;
mod descriptor;
mod request;
mod service;
mod worker;

pub(crate) use service::NetworkControlService;
