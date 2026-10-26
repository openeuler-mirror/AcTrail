use std::io;
use std::time::{Duration, Instant};

use sandbox_vsock_contract::{
    HEADER_BYTES, MAX_ENCODED_OBSERVATION_BYTES, MAX_FRAME_BYTES, OBSERVATION_BATCH_FIXED_BYTES,
};

#[derive(Clone, Debug)]
pub struct SandboxAgentConfig {
    pub io_poll_interval: Duration,
    pub resource_poll_interval: Duration,
    pub workload_poll_interval: Duration,
    pub pressure_poll_interval: Duration,
    pub max_silence_interval: Duration,
    pub reconnect_interval: Duration,
    pub control_request_timeout: Duration,
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
            ("workload_poll_interval", self.workload_poll_interval),
            ("pressure_poll_interval", self.pressure_poll_interval),
            ("max_silence_interval", self.max_silence_interval),
            ("reconnect_interval", self.reconnect_interval),
            ("control_request_timeout", self.control_request_timeout),
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
        if Instant::now()
            .checked_add(self.control_request_timeout)
            .is_none()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "sandbox control request timeout exceeds the platform clock range",
            ));
        }
        let maximum = self
            .batch_max_observations
            .checked_mul(MAX_ENCODED_OBSERVATION_BYTES)
            .and_then(|value| value.checked_add(OBSERVATION_BATCH_FIXED_BYTES))
            .and_then(|value| value.checked_add(HEADER_BYTES))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "batch size overflow"))?;
        if maximum > MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "configured observation batch can exceed the wire frame limit",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(batch_max_observations: usize) -> SandboxAgentConfig {
        SandboxAgentConfig {
            io_poll_interval: Duration::from_secs(1),
            resource_poll_interval: Duration::from_secs(1),
            workload_poll_interval: Duration::from_secs(1),
            pressure_poll_interval: Duration::from_secs(1),
            max_silence_interval: Duration::from_secs(1),
            reconnect_interval: Duration::from_secs(1),
            control_request_timeout: Duration::from_secs(1),
            observation_queue_capacity: 1,
            batch_max_observations,
            worker_thread_stack_bytes: 1,
            metrics_enabled: false,
        }
    }

    #[test]
    fn validation_uses_largest_encoded_observation() {
        assert!(config(811).validate().is_ok());
        let error = config(812)
            .validate()
            .expect_err("812 workload observations exceed a frame");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("wire frame limit"));
    }
}
