//! Seccomp socket payload capture service.

mod http;
mod identity;
mod request;
mod service;

pub(crate) use service::SeccompSocketService;
