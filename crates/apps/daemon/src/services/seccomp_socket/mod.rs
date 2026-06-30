//! Seccomp socket payload capture service.

mod http;
mod request;
mod service;

pub(crate) use service::SeccompSocketService;
