use crate::{
    HostProcessCoordinates, NamespaceIdentity, NamespaceProcessCoordinates, ProcessIdentityManager,
    ProcessObservation,
};

const HOST_PID: u32 = 4_102_039;
const NAMESPACE_PID: u32 = 2_039;
const START_TICKS: u64 = 130_587_872;
const START_BOOTTIME_NS: u64 = 1_305_878_720_357_702;

fn namespace_observation(start_time_ticks: u64) -> ProcessObservation {
    ProcessObservation::namespace(NamespaceProcessCoordinates::new(
        NamespaceIdentity::new("pid:[4026532559]"),
        NAMESPACE_PID,
        start_time_ticks,
    ))
}

fn kernel_observation() -> ProcessObservation {
    ProcessObservation::host(
        HostProcessCoordinates::new(HOST_PID, 0).with_start_boottime_ns(START_BOOTTIME_NS),
    )
    .with_namespace(NamespaceProcessCoordinates::new(
        NamespaceIdentity::new("pid:[4026532559]"),
        NAMESPACE_PID,
        0,
    ))
}

#[test]
fn tls_first_and_kernel_enrichment_share_one_logical_identity() {
    let mut registry = ProcessIdentityManager::new(1);
    let tls = registry
        .resolve_or_create(namespace_observation(START_TICKS))
        .expect("resolve TLS observation");
    let kernel = registry
        .resolve_or_create(kernel_observation())
        .expect("resolve kernel observation");

    assert_eq!(tls.identity, kernel.identity);
    let record = registry.record(tls.identity).expect("process record");
    assert_eq!(record.host.as_ref().map(|host| host.pid), Some(HOST_PID));
    assert_eq!(record.namespaces.len(), 2);
}

#[test]
fn kernel_first_and_tls_enrichment_share_one_logical_identity() {
    let mut registry = ProcessIdentityManager::new(1);
    let kernel = registry
        .resolve_or_create(kernel_observation())
        .expect("resolve kernel observation");
    let tls = registry
        .resolve_or_create(namespace_observation(START_TICKS))
        .expect("resolve TLS observation");

    assert_eq!(kernel.identity, tls.identity);
}

#[test]
fn later_procfs_coordinates_enrich_kernel_record_without_rekeying() {
    let mut registry = ProcessIdentityManager::new(1);
    let kernel = registry
        .resolve_or_create(kernel_observation())
        .expect("resolve kernel observation");
    let procfs = ProcessObservation::host(
        HostProcessCoordinates::new(HOST_PID, START_TICKS)
            .with_start_boottime_ns(START_BOOTTIME_NS),
    )
    .with_namespace(NamespaceProcessCoordinates::new(
        NamespaceIdentity::new("pid:[4026532559]"),
        NAMESPACE_PID,
        START_TICKS,
    ));
    let enriched = registry
        .resolve_or_create(procfs)
        .expect("resolve procfs observation");

    assert_eq!(kernel.identity, enriched.identity);
    assert_eq!(
        registry
            .record(kernel.identity)
            .and_then(|record| record.host.as_ref())
            .map(|host| host.start_time_ticks),
        Some(START_TICKS)
    );
}

#[test]
fn host_pid_reuse_after_exit_allocates_a_new_logical_identity() {
    let mut registry = ProcessIdentityManager::new(1);
    let first = registry
        .resolve_or_create(ProcessObservation::host(HostProcessCoordinates::new(
            HOST_PID,
            START_TICKS,
        )))
        .expect("resolve first process");
    registry.mark_exited(first.identity);
    let reused = registry
        .resolve_or_create(ProcessObservation::host(HostProcessCoordinates::new(
            HOST_PID,
            START_TICKS + 1,
        )))
        .expect("resolve reused PID");

    assert_ne!(first.identity, reused.identity);
}
