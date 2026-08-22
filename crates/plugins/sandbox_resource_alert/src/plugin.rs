use std::sync::{Arc, Mutex, TryLockError};

use plugin_system::{
    SandboxConsumerRegistration, SandboxPluginFacade, SandboxPluginRegistrationError,
};
use sandbox_observation::{GuestResourceSnapshot, Observation, ProcessIoCounters};
use sandbox_plugin_delivery::{
    SandboxConsumeError, SandboxConsumeReport, SandboxConsumerBatch, SandboxConsumerId,
    SandboxObservationConsumer, SandboxObservationKind,
};

use crate::state::{SourceStateTable, StateError};
use crate::{
    SandboxAlert, SandboxAlertKind, SandboxAlertSink, SandboxResourceAlertConfig,
    SandboxResourceAlertConfigError,
};

const PLUGIN_NAME: &str = "sandbox-resource-alert";

pub struct SandboxResourceAlertPlugin {
    config: SandboxResourceAlertConfig,
    source_states: Mutex<SourceStateTable>,
    alert_sink: Arc<dyn SandboxAlertSink>,
}

impl SandboxResourceAlertPlugin {
    pub fn new(
        config: SandboxResourceAlertConfig,
        alert_sink: Arc<dyn SandboxAlertSink>,
    ) -> Result<Self, SandboxResourceAlertConfigError> {
        let capacity = config.validate()?;
        Ok(Self {
            config,
            source_states: Mutex::new(SourceStateTable::new(capacity)),
            alert_sink,
        })
    }

    pub fn register(
        self: &Arc<Self>,
        facade: &SandboxPluginFacade,
        queue_capacity: u32,
    ) -> Result<SandboxConsumerId, SandboxPluginRegistrationError> {
        let consumer: Arc<dyn SandboxObservationConsumer> = self.clone();
        facade.register(SandboxConsumerRegistration::new(
            PLUGIN_NAME,
            [
                SandboxObservationKind::ProcessIo,
                SandboxObservationKind::GuestResource,
            ],
            queue_capacity,
            consumer,
        ))
    }

    fn build_alerts(
        &self,
        batch: &SandboxConsumerBatch,
    ) -> Result<Vec<SandboxAlert>, SandboxConsumeError> {
        let source = batch.source();
        let sequence = batch.sequence();
        let mut states = match self.source_states.try_lock() {
            Ok(states) => states,
            Err(TryLockError::WouldBlock) => {
                return Err(SandboxConsumeError::new(
                    "sandbox_resource_alert_state_busy",
                    "source state is busy",
                ));
            }
            Err(TryLockError::Poisoned(_)) => {
                return Err(SandboxConsumeError::new(
                    "sandbox_resource_alert_state_unavailable",
                    "source state lock is poisoned",
                ));
            }
        };
        states.begin_batch(source).map_err(Self::state_error)?;
        let mut alerts = Vec::new();
        for observation in batch.observations() {
            match observation {
                Observation::GuestResource(resource) => {
                    self.observe_resource(&mut states, source, sequence, resource, &mut alerts)?;
                }
                Observation::ProcessIo(process_io) => {
                    self.observe_process_io(source, sequence, process_io, &mut alerts);
                }
            }
        }
        Ok(alerts)
    }

    fn observe_resource(
        &self,
        states: &mut SourceStateTable,
        source: sandbox_plugin_delivery::SandboxSource,
        sequence: u64,
        resource: &GuestResourceSnapshot,
        alerts: &mut Vec<SandboxAlert>,
    ) -> Result<(), SandboxConsumeError> {
        if let Some(increment) = states
            .update_oom_kill_count(source, resource.memory.oom_kill_count)
            .map_err(Self::state_error)?
        {
            alerts.push(SandboxAlert {
                source,
                batch_sequence: sequence,
                kind: SandboxAlertKind::OomKilled {
                    guest_boot_id: resource.guest_boot_id,
                    sampled_at_ms: resource.sampled_at_ms,
                    previous_count: increment.previous_count,
                    current_count: increment.current_count,
                    delta: increment.delta,
                },
            });
        }
        let memory_risk =
            resource.memory.available_bytes < self.config.memory_available_threshold_bytes;
        if states
            .update_memory_risk(source, memory_risk)
            .map_err(Self::state_error)?
        {
            alerts.push(SandboxAlert {
                source,
                batch_sequence: sequence,
                kind: SandboxAlertKind::OomRisk {
                    guest_boot_id: resource.guest_boot_id,
                    sampled_at_ms: resource.sampled_at_ms,
                    available_bytes: resource.memory.available_bytes,
                    threshold_bytes: self.config.memory_available_threshold_bytes,
                },
            });
        }
        Ok(())
    }

    fn observe_process_io(
        &self,
        source: sandbox_plugin_delivery::SandboxSource,
        sequence: u64,
        process_io: &ProcessIoCounters,
        alerts: &mut Vec<SandboxAlert>,
    ) {
        if process_io.read_bytes > self.config.read_interval_threshold_bytes {
            alerts.push(SandboxAlert {
                source,
                batch_sequence: sequence,
                kind: SandboxAlertKind::HighRead {
                    guest_boot_id: process_io.guest_boot_id,
                    process: process_io.process,
                    sample_started_ms: process_io.sample_started_ms,
                    sample_ended_ms: process_io.sample_ended_ms,
                    bytes: process_io.read_bytes,
                    threshold_bytes: self.config.read_interval_threshold_bytes,
                },
            });
        }
        if process_io.write_bytes > self.config.write_interval_threshold_bytes {
            alerts.push(SandboxAlert {
                source,
                batch_sequence: sequence,
                kind: SandboxAlertKind::HighWrite {
                    guest_boot_id: process_io.guest_boot_id,
                    process: process_io.process,
                    sample_started_ms: process_io.sample_started_ms,
                    sample_ended_ms: process_io.sample_ended_ms,
                    bytes: process_io.write_bytes,
                    threshold_bytes: self.config.write_interval_threshold_bytes,
                },
            });
        }
    }

    fn state_error(error: StateError) -> SandboxConsumeError {
        match error {
            StateError::ClockExhausted => SandboxConsumeError::new(
                "sandbox_resource_alert_state_clock_exhausted",
                "source state recency clock is exhausted",
            ),
            StateError::InvariantViolation => SandboxConsumeError::new(
                "sandbox_resource_alert_state_invariant",
                "source state capacity invariant failed",
            ),
        }
    }
}

impl SandboxObservationConsumer for SandboxResourceAlertPlugin {
    fn consume(
        &self,
        batch: SandboxConsumerBatch,
    ) -> Result<SandboxConsumeReport, SandboxConsumeError> {
        let observed_records = u64::try_from(batch.observation_indices().len()).map_err(|_| {
            SandboxConsumeError::new(
                "sandbox_resource_alert_count_overflow",
                "observation count does not fit u64",
            )
        })?;
        let alerts = self.build_alerts(&batch)?;
        for alert in alerts {
            self.alert_sink.try_submit(alert).map_err(|error| {
                SandboxConsumeError::new(
                    format!("sandbox_resource_alert_sink_{}", error.code()),
                    error.message(),
                )
            })?;
        }
        Ok(SandboxConsumeReport {
            observed_records,
            dropped_records: 0,
        })
    }
}
