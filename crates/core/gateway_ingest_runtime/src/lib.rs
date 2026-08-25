//! Thread-safe admission facade for the isolated gateway-ingest path.

mod runtime;
mod sink;
mod status;

pub use runtime::{GatewayConnection, GatewayIngestRuntime, GatewayOpenError};
pub use sink::{SandboxObservationSink, SinkDeliveryError};
pub use status::GatewayIngestStatus;
