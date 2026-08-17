use model_core::ids::TraceId;
use model_core::process::{
    HostProcessCoordinates, NamespaceIdentity, NamespaceProcessCoordinates, ProcessObservation,
};

use super::{FileTracker, ProcessFileKey};

#[test]
fn exec_enrichment_preserves_state_inherited_by_host_only_fork() {
    let trace_id = TraceId::new(3);
    let parent =
        ProcessObservation::host(HostProcessCoordinates::new(100, 0).with_start_boottime_ns(10));
    let child_host =
        ProcessObservation::host(HostProcessCoordinates::new(200, 0).with_start_boottime_ns(20));
    let child_enriched = child_host
        .clone()
        .with_namespace(NamespaceProcessCoordinates::new(
            NamespaceIdentity::new("pid:[3]"),
            2,
            0,
        ));
    let mut tracker = FileTracker::default();
    tracker.seed_process(trace_id, parent.clone(), Some("/work".to_string()));
    tracker.inherit_process(trace_id, &parent, child_host.clone());

    tracker.exec_process(trace_id, child_enriched.clone(), 30);

    assert!(!tracker.processes.contains_key(&ProcessFileKey {
        trace_id,
        process: std::rc::Rc::new(child_host),
    }));
    assert_eq!(
        tracker
            .processes
            .get(&ProcessFileKey {
                trace_id,
                process: std::rc::Rc::new(child_enriched),
            })
            .and_then(|state| state.cwd.as_deref()),
        Some("/work")
    );
}
