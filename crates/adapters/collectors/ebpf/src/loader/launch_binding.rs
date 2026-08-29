//! One-shot launch binding Module shared by the daemon and exec/exit hooks.

#[cfg(actrail_launch_binding_pid_generation_hash)]
#[path = "launch_binding/pid_generation_hash.rs"]
mod pid_generation_hash;
#[cfg(actrail_launch_binding_task_storage)]
#[path = "launch_binding/task_storage.rs"]
mod task_storage;

use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};

use libbpf_rs::{MapCore, MapFlags, MapHandle, Object};
use model_core::ids::TraceId;
use model_core::process::InitialSuppressedFd;

#[cfg(actrail_launch_binding_pid_generation_hash)]
use self::pid_generation_hash::{Adapter as SelectedAdapter, Reservation as SelectedReservation};
#[cfg(actrail_launch_binding_task_storage)]
use self::task_storage::{Adapter as SelectedAdapter, Reservation as SelectedReservation};
use super::LoaderError;
use super::object::map_handle;
use super::suppressed_fd::{SUPPRESSED_FD_INDEX_SLOT_MAX, suppressed_fd_purpose_code};

#[cfg(not(target_has_atomic = "64"))]
compile_error!("launch binding requires lock-free 64-bit atomics");
#[cfg(not(any(
    actrail_launch_binding_task_storage,
    actrail_launch_binding_pid_generation_hash
)))]
compile_error!("a launch binding Adapter must be selected");
#[cfg(all(
    actrail_launch_binding_task_storage,
    actrail_launch_binding_pid_generation_hash
))]
compile_error!("exactly one launch binding Adapter must be selected");

const PENDING_EXEC_SUPPRESSED_FD_MAX: usize = SUPPRESSED_FD_INDEX_SLOT_MAX as usize;
const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;
const PENDING_EXEC_BINDING_HEADER_SIZE: usize = 32;
const PENDING_EXEC_SUPPRESSED_FD_SIZE: usize = 8;
const PENDING_EXEC_BINDING_VALUE_SIZE: usize = PENDING_EXEC_BINDING_HEADER_SIZE
    + PENDING_EXEC_SUPPRESSED_FD_MAX * PENDING_EXEC_SUPPRESSED_FD_SIZE;
pub(crate) struct ArmedLaunchBinding {
    target: LaunchBindingTarget,
    reservation: SelectedReservation,
}

pub(super) struct LaunchBindingTarget {
    pidfd: OwnedFd,
    observer_tgid: u32,
    generation: u64,
}

impl LaunchBindingTarget {
    pub(super) fn new(
        pidfd: OwnedFd,
        observer_tgid: u32,
        generation: u64,
    ) -> Result<Self, LoaderError> {
        if observer_tgid == 0 || generation == 0 {
            return Err(LoaderError::new(
                "launch_binding_target",
                "launch binding requires non-zero observer TGID and generation",
            ));
        }
        Ok(Self {
            pidfd,
            observer_tgid,
            generation,
        })
    }
}

pub(super) struct PendingLaunchBinding<'a> {
    trace_id: TraceId,
    suppressed_fds: &'a [InitialSuppressedFd],
}

impl<'a> PendingLaunchBinding<'a> {
    pub(super) fn new(trace_id: TraceId, suppressed_fds: &'a [InitialSuppressedFd]) -> Self {
        Self {
            trace_id,
            suppressed_fds,
        }
    }

    fn encode(
        &self,
        target: &LaunchBindingTarget,
        suppressed_fd_limit: usize,
        counted: bool,
    ) -> Result<[u8; PENDING_EXEC_BINDING_VALUE_SIZE], LoaderError> {
        if self.trace_id.get() == 0 {
            return Err(LoaderError::new(
                "launch_binding",
                "launch binding requires a non-zero trace ID",
            ));
        }
        if self.suppressed_fds.len() > suppressed_fd_limit {
            return Err(LoaderError::new(
                "launch_binding",
                format!(
                    "{} initial suppressed fds exceed configured per-process limit {}",
                    self.suppressed_fds.len(),
                    suppressed_fd_limit
                ),
            ));
        }
        let suppressed_fd_count = u32::try_from(self.suppressed_fds.len()).map_err(|error| {
            LoaderError::new(
                "launch_binding",
                format!("suppressed fd count overflow: {error}"),
            )
        })?;
        let mut value = [0_u8; PENDING_EXEC_BINDING_VALUE_SIZE];
        value[0..8].copy_from_slice(&self.trace_id.get().to_ne_bytes());
        value[8..16].copy_from_slice(&target.generation.to_ne_bytes());
        value[16..20].copy_from_slice(&target.observer_tgid.to_ne_bytes());
        value[20..24].copy_from_slice(&suppressed_fd_count.to_ne_bytes());
        value[24..28].copy_from_slice(&u32::from(counted).to_ne_bytes());
        for (index, suppressed_fd) in self.suppressed_fds.iter().enumerate() {
            let offset = PENDING_EXEC_BINDING_HEADER_SIZE + index * PENDING_EXEC_SUPPRESSED_FD_SIZE;
            value[offset..offset + 4].copy_from_slice(&suppressed_fd.fd.to_ne_bytes());
            value[offset + 4..offset + 8]
                .copy_from_slice(&suppressed_fd_purpose_code(suppressed_fd.purpose).to_ne_bytes());
        }
        Ok(value)
    }
}

pub(super) struct LaunchExecBindings {
    adapter: SelectedAdapter,
    count: PendingExecCount,
    suppressed_fd_limit: usize,
}

impl LaunchExecBindings {
    pub(super) fn from_object(
        object: &Object,
        suppressed_fd_limit: u32,
    ) -> Result<Self, LoaderError> {
        let suppressed_fd_limit = usize::try_from(suppressed_fd_limit).map_err(|error| {
            LoaderError::new(
                "launch_binding",
                format!("suppressed fd limit overflow: {error}"),
            )
        })?;
        if suppressed_fd_limit > PENDING_EXEC_SUPPRESSED_FD_MAX {
            return Err(LoaderError::new(
                "launch_binding",
                format!(
                    "suppressed fd limit {suppressed_fd_limit} exceeds pending binding capacity {PENDING_EXEC_SUPPRESSED_FD_MAX}"
                ),
            ));
        }
        Ok(Self {
            adapter: SelectedAdapter::from_object(object)?,
            count: PendingExecCount::map(map_handle(
                object,
                "pending_exec_count",
                "pending_exec_count",
            )?)?,
            suppressed_fd_limit,
        })
    }

    pub(super) fn arm(
        &self,
        target: LaunchBindingTarget,
        pending: &PendingLaunchBinding<'_>,
    ) -> Result<ArmedLaunchBinding, LoaderError> {
        let uncounted = pending.encode(&target, self.suppressed_fd_limit, false)?;
        let reservation = self
            .adapter
            .reserve(&target, pending.trace_id, &uncounted)?;
        if let Err(increment_error) = self.count.increment() {
            let rollback = self.adapter.delete(&target, &reservation);
            return Err(transaction_error(increment_error, rollback));
        }

        let counted = pending.encode(&target, self.suppressed_fd_limit, true)?;
        if let Err(publish_error) = self.adapter.publish(&target, &reservation, &counted) {
            let rollback = self.adapter.delete(&target, &reservation);
            let decrement = self.count.decrement();
            if let Err(decrement_error) = decrement {
                return Err(LoaderError::new(
                    "launch_binding_rollback",
                    format!(
                        "publish failed at {}: {}; counter rollback failed at {}: {}",
                        publish_error.stage,
                        publish_error.message,
                        decrement_error.stage,
                        decrement_error.message
                    ),
                ));
            }
            return Err(transaction_error(publish_error, rollback));
        }

        Ok(ArmedLaunchBinding {
            target,
            reservation,
        })
    }

    pub(super) fn cancel(&self, armed: &ArmedLaunchBinding) -> Result<bool, LoaderError> {
        match self.adapter.delete(&armed.target, &armed.reservation) {
            DeleteOutcome::Missing => Ok(false),
            DeleteOutcome::Deleted => {
                self.count.decrement()?;
                Ok(true)
            }
            DeleteOutcome::DeletedWithCleanupFailure(error) => {
                self.count.decrement()?;
                Err(error)
            }
            DeleteOutcome::Failed(error) => Err(error),
        }
    }
}

pub(super) enum DeleteOutcome {
    Missing,
    Deleted,
    DeletedWithCleanupFailure(LoaderError),
    Failed(LoaderError),
}

fn transaction_error(primary: LoaderError, rollback: DeleteOutcome) -> LoaderError {
    match rollback {
        DeleteOutcome::Missing | DeleteOutcome::Deleted => primary,
        DeleteOutcome::DeletedWithCleanupFailure(rollback_error) => LoaderError::new(
            "launch_binding_rollback",
            format!(
                "operation failed at {}: {}; rollback failed at {}: {}",
                primary.stage, primary.message, rollback_error.stage, rollback_error.message
            ),
        ),
        DeleteOutcome::Failed(rollback_error) => LoaderError::new(
            "launch_binding_rollback",
            format!(
                "operation failed at {}: {}; rollback failed at {}: {}",
                primary.stage, primary.message, rollback_error.stage, rollback_error.message
            ),
        ),
    }
}

fn configure_generation_ticks(map: &MapHandle, stage: &'static str) -> Result<(), LoaderError> {
    if map.key_size() as usize != std::mem::size_of::<u32>()
        || map.value_size() as usize != std::mem::size_of::<u64>()
        || map.max_entries() != 1
    {
        return Err(LoaderError::new(
            stage,
            format!(
                "unexpected generation config ABI key_size={} value_size={} max_entries={}",
                map.key_size(),
                map.value_size(),
                map.max_entries()
            ),
        ));
    }
    let clock_ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let clock_ticks = u64::try_from(clock_ticks).map_err(|_| {
        LoaderError::new(
            stage,
            format!("invalid sysconf(_SC_CLK_TCK) value {clock_ticks}"),
        )
    })?;
    if clock_ticks == 0 || NANOSECONDS_PER_SECOND % clock_ticks != 0 {
        return Err(LoaderError::new(
            stage,
            format!(
                "kernel clock tick rate {clock_ticks} cannot represent exact launch generations"
            ),
        ));
    }
    let tick_ns = NANOSECONDS_PER_SECOND / clock_ticks;
    map.update(&0_u32.to_ne_bytes(), &tick_ns.to_ne_bytes(), MapFlags::ANY)
        .map_err(|error| {
            LoaderError::new(
                stage,
                format!("configure generation tick duration: {error}"),
            )
        })
}

struct PendingExecCount {
    _map: MapHandle,
    address: NonNull<libc::c_void>,
    length: usize,
}

impl PendingExecCount {
    fn map(map: MapHandle) -> Result<Self, LoaderError> {
        if map.value_size() as usize != std::mem::size_of::<u64>() || map.max_entries() != 1 {
            return Err(LoaderError::new(
                "pending_exec_count",
                format!(
                    "unexpected count map ABI value_size={} max_entries={}",
                    map.value_size(),
                    map.max_entries()
                ),
            ));
        }
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page_size <= 0 {
            return Err(LoaderError::new(
                "pending_exec_count",
                format!("invalid system page size {page_size}"),
            ));
        }
        let length = usize::try_from(page_size).map_err(|error| {
            LoaderError::new("pending_exec_count", format!("page size overflow: {error}"))
        })?;
        let address = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                map.as_fd().as_raw_fd(),
                0,
            )
        };
        if address == libc::MAP_FAILED {
            return Err(LoaderError::new(
                "pending_exec_count",
                format!("mmap shared counter: {}", std::io::Error::last_os_error()),
            ));
        }
        let address = NonNull::new(address).ok_or_else(|| {
            LoaderError::new("pending_exec_count", "mmap returned a null address")
        })?;
        Ok(Self {
            _map: map,
            address,
            length,
        })
    }

    fn increment(&self) -> Result<(), LoaderError> {
        let previous = self.value().fetch_add(1, Ordering::AcqRel);
        if previous == u64::MAX {
            self.value().fetch_sub(1, Ordering::AcqRel);
            return Err(LoaderError::new(
                "pending_exec_count",
                "pending exec count overflow",
            ));
        }
        Ok(())
    }

    fn decrement(&self) -> Result<(), LoaderError> {
        let previous = self.value().fetch_sub(1, Ordering::AcqRel);
        if previous == 0 {
            self.value().fetch_add(1, Ordering::AcqRel);
            return Err(LoaderError::new(
                "pending_exec_count",
                "pending exec count underflow",
            ));
        }
        Ok(())
    }

    fn value(&self) -> &AtomicU64 {
        unsafe { &*self.address.as_ptr().cast::<AtomicU64>() }
    }
}

impl Drop for PendingExecCount {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.address.as_ptr(), self.length);
        }
    }
}
