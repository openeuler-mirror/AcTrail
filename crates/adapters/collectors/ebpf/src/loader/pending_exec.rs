//! Pidfd-addressed task-storage registration for one-shot exec promotion.

use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};

use libbpf_rs::{ErrorKind, MapCore, MapFlags, MapHandle, Object};
use model_core::ids::TraceId;
use model_core::process::InitialSuppressedFd;

use super::LoaderError;
use super::object::map_handle;
use super::suppressed_fd::{SUPPRESSED_FD_INDEX_SLOT_MAX, suppressed_fd_purpose_code};

const PENDING_EXEC_SUPPRESSED_FD_MAX: usize = SUPPRESSED_FD_INDEX_SLOT_MAX as usize;
const PENDING_EXEC_BINDING_HEADER_SIZE: usize = 24;
const PENDING_EXEC_SUPPRESSED_FD_SIZE: usize = 8;
const PENDING_EXEC_BINDING_VALUE_SIZE: usize = PENDING_EXEC_BINDING_HEADER_SIZE
    + PENDING_EXEC_SUPPRESSED_FD_MAX * PENDING_EXEC_SUPPRESSED_FD_SIZE;

pub(super) struct PendingExecBindings {
    task_storage: MapHandle,
    count: PendingExecCount,
    suppressed_fd_limit: usize,
}

impl PendingExecBindings {
    pub(super) fn from_object(
        object: &Object,
        suppressed_fd_limit: u32,
    ) -> Result<Self, LoaderError> {
        let task_storage = map_handle(object, "pending_exec_bindings", "pending_exec_bindings")?;
        if task_storage.key_size() as usize != std::mem::size_of::<i32>()
            || task_storage.value_size() as usize != PENDING_EXEC_BINDING_VALUE_SIZE
        {
            return Err(LoaderError::new(
                "pending_exec_bindings",
                format!(
                    "unexpected task-storage ABI key_size={} value_size={}",
                    task_storage.key_size(),
                    task_storage.value_size()
                ),
            ));
        }
        let suppressed_fd_limit = usize::try_from(suppressed_fd_limit).map_err(|error| {
            LoaderError::new(
                "pending_exec_bindings",
                format!("suppressed fd limit overflow: {error}"),
            )
        })?;
        if suppressed_fd_limit > PENDING_EXEC_SUPPRESSED_FD_MAX {
            return Err(LoaderError::new(
                "pending_exec_bindings",
                format!(
                    "suppressed fd limit {suppressed_fd_limit} exceeds pending binding capacity {PENDING_EXEC_SUPPRESSED_FD_MAX}"
                ),
            ));
        }
        Ok(Self {
            task_storage,
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
        pidfd: BorrowedFd<'_>,
        trace_id: TraceId,
        generation: u64,
        suppressed_fds: &[InitialSuppressedFd],
    ) -> Result<(), LoaderError> {
        if trace_id.get() == 0 || generation == 0 {
            return Err(LoaderError::new(
                "pending_exec_bindings",
                "pending exec binding requires non-zero trace and generation",
            ));
        }
        if suppressed_fds.len() > self.suppressed_fd_limit {
            return Err(LoaderError::new(
                "pending_exec_bindings",
                format!(
                    "{} initial suppressed fds exceed configured per-process limit {}",
                    suppressed_fds.len(),
                    self.suppressed_fd_limit
                ),
            ));
        }
        let key = pidfd.as_raw_fd().to_ne_bytes();
        // Publish the value as uncounted first so an exit racing this setup
        // cannot subtract a reservation that has not been added yet.
        let value = encode_binding(trace_id, generation, suppressed_fds, false)?;
        self.task_storage
            .update(&key, &value, MapFlags::NO_EXIST)
            .map_err(|error| LoaderError::new("pending_exec_bindings", error.to_string()))?;
        if let Err(error) = self.count.increment() {
            let _ = self.task_storage.delete(&key);
            return Err(error);
        }
        // Once the shared reservation exists, publish ownership of its
        // matching decrement to exec/exit.
        let counted_value = encode_binding(trace_id, generation, suppressed_fds, true)?;
        if let Err(error) = self
            .task_storage
            .update(&key, &counted_value, MapFlags::EXIST)
        {
            let _ = self.task_storage.delete(&key);
            self.count.decrement()?;
            return Err(LoaderError::new(
                "pending_exec_bindings",
                format!("publish counted pending binding: {error}"),
            ));
        }
        Ok(())
    }

    pub(super) fn cancel(&self, pidfd: BorrowedFd<'_>) -> Result<bool, LoaderError> {
        let key = pidfd.as_raw_fd().to_ne_bytes();
        match self.task_storage.delete(&key) {
            Ok(()) => {
                self.count.decrement()?;
                Ok(true)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
            Err(error) => Err(LoaderError::new(
                "pending_exec_bindings",
                format!("delete pending task storage: {error}"),
            )),
        }
    }
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

fn encode_binding(
    trace_id: TraceId,
    generation: u64,
    suppressed_fds: &[InitialSuppressedFd],
    counted: bool,
) -> Result<[u8; PENDING_EXEC_BINDING_VALUE_SIZE], LoaderError> {
    let suppressed_fd_count = u32::try_from(suppressed_fds.len()).map_err(|error| {
        LoaderError::new(
            "pending_exec_bindings",
            format!("suppressed fd count overflow: {error}"),
        )
    })?;
    let mut value = [0_u8; PENDING_EXEC_BINDING_VALUE_SIZE];
    value[0..8].copy_from_slice(&trace_id.get().to_ne_bytes());
    value[8..16].copy_from_slice(&generation.to_ne_bytes());
    value[16..20].copy_from_slice(&suppressed_fd_count.to_ne_bytes());
    value[20..24].copy_from_slice(&u32::from(counted).to_ne_bytes());
    for (index, suppressed_fd) in suppressed_fds.iter().enumerate() {
        let offset = PENDING_EXEC_BINDING_HEADER_SIZE + index * PENDING_EXEC_SUPPRESSED_FD_SIZE;
        value[offset..offset + 4].copy_from_slice(&suppressed_fd.fd.to_ne_bytes());
        value[offset + 4..offset + 8]
            .copy_from_slice(&suppressed_fd_purpose_code(suppressed_fd.purpose).to_ne_bytes());
    }
    Ok(value)
}
