use std::sync::{Arc, Mutex, TryLockError};

use plugin_system::{
    SandboxConsumerRegistration, SandboxPluginFacade, SandboxPluginRegistrationError,
};
use sandbox_alert_store::{
    SandboxAlertAdmission, SandboxAlertKind, SandboxAlertRecord, SandboxAlertSource,
    SandboxAlertWritePort,
};
use sandbox_observation::{GuestResourceSnapshot, Observation, ProcessIoCounters};
use sandbox_plugin_delivery::{
    SandboxConsumeError, SandboxConsumeReport, SandboxConsumerBatch, SandboxConsumerId,
    SandboxObservationConsumer, SandboxObservationKind,
};

use crate::state::{SourceStateTable, StateError};
use crate::{SandboxResourceAlertConfig, SandboxResourceAlertConfigError};

const PLUGIN_NAME: &str = "sandbox-resource-alert";

pub struct SandboxResourceAlertPlugin {
    config: SandboxResourceAlertConfig,
    source_states: Mutex<SourceStateTable>,
    alert_sink: Arc<dyn SandboxAlertWritePort>,
}

impl SandboxResourceAlertPlugin {
    pub fn new(
        config: SandboxResourceAlertConfig,
        alert_sink: Arc<dyn SandboxAlertWritePort>,
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
        observation_kinds: impl Into<Box<[SandboxObservationKind]>>,
        queue_capacity: u32,
    ) -> Result<SandboxConsumerId, SandboxPluginRegistrationError> {
        let consumer: Arc<dyn SandboxObservationConsumer> = self.clone();
        facade.register(SandboxConsumerRegistration::new(
            PLUGIN_NAME,
            observation_kinds,
            queue_capacity,
            consumer,
        ))
    }

    fn build_alerts(
        &self,
        batch: &SandboxConsumerBatch,
    ) -> Result<Vec<SandboxAlertRecord>, SandboxConsumeError> {
        let source = batch.source();
        let alert_source =
            SandboxAlertSource::new(source.gateway_id(), source.sb_id()).map_err(|_| {
                SandboxConsumeError::new(
                    "sandbox_resource_alert_source",
                    "sandbox source identifiers must be non-zero",
                )
            })?;
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
        for &observation_index in batch.observation_indices() {
            let Some(observation) = batch.observation(observation_index) else {
                continue;
            };
            match observation {
                Observation::GuestResource(resource) => {
                    self.observe_resource(
                        &mut states,
                        source,
                        alert_source,
                        sequence,
                        observation_index,
                        resource,
                        &mut alerts,
                    )?;
                }
                Observation::ProcessIo(process_io) => {
                    self.observe_process_io(
                        alert_source,
                        sequence,
                        observation_index,
                        process_io,
                        &mut alerts,
                    );
                }
            }
        }
        Ok(alerts)
    }

    fn observe_resource(
        &self,
        states: &mut SourceStateTable,
        source: sandbox_plugin_delivery::SandboxSource,
        alert_source: SandboxAlertSource,
        sequence: u64,
        observation_index: u32,
        resource: &GuestResourceSnapshot,
        alerts: &mut Vec<SandboxAlertRecord>,
    ) -> Result<(), SandboxConsumeError> {
        if let Some(usage_basis_points) = states
            .update_cpu_sample(source, resource.guest_boot_id, resource.cpu)
            .map_err(Self::state_error)?
        {
            let cpu_risk = usage_basis_points >= self.config.cpu_usage_threshold_basis_points;
            if states
                .update_cpu_risk(source, cpu_risk)
                .map_err(Self::state_error)?
            {
                alerts.push(SandboxAlertRecord::new(
                    alert_source,
                    sequence,
                    observation_index,
                    SandboxAlertKind::HighCpu {
                        guest_boot_id: resource.guest_boot_id,
                        sampled_at_ms: resource.sampled_at_ms,
                        usage_basis_points,
                        threshold_basis_points: self.config.cpu_usage_threshold_basis_points,
                    },
                ));
            }
        }
        if let Some(increment) = states
            .update_oom_kill_count(source, resource.memory.oom_kill_count)
            .map_err(Self::state_error)?
        {
            alerts.push(SandboxAlertRecord::new(
                alert_source,
                sequence,
                observation_index,
                SandboxAlertKind::OomKilled {
                    guest_boot_id: resource.guest_boot_id,
                    sampled_at_ms: resource.sampled_at_ms,
                    previous_count: increment.previous_count,
                    current_count: increment.current_count,
                    delta: increment.delta,
                },
            ));
        }
        let memory_risk =
            resource.memory.available_bytes < self.config.memory_available_threshold_bytes;
        if states
            .update_memory_risk(source, memory_risk)
            .map_err(Self::state_error)?
        {
            alerts.push(SandboxAlertRecord::new(
                alert_source,
                sequence,
                observation_index,
                SandboxAlertKind::OomRisk {
                    guest_boot_id: resource.guest_boot_id,
                    sampled_at_ms: resource.sampled_at_ms,
                    available_bytes: resource.memory.available_bytes,
                    threshold_bytes: self.config.memory_available_threshold_bytes,
                },
            ));
        }
        Ok(())
    }

    fn observe_process_io(
        &self,
        source: SandboxAlertSource,
        sequence: u64,
        observation_index: u32,
        process_io: &ProcessIoCounters,
        alerts: &mut Vec<SandboxAlertRecord>,
    ) {
        if process_io.read_bytes > self.config.read_interval_threshold_bytes {
            alerts.push(SandboxAlertRecord::new(
                source,
                sequence,
                observation_index,
                SandboxAlertKind::HighRead {
                    guest_boot_id: process_io.guest_boot_id,
                    process: process_io.process,
                    sample_started_ms: process_io.sample_started_ms,
                    sample_ended_ms: process_io.sample_ended_ms,
                    bytes: process_io.read_bytes,
                    threshold_bytes: self.config.read_interval_threshold_bytes,
                },
            ));
        }
        if process_io.write_bytes > self.config.write_interval_threshold_bytes {
            alerts.push(SandboxAlertRecord::new(
                source,
                sequence,
                observation_index,
                SandboxAlertKind::HighWrite {
                    guest_boot_id: process_io.guest_boot_id,
                    process: process_io.process,
                    sample_started_ms: process_io.sample_started_ms,
                    sample_ended_ms: process_io.sample_ended_ms,
                    bytes: process_io.write_bytes,
                    threshold_bytes: self.config.write_interval_threshold_bytes,
                },
            ));
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
        let dropped_records = alerts
            .into_iter()
            .filter(|alert| {
                !matches!(
                    self.alert_sink.try_append(*alert),
                    SandboxAlertAdmission::Accepted
                )
            })
            .count();
        let dropped_records = u64::try_from(dropped_records).unwrap_or(u64::MAX);
        Ok(SandboxConsumeReport {
            observed_records,
            dropped_records,
        })
    }
}
