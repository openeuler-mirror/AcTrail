//! actrail-vsock-gateway composition root.

mod startup;

pub use startup::{GatewayAppConfig, GatewayBackend, GatewayBootstrap, GatewayConfigOverrides};
