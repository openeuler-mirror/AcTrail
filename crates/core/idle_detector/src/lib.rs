//! No-observable-progress interval detector.
//!
//! Consumes real-time semantic action increments plus task lifecycle events
//! and opens persisted `IdleInterval` records when a root user task
//! has no observable progress. Under the active-child protection semantics a
//! task with any unfinished command, process, or sub-agent action is never
//! considered idle; intervals appear only when the whole task chain is silent
//! beyond the configured threshold.

mod detector;
mod task_state;

pub use detector::IdleDetector;
