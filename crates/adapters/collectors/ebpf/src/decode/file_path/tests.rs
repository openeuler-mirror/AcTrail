use model_core::ids::TraceId;

use crate::decode::{FILE_EVENT_CONTEXT, FILE_EVENT_MMAP, FILE_EVENT_OPEN, FILE_EVENT_RENAME};
use crate::loader::KernelFilePathEvent;

use super::state::{
    FILE_FD_MISSING, FILE_PHASE_ENTER, FILE_PHASE_EXIT, FILE_SYSCALL_CHDIR, FILE_SYSCALL_DUP,
    FILE_SYSCALL_MMAP, FILE_SYSCALL_OPENAT, FILE_SYSCALL_RENAMEAT, FileTracker, PATH_FLAG_CAPTURED,
};

#[test]
fn tracker_resolves_openat_relative_path_from_seeded_cwd() {
    let mut tracker = FileTracker::default();
    let enter = file_event(FILE_EVENT_OPEN, FILE_PHASE_ENTER, FILE_SYSCALL_OPENAT)
        .with_arg0(libc::AT_FDCWD as u64)
        .with_arg2(libc::O_RDONLY as u64)
        .with_path("relative.txt")
        .build();
    tracker.seed_test_process(enter.pid, "/tmp/actrail-cwd");

    assert!(tracker.record(enter).is_none());
    let outcome = tracker
        .record(file_event(FILE_EVENT_OPEN, FILE_PHASE_EXIT, FILE_SYSCALL_OPENAT).with_result(7))
        .expect("openat exit completes pending syscall");

    assert_eq!(
        outcome.primary_path.resolved.as_deref(),
        Some("/tmp/actrail-cwd/relative.txt")
    );
    assert_eq!(
        tracker.resolve_fd_path(outcome.enter.pid, 7).as_deref(),
        Some("/tmp/actrail-cwd/relative.txt")
    );
}

#[test]
fn tracker_updates_fd_table_for_dup() {
    let mut tracker = FileTracker::default();
    tracker.seed_test_process(TEST_PID, "/tmp");
    tracker.seed_test_fd(TEST_PID, 3, "/tmp/source.txt");
    let enter = file_event(FILE_EVENT_CONTEXT, FILE_PHASE_ENTER, FILE_SYSCALL_DUP).with_arg0(3);

    tracker.record(enter.build());
    tracker
        .record(file_event(FILE_EVENT_CONTEXT, FILE_PHASE_EXIT, FILE_SYSCALL_DUP).with_result(8));

    assert_eq!(
        tracker.resolve_fd_path(TEST_PID, 8).as_deref(),
        Some("/tmp/source.txt")
    );
}

#[test]
fn tracker_updates_cwd_after_chdir_success() {
    let mut tracker = FileTracker::default();
    tracker.seed_test_process(TEST_PID, "/tmp");
    let enter = file_event(FILE_EVENT_CONTEXT, FILE_PHASE_ENTER, FILE_SYSCALL_CHDIR)
        .with_path("nested")
        .build();

    tracker.record(enter);
    tracker
        .record(file_event(FILE_EVENT_CONTEXT, FILE_PHASE_EXIT, FILE_SYSCALL_CHDIR).with_result(0));

    let open = file_event(FILE_EVENT_OPEN, FILE_PHASE_ENTER, FILE_SYSCALL_OPENAT)
        .with_arg0(libc::AT_FDCWD as u64)
        .with_path("file.txt")
        .build();
    tracker.record(open);
    let outcome = tracker
        .record(file_event(FILE_EVENT_OPEN, FILE_PHASE_EXIT, FILE_SYSCALL_OPENAT).with_result(4))
        .expect("openat exit completes pending syscall");
    assert_eq!(
        outcome.primary_path.resolved.as_deref(),
        Some("/tmp/nested/file.txt")
    );
}

#[test]
fn tracker_resolves_rename_target_with_separate_dirfds() {
    let mut tracker = FileTracker::default();
    tracker.seed_test_process(TEST_PID, "/tmp");
    tracker.seed_test_fd(TEST_PID, 10, "/tmp/source-dir");
    tracker.seed_test_fd(TEST_PID, 11, "/tmp/target-dir");
    let enter = file_event(FILE_EVENT_RENAME, FILE_PHASE_ENTER, FILE_SYSCALL_RENAMEAT)
        .with_arg0(10)
        .with_arg2(11)
        .with_path("old.txt")
        .with_secondary_path("new.txt")
        .build();

    tracker.record(enter);
    let outcome = tracker
        .record(
            file_event(FILE_EVENT_RENAME, FILE_PHASE_EXIT, FILE_SYSCALL_RENAMEAT).with_result(0),
        )
        .expect("renameat exit completes pending syscall");

    assert_eq!(
        outcome.primary_path.resolved.as_deref(),
        Some("/tmp/source-dir/old.txt")
    );
    assert_eq!(
        outcome
            .secondary_path
            .as_ref()
            .and_then(|path| path.resolved.as_deref()),
        Some("/tmp/target-dir/new.txt")
    );
}

#[test]
fn tracker_keeps_mmap_fd_path_from_fd_table() {
    let mut tracker = FileTracker::default();
    tracker.seed_test_process(TEST_PID, "/tmp");
    tracker.seed_test_fd(TEST_PID, 5, "/tmp/mmap.bin");
    let enter = file_event(FILE_EVENT_MMAP, FILE_PHASE_ENTER, FILE_SYSCALL_MMAP)
        .with_fd(5)
        .with_arg2((libc::PROT_READ | libc::PROT_WRITE) as u64)
        .with_arg3(libc::MAP_SHARED as u64)
        .build();

    tracker.record(enter);
    let outcome = tracker
        .record(file_event(FILE_EVENT_MMAP, FILE_PHASE_EXIT, FILE_SYSCALL_MMAP).with_result(4096))
        .expect("mmap exit completes pending syscall");

    assert_eq!(outcome.fd_path.as_deref(), Some("/tmp/mmap.bin"));
}

const TEST_PID: u32 = 4242;
const TEST_TID: u32 = 4242;
const TEST_TRACE_ID: TraceId = TraceId::new(7);
const TEST_PATH_MAX_BYTES: u32 = 255;

#[derive(Clone)]
struct EventBuilder {
    event: KernelFilePathEvent,
}

impl EventBuilder {
    fn build(self) -> KernelFilePathEvent {
        self.event
    }

    fn with_result(mut self, result: i64) -> KernelFilePathEvent {
        self.event.result = result;
        self.event
    }

    fn with_fd(mut self, fd: u32) -> Self {
        self.event.fd = fd;
        self.event.arg4 = fd as u64;
        self
    }

    fn with_arg0(mut self, value: u64) -> Self {
        self.event.arg0 = value;
        self
    }

    fn with_arg2(mut self, value: u64) -> Self {
        self.event.arg2 = value;
        self
    }

    fn with_arg3(mut self, value: u64) -> Self {
        self.event.arg3 = value;
        self
    }

    fn with_path(mut self, value: &str) -> Self {
        self.event.path = value.as_bytes().to_vec();
        self.event.path_size = self.event.path.len() as u32;
        self.event.path_flags = PATH_FLAG_CAPTURED;
        self
    }

    fn with_secondary_path(mut self, value: &str) -> Self {
        self.event.secondary_path = value.as_bytes().to_vec();
        self.event.secondary_path_size = self.event.secondary_path.len() as u32;
        self.event.secondary_path_flags = PATH_FLAG_CAPTURED;
        self
    }
}

fn file_event(kind: u32, phase: u32, syscall: u32) -> EventBuilder {
    EventBuilder {
        event: KernelFilePathEvent {
            kind,
            pid: TEST_PID,
            tid: TEST_TID,
            phase,
            result: 0,
            trace_id: TEST_TRACE_ID,
            observed_ktime_ns: 0,
            fd: FILE_FD_MISSING,
            aux: syscall,
            path_size: 0,
            path_flags: 0,
            secondary_path_size: 0,
            secondary_path_flags: 0,
            path_max_bytes: TEST_PATH_MAX_BYTES,
            arg0: 0,
            arg1: 0,
            arg2: 0,
            arg3: 0,
            arg4: 0,
            arg5: 0,
            pid_generation: 0,
            path: Vec::new(),
            secondary_path: Vec::new(),
        },
    }
}

impl FileTracker {
    fn seed_test_process(&mut self, pid: u32, cwd: &str) {
        let state = self.test_process_state(pid);
        state.cwd = Some(cwd.to_string());
    }

    fn seed_test_fd(&mut self, pid: u32, fd: u32, path: &str) {
        self.test_process_state(pid)
            .fds
            .insert(fd, path.to_string());
    }

    fn test_process_state(&mut self, pid: u32) -> &mut super::state::ProcessFileState {
        self.processes.entry(pid).or_default()
    }
}
