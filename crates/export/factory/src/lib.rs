//! Export subsystem configuration and factory.

mod builder;

pub use builder::{build_observation_consumer_from_manifest, validate_observation_consumer_config};
