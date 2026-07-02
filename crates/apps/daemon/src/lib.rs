//! Daemon application package skeleton.

pub(crate) mod bootstrap;
pub(crate) mod control_loop;
pub(crate) mod ebpf_resolve;
pub(crate) mod peer_identity;
pub(crate) mod profiles;
pub(crate) mod runtime_wiring;
pub(crate) mod service_host;
pub(crate) mod services;
pub(crate) mod socket_loop;

pub use bootstrap::{DaemonBootstrap, LocalDaemonServer};
pub use control_loop::handle_request;
pub use ebpf_resolve::{EbpfResolution, resolve_ebpf_collector_config};
pub use profiles::DaemonProfileRegistry;
pub use runtime_wiring::DaemonRuntimeWiring;
pub use service_host::{AttachService, DaemonServiceHost};
pub use socket_loop::DaemonRunError;
