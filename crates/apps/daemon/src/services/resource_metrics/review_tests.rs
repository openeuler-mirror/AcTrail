use super::*;
use linux_platform::cgroup_v2::TraceScopePaths;
use std::fs;

fn fixture(root: &std::path::Path) -> (ResourceMetricsSampler, TraceScopePaths) {
    let mut storage = storage_factory::open_storage_backend(
        &storage_factory::StorageConfig::sqlite_path(root.join("test.sqlite")),
        storage_core::StorageOpenMode::ReadWrite,
    )
    .unwrap();
    let mut sampler =
        ResourceMetricsSampler::new(ResourceMetricsConfig::default(), storage.as_mut()).unwrap();
    sampler.config.include_system = false;
    sampler.config.finalization_timeout_ms = 1000;
    let mut runtime = CgroupResourceRuntime::test_runtime(&root.join("cg"));
    let paths = runtime.adapter.trace_paths(1, "test").unwrap();
    runtime.adapter.create_trace_scope(&paths).unwrap();
    fs::write(paths.aggregate.join("cgroup.events"), "populated 0\n").unwrap();
    fs::write(paths.aggregate.join("cpu.stat"), "usage_usec 100\n").unwrap();
    runtime.scopes.insert(TraceId::new(1), paths.clone());
    runtime
        .recovered_processes
        .insert(TraceId::new(1), ProcessIdentity::new(1));
    sampler.cgroup = Some(runtime);
    (sampler, paths)
}

#[test]
fn empty_scope_counter_failure_waits_then_produces_partial_final_without_aborting_poll() {
    let temp = tempfile::tempdir().unwrap();
    let (mut sampler, _) = fixture(temp.path());
    let traces = trace_runtime::TraceRuntime::new(Vec::new(), 1);
    let processes = ProcessIdentityManager::new(1);
    let poll = sampler.poll_finalizations(&traces, &processes).unwrap();
    assert_eq!(poll.waiting, vec![TraceId::new(1)]);
    assert!(poll.ready.is_empty());
    sampler.force_pending_finalizations_due();
    let poll = sampler.poll_finalizations(&traces, &processes).unwrap();
    assert_eq!(poll.ready.len(), 1);
    let finalization = &poll.ready[0];
    assert!(finalization.timed_out);
    assert_eq!(
        finalization.sample.payload.accounting_coverage,
        ResourceAccountingCoverage::Partial
    );
    assert_eq!(finalization.sample.payload.memory_current_bytes, None);
    assert!(
        finalization
            .sample
            .payload
            .metadata
            .contains_key("finalization_read_error")
    );
    sampler.finish_finalization(TraceId::new(1), false);
    assert!(
        sampler
            .poll_finalizations(&traces, &processes)
            .unwrap()
            .ready
            .is_empty()
    );
    assert!(sampler.resource_barrier_ready(TraceId::new(1)));
}

#[test]
fn production_sampler_retains_peak_reader_through_periodic_and_final_samples() {
    let temp = tempfile::tempdir().unwrap();
    let (mut sampler, paths) = fixture(temp.path());
    fs::write(paths.aggregate.join("memory.current"), "1024\n").unwrap();
    fs::write(paths.aggregate.join("memory.peak"), "4096\n").unwrap();
    let sample = sampler
        .collect_cgroup_sample(
            TraceId::new(1),
            ProcessIdentity::new(1),
            &paths,
            Instant::now(),
            SystemTime::now(),
            true,
        )
        .unwrap();
    assert_eq!(sample.payload.memory_peak_bytes, Some(4096));
    fs::rename(
        paths.aggregate.join("memory.peak"),
        paths.aggregate.join("old-peak"),
    )
    .unwrap();
    fs::write(paths.aggregate.join("memory.peak"), "8192\n").unwrap();
    let poll = sampler
        .poll_finalizations(
            &trace_runtime::TraceRuntime::new(Vec::new(), 1),
            &ProcessIdentityManager::new(1),
        )
        .unwrap();
    assert_eq!(poll.ready[0].sample.payload.memory_peak_bytes, Some(4096));
    sampler.finish_finalization(TraceId::new(1), true);
    assert!(sampler.cgroup.as_ref().unwrap().readers.is_empty());
    assert!(
        sampler.cgroup.as_ref().unwrap().cleanup_pending(&paths),
        "a failed final cleanup must remain queued after its reader is released"
    );
}

#[test]
fn failed_final_counter_read_does_not_block_another_trace() {
    let temp = tempfile::tempdir().unwrap();
    let (mut sampler, _) = fixture(temp.path());
    let runtime = sampler.cgroup.as_mut().unwrap();
    let paths = runtime.adapter.trace_paths(2, "healthy").unwrap();
    runtime.adapter.create_trace_scope(&paths).unwrap();
    fs::write(paths.aggregate.join("cgroup.events"), "populated 0\n").unwrap();
    fs::write(paths.aggregate.join("memory.current"), "1024\n").unwrap();
    fs::write(paths.aggregate.join("cpu.stat"), "usage_usec 100\n").unwrap();
    runtime.scopes.insert(TraceId::new(2), paths);
    runtime
        .recovered_processes
        .insert(TraceId::new(2), ProcessIdentity::new(2));
    let poll = sampler
        .poll_finalizations(
            &trace_runtime::TraceRuntime::new(Vec::new(), 1),
            &ProcessIdentityManager::new(1),
        )
        .unwrap();
    assert_eq!(poll.ready.len(), 1);
    assert_eq!(poll.ready[0].trace_id, TraceId::new(2));
    assert!(!poll.ready[0].timed_out);
}
