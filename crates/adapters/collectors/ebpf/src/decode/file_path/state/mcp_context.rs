//! Adapts completed FD operations to the optional MCP stdio tracker.

use collector_event::RawCollectorEvent;

use super::*;

impl FileTracker {
    pub(in crate::decode) fn mcp_stdio_enabled(&self) -> bool {
        self.mcp_stdio.enabled()
    }

    pub(in crate::decode::file_path) fn invalidate_mcp_fd_binding(
        &mut self,
        trace_id: TraceId,
        process: &ProcessObservation,
        fd: u32,
        observed_ktime_ns: u64,
    ) {
        if !self.mcp_stdio_enabled() {
            return;
        }
        let key = ProcessFileKey {
            trace_id,
            process: self.intern_process(process),
        };
        self.mcp_stdio
            .invalidate_fd_binding(&key, fd, observed_ktime_ns);
    }

    pub(crate) fn take_stdio_bundle_events(&mut self) -> Vec<RawCollectorEvent> {
        self.mcp_stdio.take_lifecycle_events()
    }

    pub(crate) fn mcp_stdio_diagnostics(&self) -> Vec<(&'static str, u64)> {
        self.mcp_stdio.mcp_stdio_diagnostics()
    }

    pub(super) fn apply_mcp_context(
        &mut self,
        process: &ProcessFileKey,
        syscall: &KernelFilePathEvent,
        result: i64,
        observed_ktime_ns: u64,
    ) {
        if !self.mcp_stdio_enabled() {
            return;
        }
        if syscall.aux == FILE_SYSCALL_CLOSE {
            self.mcp_stdio
                .close_fd(process, syscall.arg0 as u32, observed_ktime_ns);
            return;
        }
        if result < 0 {
            return;
        }
        match syscall.aux {
            FILE_SYSCALL_OPEN | FILE_SYSCALL_OPENAT | FILE_SYSCALL_OPENAT2 | FILE_SYSCALL_CREAT => {
                if let Ok(fd) = u32::try_from(result) {
                    self.mcp_stdio
                        .invalidate_fd_binding(process, fd, observed_ktime_ns);
                }
            }
            FILE_SYSCALL_CLOSE_RANGE => {
                self.mcp_stdio.close_range(
                    process,
                    syscall.arg0 as u32,
                    syscall.arg1 as u32,
                    syscall.arg2 & CLOSE_RANGE_CLOEXEC != 0,
                    observed_ktime_ns,
                );
            }
            FILE_SYSCALL_DUP | FILE_SYSCALL_DUP2 | FILE_SYSCALL_DUP3 | FILE_SYSCALL_FCNTL
                if syscall.aux != FILE_SYSCALL_FCNTL || fcntl_duplicates_fd(syscall) =>
            {
                if let Some(fd) = dup_target_fd(syscall, result) {
                    self.mcp_stdio.duplicate_fd(
                        process,
                        syscall.arg0 as u32,
                        fd,
                        duplicated_fd_close_on_exec(syscall),
                        observed_ktime_ns,
                    );
                }
            }
            FILE_SYSCALL_FCNTL if fcntl_sets_fd_flags(syscall) => {
                self.mcp_stdio.set_fd_close_on_exec(
                    process,
                    syscall.arg0 as u32,
                    syscall.arg2 & libc::FD_CLOEXEC as u64 != 0,
                );
            }
            FILE_SYSCALL_IOCTL_CLOEXEC | FILE_SYSCALL_IOCTL_NCLOEXEC => {
                self.mcp_stdio.set_fd_close_on_exec(
                    process,
                    syscall.arg0 as u32,
                    syscall.aux == FILE_SYSCALL_IOCTL_CLOEXEC,
                );
            }
            _ => {}
        }
    }
}
