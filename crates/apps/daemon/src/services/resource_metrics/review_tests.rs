use super::*;
use linux_platform::cgroup_v2::TraceScopePaths;
use std::fs;

fn host_boot_time_ns(ticks: u64, units: SystemUnits) -> u64 {
    // /proc start ticks already include this observer's namespace offset.
    // Undo it to construct the host-clock timestamp carried by eBPF events.
    let offsets = fs::read_to_string("/proc/self/timens_offsets").unwrap();
    let mut fields = offsets
        .lines()
        .find(|line| line.starts_with("boottime"))
        .unwrap()
        .split_whitespace()
        .skip(1);
    let seconds = fields.next().unwrap().parse::<i128>().unwrap();
    let nanos = fields.next().unwrap().parse::<i128>().unwrap();
    let observer_ns =
        (u128::from(ticks) * NANOS_PER_SECOND).div_ceil(u128::from(units.clock_ticks_per_second));
    u64::try_from(observer_ns as i128 - seconds * 1_000_000_000 - nanos).unwrap()
}

#[test]
fn stale_identity_emits_failure_while_allowing_fallback_for_live_and_recovered_traces() {
    let temp = tempfile::tempdir().unwrap();
    let (mut sampler, _) = fixture(temp.path());
    for recovered in [false, true] {
        let id = TraceId::new(if recovered { 2 } else { 1 });
        let mut drafts = Vec::new();
        let mut failures = Vec::new();
        for _ in 0..2 {
            assert!(!sampler.record_external_outcome(
                id,
                recovered,
                Ok(ExternalSampleOutcome::BecameStale),
                &mut drafts,
                &mut failures
            ));
        }
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].recovered, recovered);
        assert!(failures[0].message.contains("identity lost"));
    }
}

#[test]
fn procfs_fallback_rejects_reused_or_unverified_process_identity() {
    use model_core::process::{HostProcessCoordinates, ProcessObservation, ProcessRecord};
    let temp = tempfile::tempdir().unwrap();
    let (mut sampler, _) = fixture(temp.path());
    let pid = std::process::id();
    let ticks = read_proc_stat(pid).unwrap().unwrap().start_time_ticks;
    let process = ProcessIdentity::new(1);
    let units = sampler.units().unwrap();
    let tick_ns = 1_000_000_000 / units.clock_ticks_per_second;
    let boot_ns = host_boot_time_ns(ticks, units) + tick_ns / 2;
    for recovered in [false, true] {
        for (start, boot, should_sample) in [
            (ticks, None, true),
            (ticks + 1, None, false),
            (0, None, false),
            (0, Some(0), false),
            (0, Some(boot_ns), true),
            (0, Some(boot_ns + tick_ns), false),
            (ticks + 1, Some(boot_ns), false),
        ] {
            let mut host = HostProcessCoordinates::new(pid, start);
            host.start_boottime_ns = boot;
            let registry = ProcessIdentityManager::with_reserved_block(
                2,
                3,
                [ProcessRecord::new(process, ProcessObservation::host(host))],
            )
            .unwrap();
            let sample = sampler
                .collect_procfs_sample(
                    TraceId::new(1),
                    process,
                    vec![process],
                    &registry,
                    Instant::now(),
                    SystemTime::now(),
                    units,
                    recovered,
                    Some("stale".into()),
                )
                .unwrap();
            assert_eq!(
                sample.is_some(),
                should_sample,
                "ticks={start}, boot={boot:?}, recovered={recovered}"
            );
        }
    }
}

#[test]
fn procfs_sample_includes_child_with_only_boottime_identity() {
    use model_core::process::ProcessObservation;

    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let child = Child(
        std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap(),
    );
    let temp = tempfile::tempdir().unwrap();
    let (mut sampler, _) = fixture(temp.path());
    sampler.cgroup = None;
    let units = sampler.units().unwrap();
    let root_pid = std::process::id();
    let root_ticks = read_proc_stat(root_pid).unwrap().unwrap().start_time_ticks;
    let child_ticks = read_proc_stat(child.0.id())
        .unwrap()
        .unwrap()
        .start_time_ticks;
    let child_boot_ns = host_boot_time_ns(child_ticks, units);
    let mut registry = ProcessIdentityManager::new(1);
    let root = registry
        .resolve_or_create(ProcessObservation::host(HostProcessCoordinates::new(
            root_pid, root_ticks,
        )))
        .unwrap()
        .identity;
    let child_identity = registry
        .resolve_or_create(ProcessObservation::host(
            HostProcessCoordinates::new(child.0.id(), 0).with_start_boottime_ns(child_boot_ns),
        ))
        .unwrap()
        .identity;

    for recovered in [false, true] {
        let sample = sampler
            .collect_procfs_sample(
                TraceId::new(1),
                root,
                vec![root, child_identity],
                &registry,
                Instant::now(),
                SystemTime::now(),
                units,
                recovered,
                None,
            )
            .unwrap()
            .unwrap();
        assert_eq!(sample.payload.metadata["sampled_processes"], "2");
        assert_eq!(sample.payload.metadata["children"], "1");
        assert!(sample.payload.process_rss_sum_kb.unwrap() > 0);
        assert!(sample.payload.virtual_memory_kb.unwrap() > 0);
        assert!(sampler.previous_cpu.contains_key(&child_identity));
    }
}

#[test]
#[ignore = "requires permission to create Linux time namespaces and set clock offsets"]
fn procfs_samples_in_time_namespaces() {
    use std::os::unix::process::CommandExt;

    for offset in [60, -60] {
        let offsets = format!("boottime {offset} 0\n");
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args([
            "services::resource_metrics::review_tests::procfs",
            "--nocapture",
        ]);
        // Set the child namespace's offset before exec enters and freezes it.
        // Only async-signal-safe syscalls run after fork; the text is prebuilt.
        unsafe {
            command.pre_exec(move || {
                if libc::unshare(libc::CLONE_NEWTIME) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let fd = libc::open(
                    c"/proc/self/timens_offsets".as_ptr(),
                    libc::O_WRONLY | libc::O_CLOEXEC,
                );
                if fd < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let written = libc::write(fd, offsets.as_ptr().cast(), offsets.len());
                let error = std::io::Error::last_os_error();
                libc::close(fd);
                if written < 0 {
                    return Err(error);
                }
                if written as usize != offsets.len() {
                    return Err(std::io::ErrorKind::WriteZero.into());
                }
                Ok(())
            });
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "boottime offset {offset}s:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn live_and_recovered_external_outcomes_share_retry_and_fallback_semantics() {
    let temp = tempfile::tempdir().unwrap();
    let (mut sampler, _) = fixture(temp.path());
    for recovered in [false, true] {
        let id = TraceId::new(if recovered { 2 } else { 1 });
        let mut drafts = Vec::new();
        let mut failures = Vec::new();
        for _ in 0..2 {
            assert!(sampler.record_external_outcome(
                id,
                recovered,
                Ok(ExternalSampleOutcome::Failed("retry".into())),
                &mut drafts,
                &mut failures
            ));
        }
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].recovered, recovered);
        assert!(!sampler.record_external_outcome(
            id,
            recovered,
            Ok(ExternalSampleOutcome::BecameStale),
            &mut drafts,
            &mut failures
        ));
        assert!(drafts.is_empty());
    }
}

#[test]
fn forgotten_trace_drops_retained_barrier_and_fallback_reason() {
    let temp = tempfile::tempdir().unwrap();
    let (mut sampler, _) = fixture(temp.path());
    sampler.finalized_barriers.insert(TraceId::new(1));
    sampler
        .trace_fallback_reasons
        .insert(TraceId::new(1), "fallback".to_string());
    sampler.prune_forgotten(&trace_runtime::TraceRuntime::new(Vec::new(), 1));
    assert!(sampler.finalized_barriers.is_empty());
    assert!(sampler.trace_fallback_reasons.is_empty());
}

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
