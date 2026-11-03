//! Trace-scoped INET connect policy enforcement.

mod alerts;
mod audit;
mod descriptor;
mod request;
mod service;
mod worker;

pub(in crate::services) use alerts::NetworkBoundaryAlerts;
pub(crate) use service::NetworkControlService;
