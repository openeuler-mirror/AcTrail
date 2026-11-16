//! Stops a Trace's live processes with SIGSTOP before its files are restored.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use checkpoint_core::model::TraceRecord;
use checkpoint_core::ports::{PausedProcess, ProcessControl, ProcessControlError};
use ebpf_collector::procfs::ProcfsIdentityReader;
use model_core::ids::TraceId;
use model_core::process::ProcessRecord;
use process_identity::{IdentityLookupError, ProcessIdentityReader};

use super::procfs::{self, ProcStat};

type MemberSnapshot = Result<Vec<ProcessRecord>, ProcessControlError>;

/// Starts from the Trace root and known members, including reparented descendants.
#[derive(Clone)]
pub(crate) struct ProcfsProcessControl {
    members: Arc<Mutex<BTreeMap<TraceId, MemberSnapshot>>>,
    stop_timeout: Duration,
    stop_poll_interval: Duration,
}

impl ProcfsProcessControl {
    pub(super) fn new(stop_timeout: Duration, stop_poll_interval: Duration) -> Self {
        Self {
            members: Arc::default(),
            stop_timeout,
            stop_poll_interval,
        }
    }

    pub(super) fn set_members(&self, trace_id: TraceId, members: MemberSnapshot) {
        self.members
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(trace_id, members);
    }

    pub(super) fn forget(&self, trace_id: TraceId) {
        self.members
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&trace_id);
    }
}

impl ProcessControl for ProcfsProcessControl {
    fn pause(&self, trace: &TraceRecord) -> Result<Vec<PausedProcess>, ProcessControlError> {
        let members = self
            .members
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&trace.trace_id)
            .cloned()
            .unwrap_or_else(|| Ok(Vec::new()))?;
        let mut anchors = member_anchors(&members)?;
        anchors.push(PausedProcess {
            host_pid: trace.root.host_pid,
            start_time_ticks: trace.root.start_time_ticks,
        });
        let mut paused = Vec::new();
        match stop_trees(
            &anchors,
            &mut paused,
            self.stop_timeout,
            self.stop_poll_interval,
        ) {
            Ok(()) => Ok(paused),
            Err(error) => {
                let _ = self.resume(&paused);
                Err(error)
            }
        }
    }

    fn resume(&self, processes: &[PausedProcess]) -> Result<(), ProcessControlError> {
        for process in processes {
            let same_process = procfs::read_stat(process.host_pid)
                .map_err(io_error)?
                .is_some_and(|stat| stat.start_time_ticks == process.start_time_ticks);
            if same_process {
                signal(process.host_pid, libc::SIGCONT)?;
            }
        }
        Ok(())
    }
}

/// Late namespace-only observations can outlive the process. Match its generation before
/// treating it as a live anchor, rather than failing a restore because it has no cached host PID.
fn member_anchors(members: &[ProcessRecord]) -> Result<Vec<PausedProcess>, ProcessControlError> {
    let mut anchors = Vec::new();
    let processes = if members.iter().any(|member| member.host.is_none()) {
        procfs::all_processes().map_err(io_error)?
    } else {
        Vec::new()
    };
    for member in members {
        let host = if let Some(host) = &member.host {
            Some(host.clone())
        } else {
            let mut found = None;
            for process in &processes {
                let observation = match ProcfsIdentityReader.read_identity(process.pid) {
                    Ok(observation) => observation,
                    Err(IdentityLookupError::NotFound { .. }) => continue,
                    Err(error) => {
                        return Err(ProcessControlError::new(format!(
                            "read process {} identity: {error:?}",
                            process.pid
                        )));
                    }
                };
                if observation
                    .namespace
                    .as_ref()
                    .is_some_and(|namespace| member.namespaces.contains(namespace))
                {
                    found = observation.host;
                    break;
                }
            }
            found
        };
        if let Some(host) = host {
            let start_time_ticks = ProcfsIdentityReader
                .expected_start_time_ticks(&host)
                .ok_or_else(|| {
                    ProcessControlError::new(format!(
                        "process {} generation cannot be verified",
                        host.pid
                    ))
                })?;
            anchors.push(PausedProcess {
                host_pid: host.pid,
                start_time_ticks,
            });
        }
    }
    Ok(anchors)
}

fn stop_trees(
    anchors: &[PausedProcess],
    paused: &mut Vec<PausedProcess>,
    stop_timeout: Duration,
    stop_poll_interval: Duration,
) -> Result<(), ProcessControlError> {
    let mut seen = BTreeSet::new();
    loop {
        let processes = procfs::all_processes().map_err(io_error)?;
        let fresh: Vec<ProcStat> = anchors
            .iter()
            .filter(|anchor| {
                processes.iter().any(|stat| {
                    stat.pid == anchor.host_pid && stat.start_time_ticks == anchor.start_time_ticks
                })
            })
            .flat_map(|anchor| tree(anchor.host_pid, &processes))
            .filter(|process| seen.insert((process.pid, process.start_time_ticks)))
            .collect();
        if fresh.is_empty() {
            return Ok(());
        }
        let mut stopping = Vec::new();
        for process in fresh {
            if process.group_exited() {
                continue;
            }
            let paused_process = PausedProcess {
                host_pid: process.pid,
                start_time_ticks: process.start_time_ticks,
            };
            // Verify complete groups, but only resume stops this restore owns.
            if !process.is_stopped() {
                signal(process.pid, libc::SIGSTOP)?;
                paused.push(paused_process);
            }
            stopping.push(paused_process);
        }
        wait_until_stopped(&stopping, stop_timeout, stop_poll_interval)?;
    }
}

/// The root and every live descendant, found through parent links.
fn tree(root: u32, processes: &[ProcStat]) -> Vec<ProcStat> {
    let mut members: Vec<ProcStat> = processes
        .iter()
        .filter(|process| process.pid == root)
        .copied()
        .collect();
    let mut index = 0;
    while index < members.len() {
        let parent = members[index].pid;
        members.extend(
            processes
                .iter()
                .filter(|process| process.ppid == parent)
                .copied(),
        );
        index += 1;
    }
    members
}

fn wait_until_stopped(
    processes: &[PausedProcess],
    stop_timeout: Duration,
    poll_interval: Duration,
) -> Result<(), ProcessControlError> {
    let deadline = Instant::now() + stop_timeout;
    for process in processes {
        loop {
            let settled = match procfs::read_stat(process.host_pid).map_err(io_error)? {
                None => true,
                Some(stat)
                    if stat.start_time_ticks != process.start_time_ticks || stat.group_exited() =>
                {
                    true
                }
                Some(_) => procfs::thread_group_stopped(process.host_pid).map_err(io_error)?,
            };
            if settled {
                break;
            }
            if Instant::now() >= deadline {
                return Err(ProcessControlError::new(format!(
                    "thread group {} did not stop within {} ms",
                    process.host_pid,
                    stop_timeout.as_millis()
                )));
            }
            std::thread::sleep(
                poll_interval.min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
    Ok(())
}

fn signal(pid: u32, signal: libc::c_int) -> Result<(), ProcessControlError> {
    let pid = libc::pid_t::try_from(pid)
        .map_err(|_| ProcessControlError::new(format!("pid {pid} does not fit pid_t")))?;
    // SAFETY: kill only sends a signal to the given process.
    if unsafe { libc::kill(pid, signal) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        // The process exited in between, which leaves nothing to stop or continue.
        return Ok(());
    }
    Err(ProcessControlError::new(format!(
        "signal {signal} to process {pid}: {error}"
    )))
}

fn io_error(error: std::io::Error) -> ProcessControlError {
    ProcessControlError::new(format!("read /proc: {error}"))
}
