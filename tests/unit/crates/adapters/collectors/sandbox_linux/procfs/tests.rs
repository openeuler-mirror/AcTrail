// Tests for the corresponding production module.
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use sandbox_observation::CpuSnapshot;

use super::ProcfsReader;

static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TempProcfs {
    root: PathBuf,
}

impl TempProcfs {
    fn with_stat(stat: &str) -> Self {
        let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "actrail-sandbox-linux-procfs-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create temporary procfs root");
        fs::write(root.join("stat"), stat).expect("write temporary procfs stat");
        Self { root }
    }

    fn reader(&self) -> ProcfsReader {
        ProcfsReader::open(self.root.clone()).expect("open temporary procfs root")
    }
}

impl Drop for TempProcfs {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("remove temporary procfs root");
    }
}

#[test]
fn cpu_snapshot_uses_linux_cpu_counter_semantics() {
    let procfs = TempProcfs::with_stat(
        "cpu  10 20 30 40 5 6 7 8 900 1000\n\
         cpu0 1 2 3 4 5 6 7 8 9 10\n\
         cpu1 1 2 3 4 5 6 7 8 9 10\n\
         cpux 1 2 3 4 5 6 7 8 9 10\n\
         intr 42\n",
    );

    let snapshot = procfs.reader().cpu_snapshot().expect("read CPU snapshot");

    assert_eq!(
        snapshot,
        CpuSnapshot {
            total_ticks: 126,
            idle_ticks: 45,
            logical_cpu_count: 2,
        }
    );
}

#[test]
fn cpu_snapshot_rejects_missing_aggregate_row() {
    let procfs = TempProcfs::with_stat("intr 42\ncpu0 1 2 3 4 5\n");

    let error = procfs.reader().cpu_snapshot().expect_err("reject stat");

    assert_eq!(error.stage(), "read_cpu");
    assert!(error.detail().contains("has no aggregate cpu row"));
}

#[test]
fn cpu_snapshot_rejects_aggregate_with_too_few_counters() {
    let procfs = TempProcfs::with_stat("cpu 1 2 3 4\ncpu0 1 2 3 4 5\n");

    let error = procfs.reader().cpu_snapshot().expect_err("reject stat");

    assert_eq!(error.stage(), "read_cpu");
    assert!(
        error
            .detail()
            .contains("aggregate cpu row has fewer than five counters")
    );
}

#[test]
fn cpu_snapshot_rejects_zero_logical_cpus() {
    let procfs = TempProcfs::with_stat(
        "cpu 1 2 3 4 5 6 7 8\n\
         cpux 1 2 3 4 5 6 7 8\n\
         intr 42\n",
    );

    let error = procfs.reader().cpu_snapshot().expect_err("reject stat");

    assert_eq!(error.stage(), "read_cpu");
    assert_eq!(error.detail(), "procfs reported zero logical CPUs");
}

#[test]
fn cpu_snapshot_rejects_aggregate_counter_overflow() {
    let procfs =
        TempProcfs::with_stat(&format!("cpu {} 1 0 0 0 0 0 0\ncpu0 1 2 3 4 5\n", u64::MAX));

    let error = procfs.reader().cpu_snapshot().expect_err("reject stat");

    assert_eq!(error.stage(), "read_cpu");
    assert_eq!(error.detail(), "aggregate CPU tick counter overflow");
}

#[test]
fn process_start_time_reader_uses_the_pid_reuse_marker() {
    let procfs = TempProcfs::with_stat("cpu 1 1 1 1 1\ncpu0 1 1 1 1 1\n");
    let process = procfs.root.join("42");
    fs::create_dir(&process).unwrap();
    let mut fields = vec!["0"; 20];
    fields[0] = "S";
    fields[1] = "1";
    fields[19] = "4242";
    fs::write(
        process.join("stat"),
        format!("42 (workload root) {}\n", fields.join(" ")),
    )
    .unwrap();

    assert_eq!(procfs.reader().process_start_time_ticks(42).unwrap(), 4242);
}
