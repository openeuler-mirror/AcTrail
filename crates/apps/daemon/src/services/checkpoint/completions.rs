//! Hands coordinator completions from worker threads to the daemon loop through an eventfd.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::{Arc, Mutex, PoisonError};

use checkpoint_core::{Completion, CompletionSink};
use control_contract::reply::ControlError;

pub(super) struct CompletionQueue {
    completions: Mutex<Vec<Completion>>,
    wake: OwnedFd,
}

impl CompletionQueue {
    pub(super) fn new() -> Result<Arc<Self>, ControlError> {
        // SAFETY: eventfd only creates a new file descriptor.
        let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if fd < 0 {
            return Err(ControlError::new(
                "checkpoint_wake",
                format!("eventfd: {}", std::io::Error::last_os_error()),
            ));
        }
        Ok(Arc::new(Self {
            completions: Mutex::new(Vec::new()),
            // SAFETY: fd is a new descriptor that nothing else owns.
            wake: unsafe { OwnedFd::from_raw_fd(fd) },
        }))
    }

    /// Polled by the daemon loop; readable while completions wait.
    pub(super) fn wake_fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    pub(super) fn take(&self) -> Vec<Completion> {
        let mut counter = 0_u64;
        // SAFETY: reads eight bytes into the counter from the owned eventfd; a nonblocking read
        // with nothing pending fails with EAGAIN, which is fine here.
        let _ = unsafe {
            libc::read(
                self.wake.as_raw_fd(),
                (&mut counter as *mut u64).cast(),
                std::mem::size_of::<u64>(),
            )
        };
        std::mem::take(
            &mut *self
                .completions
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    /// Makes the daemon loop drain again soon, also without completions.
    pub(super) fn wake(&self) {
        let one = 1_u64;
        // SAFETY: writes eight bytes from `one` to the owned eventfd.
        let _ = unsafe {
            libc::write(
                self.wake.as_raw_fd(),
                (&one as *const u64).cast(),
                std::mem::size_of::<u64>(),
            )
        };
    }
}

pub(super) struct QueueSink(pub(super) Arc<CompletionQueue>);

impl CompletionSink for QueueSink {
    fn completed(&self, completion: Completion) {
        self.0
            .completions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(completion);
        self.0.wake();
    }
}
