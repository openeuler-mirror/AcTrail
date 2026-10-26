//! Low-frequency detection of agent execution stalled outside normal waits.
mod alert;
mod config;
mod runtime;

pub use alert::IdleAlert;
pub use config::IdleDetectionConfig;
pub use runtime::IdleDetector;
