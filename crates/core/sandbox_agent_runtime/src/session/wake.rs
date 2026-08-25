use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, Thread};
use std::time::Duration;

pub(crate) struct SessionWake {
    owner: OnceLock<Thread>,
    pending_commands: AtomicUsize,
    notified: AtomicBool,
}

impl SessionWake {
    pub(crate) fn new() -> Self {
        Self {
            owner: OnceLock::new(),
            pending_commands: AtomicUsize::new(0),
            notified: AtomicBool::new(false),
        }
    }

    pub(crate) fn bind_current(&self) {
        let _ = self.owner.set(thread::current());
    }

    pub(crate) fn begin_command(&self) {
        self.pending_commands.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn cancel_command(&self) {
        self.pending_commands.fetch_sub(1, Ordering::AcqRel);
    }

    pub(crate) fn command_received(&self) {
        self.pending_commands.fetch_sub(1, Ordering::AcqRel);
    }

    pub(crate) fn command_pending(&self) -> bool {
        self.pending_commands.load(Ordering::Acquire) != 0
    }

    pub(crate) fn notify(&self) {
        if !self.notified.swap(true, Ordering::AcqRel)
            && let Some(owner) = self.owner.get()
        {
            owner.unpark();
        }
    }

    pub(crate) fn wait(&self, timeout: Duration) {
        if self.notified.swap(false, Ordering::AcqRel) {
            return;
        }
        thread::park_timeout(timeout);
    }
}
