// Tests for the corresponding production module.
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
