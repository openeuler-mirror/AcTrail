use std::io;
use std::time::Duration;

use sandbox_vsock_contract::MAX_FRAME_BYTES;

const MAX_ENCODED_OBSERVATION_BYTES: usize = 111;
const BATCH_FIXED_BYTES: usize = 10;

#[derive(Clone, Debug)]
pub struct SandboxAgentConfig {
    pub io_poll_interval: Duration,
    pub resource_poll_interval: Duration,
    pub max_silence_interval: Duration,
    pub reconnect_interval: Duration,
    pub observation_queue_capacity: usize,
    pub batch_max_observations: usize,
    pub worker_thread_stack_bytes: usize,
    pub metrics_enabled: bool,
}

impl SandboxAgentConfig {
    pub fn validate(&self) -> io::Result<()> {
        for (name, value) in [
            ("io_poll_interval", self.io_poll_interval),
            ("resource_poll_interval", self.resource_poll_interval),
            ("max_silence_interval", self.max_silence_interval),
            ("reconnect_interval", self.reconnect_interval),
        ] {
            if value.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("sandbox agent {name} must be positive"),
                ));
            }
        }
        if self.observation_queue_capacity == 0
            || self.batch_max_observations == 0
            || self.worker_thread_stack_bytes == 0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "sandbox agent capacities and thread stack must be positive",
            ));
        }
        let maximum = self
            .batch_max_observations
            .checked_mul(MAX_ENCODED_OBSERVATION_BYTES)
            .and_then(|value| value.checked_add(BATCH_FIXED_BYTES))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "batch size overflow"))?;
        if maximum + 8 > MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "configured observation batch can exceed the wire frame limit",
            ));
        }
        Ok(())
    }
}
