//! Seccomp socket payload capture service.

mod identity;
mod request;
mod service;

pub(crate) use service::SeccompSocketService;
