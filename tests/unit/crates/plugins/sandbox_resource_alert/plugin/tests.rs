// Tests for the corresponding production module.
use std::sync::{Arc, Mutex};

use sandbox_alert_store::{
    SandboxAlertAdmission, SandboxAlertKind, SandboxAlertRecord, SandboxAlertWritePort,
};
use sandbox_observation::{
    CpuSnapshot, GuestBootId, GuestPressureSnapshot, GuestResourceSnapshot, MemorySnapshot,
    PsiAverages,
};
use sandbox_plugin_delivery::{SandboxConsumerBatch, SandboxObservationConsumer, SandboxSource};

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
            memory_pressure_some_avg10_threshold_millipercent: 30_000,
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

fn pressure_batch(
    sequence: u64,
    sampled_at_ms: u64,
    some_avg10_millipercent: u32,
) -> SandboxConsumerBatch {
    let observations = vec![sandbox_observation::Observation::GuestPressure(
        GuestPressureSnapshot {
            guest_boot_id: GuestBootId::new([7; 16]),
            sampled_at_ms,
            memory_some: PsiAverages {
                avg10_millipercent: some_avg10_millipercent,
                avg60_millipercent: 0,
                avg300_millipercent: 0,
            },
            memory_full: PsiAverages::default(),
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
fn oom_imminent_is_edge_triggered_on_pressure_and_low_memory() {
    let sink = Arc::new(RecordingAlertSink::default());
    let plugin = SandboxResourceAlertPlugin::new(
        SandboxResourceAlertConfig {
            cpu_usage_threshold_basis_points: 10_000,
            memory_available_threshold_bytes: 2_000,
            memory_pressure_some_avg10_threshold_millipercent: 30_000,
            read_interval_threshold_bytes: 1,
            write_interval_threshold_bytes: 1,
            source_state_capacity: 4,
        },
        sink.clone(),
    )
    .expect("valid plugin config");

    // Establish memory availability (1024 < 2000) and the guest boot id.
    plugin
        .consume(resource_batch(1, 10, 1_000, 800))
        .expect("resource baseline");

    let imminent = |sink: &RecordingAlertSink| {
        sink.alerts()
            .into_iter()
            .filter(|alert| matches!(alert.kind(), SandboxAlertKind::OomImminent { .. }))
            .count()
    };

    plugin
        .consume(pressure_batch(2, 20, 25_000))
        .expect("pressure below threshold");
    assert_eq!(imminent(&sink), 0);

    plugin
        .consume(pressure_batch(3, 30, 30_000))
        .expect("pressure crosses threshold");
    assert_eq!(imminent(&sink), 1);

    plugin
        .consume(pressure_batch(4, 40, 40_000))
        .expect("sustained pressure");
    assert_eq!(imminent(&sink), 1);

    plugin
        .consume(pressure_batch(5, 50, 0))
        .expect("pressure recovery");
    assert_eq!(imminent(&sink), 1);

    plugin
        .consume(pressure_batch(6, 60, 50_000))
        .expect("second pressure transition");
    assert_eq!(imminent(&sink), 2);
}
