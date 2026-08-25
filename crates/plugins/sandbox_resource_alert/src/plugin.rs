use std::sync::{Arc, Mutex, TryLockError};

use arc_swap::ArcSwap;

use plugin_system::{
    SandboxConsumerRegistration, SandboxPluginFacade, SandboxPluginRegistrationError,
};
use sandbox_alert_store::{
    SandboxAlertAdmission, SandboxAlertKind, SandboxAlertRecord, SandboxAlertSource,
    SandboxAlertWritePort,
};
use sandbox_observation::{
    GuestResourceSnapshot, Observation, OomVictimObservation, ProcessIoCounters,
};
use sandbox_plugin_delivery::{
    SandboxConsumeError, SandboxConsumeReport, SandboxConsumerBatch, SandboxConsumerId,
    SandboxObservationConsumer, SandboxObservationKind,
};

use crate::state::{SourceStateTable, StateError};
use crate::{SandboxResourceAlertConfig, SandboxResourceAlertConfigError};

const PLUGIN_NAME: &str = "sandbox-resource-alert";

pub struct SandboxResourceAlertPlugin {
    config: ArcSwap<SandboxResourceAlertConfig>,
    source_state_capacity: u32,
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
            config: ArcSwap::from_pointee(config),
            source_state_capacity: config.source_state_capacity,
            source_states: Mutex::new(SourceStateTable::new(capacity)),
            alert_sink,
        })
    }

    pub fn publish_config(&self, config: SandboxResourceAlertConfig) {
        debug_assert_eq!(config.source_state_capacity, self.source_state_capacity);
        self.config.store(Arc::new(config));
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
        let config = self.config.load();
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
                        &config,
                        resource,
                        &mut alerts,
                    )?;
                }
                Observation::ProcessIo(process_io) => {
                    self.observe_process_io(
                        alert_source,
                        sequence,
                        observation_index,
                        &config,
                        process_io,
                        &mut alerts,
                    );
                }
                Observation::OomVictim(victim) => {
                    self.observe_oom_victim(
                        alert_source,
                        sequence,
                        observation_index,
                        victim,
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
        config: &SandboxResourceAlertConfig,
        resource: &GuestResourceSnapshot,
        alerts: &mut Vec<SandboxAlertRecord>,
    ) -> Result<(), SandboxConsumeError> {
        if let Some(usage_basis_points) = states
            .update_cpu_sample(source, resource.guest_boot_id, resource.cpu)
            .map_err(Self::state_error)?
        {
            let cpu_risk = usage_basis_points >= config.cpu_usage_threshold_basis_points;
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
                        threshold_basis_points: config.cpu_usage_threshold_basis_points,
                    },
                ));
            }
        }
        let memory_risk = resource.memory.available_bytes < config.memory_available_threshold_bytes;
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
                    threshold_bytes: config.memory_available_threshold_bytes,
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
        config: &SandboxResourceAlertConfig,
        process_io: &ProcessIoCounters,
        alerts: &mut Vec<SandboxAlertRecord>,
    ) {
        if process_io.read_bytes > config.read_interval_threshold_bytes {
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
                    threshold_bytes: config.read_interval_threshold_bytes,
                },
            ));
        }
        if process_io.write_bytes > config.write_interval_threshold_bytes {
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
                    threshold_bytes: config.write_interval_threshold_bytes,
                },
            ));
        }
    }

    fn observe_oom_victim(
        &self,
        source: SandboxAlertSource,
        sequence: u64,
        observation_index: u32,
        victim: &OomVictimObservation,
        alerts: &mut Vec<SandboxAlertRecord>,
    ) {
        alerts.push(SandboxAlertRecord::new(
            source,
            sequence,
            observation_index,
            SandboxAlertKind::OomKilled {
                guest_boot_id: victim.guest_boot_id,
                detected_at_ms: victim.detected_at_ms,
                victim_pid: victim.victim_pid,
                victim_comm: victim.victim_comm,
                attribution: victim.attribution,
                monitored_root: victim.monitored_root,
            },
        ));
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use sandbox_alert_store::{
        SandboxAlertAdmission, SandboxAlertKind, SandboxAlertRecord, SandboxAlertWritePort,
    };
    use sandbox_observation::{CpuSnapshot, GuestBootId, GuestResourceSnapshot, MemorySnapshot};
    use sandbox_plugin_delivery::{
        SandboxConsumerBatch, SandboxObservationConsumer, SandboxSource,
    };

    use super::{SandboxResourceAlertConfig, SandboxResourceAlertPlugin};

    #[derive(Default)]
    struct RecordingAlertSink {
        alerts: Mutex<Vec<SandboxAlertRecord>>,
    }

    impl RecordingAlertSink {
        fn alerts(&self) -> Vec<SandboxAlertRecord> {
            self.alerts.lock().expect("alert sink lock").clone()
        }
    }

    impl SandboxAlertWritePort for RecordingAlertSink {
        fn try_append(&self, alert: SandboxAlertRecord) -> SandboxAlertAdmission {
            self.alerts.lock().expect("alert sink lock").push(alert);
            SandboxAlertAdmission::Accepted
        }
    }

    fn resource_batch(
        sequence: u64,
        sampled_at_ms: u64,
        total_ticks: u64,
        idle_ticks: u64,
    ) -> SandboxConsumerBatch {
        let observations = vec![sandbox_observation::Observation::GuestResource(
            GuestResourceSnapshot {
                guest_boot_id: GuestBootId::new([7; 16]),
                sampled_at_ms,
                cpu: CpuSnapshot {
                    total_ticks,
                    idle_ticks,
                    logical_cpu_count: 2,
                },
                memory: MemorySnapshot {
                    total_bytes: 1_024,
                    available_bytes: 1_024,
                    used_bytes: 0,
                    oom_kill_count: 0,
                },
            },
        )]
        .into();
        SandboxConsumerBatch::new(
            SandboxSource::new(3, 5).expect("valid source"),
            sequence,
            observations,
            vec![0].into(),
        )
    }

    #[test]
    fn high_cpu_alert_is_emitted_only_when_usage_enters_risk() {
        let sink = Arc::new(RecordingAlertSink::default());
        let plugin = SandboxResourceAlertPlugin::new(
            SandboxResourceAlertConfig {
                cpu_usage_threshold_basis_points: 7_500,
                memory_available_threshold_bytes: 1,
                read_interval_threshold_bytes: 1,
                write_interval_threshold_bytes: 1,
                source_state_capacity: 4,
            },
            sink.clone(),
        )
        .expect("valid plugin config");

        plugin
            .consume(resource_batch(1, 10, 1_000, 800))
            .expect("baseline sample");
        assert!(sink.alerts().is_empty());

        plugin
            .consume(resource_batch(2, 20, 1_100, 820))
            .expect("first high-CPU sample");
        let alerts = sink.alerts();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].batch_sequence(), 2);
        assert_eq!(alerts[0].observation_index(), 0);
        assert_eq!(
            alerts[0].kind(),
            SandboxAlertKind::HighCpu {
                guest_boot_id: GuestBootId::new([7; 16]),
                sampled_at_ms: 20,
                usage_basis_points: 8_000,
                threshold_basis_points: 7_500,
            }
        );

        plugin
            .consume(resource_batch(3, 30, 1_200, 840))
            .expect("sustained high-CPU sample");
        assert_eq!(sink.alerts().len(), 1);

        plugin
            .consume(resource_batch(4, 40, 1_300, 940))
            .expect("recovery sample");
        assert_eq!(sink.alerts().len(), 1);

        plugin
            .consume(resource_batch(5, 50, 1_400, 960))
            .expect("second high-CPU transition");
        let alerts = sink.alerts();
        assert_eq!(alerts.len(), 2);
        assert_eq!(alerts[1].batch_sequence(), 5);
        assert_eq!(alerts[1].detected_at_ms(), 50);
    }
}
