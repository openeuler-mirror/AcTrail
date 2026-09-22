//! Complete syscall observations before applying file or MCP consumers.

use super::*;

impl FileTracker {
    pub(in crate::decode::file_path) fn record(
        &mut self,
        event: KernelFilePathEvent,
        process: ProcessObservation,
        consumers: FileContextConsumers,
    ) -> Option<FileSyscallOutcome> {
        if event.phase == FILE_PHASE_EXIT
            && matches!(
                event.aux,
                FILE_SYSCALL_FCNTL
                    | FILE_SYSCALL_CLOSE
                    | FILE_SYSCALL_MMAP
                    | FILE_SYSCALL_OPEN
                    | FILE_SYSCALL_OPENAT
                    | FILE_SYSCALL_OPENAT2
                    | FILE_SYSCALL_CREAT
                    | FILE_SYSCALL_DUP
                    | FILE_SYSCALL_DUP2
                    | FILE_SYSCALL_DUP3
                    | FILE_SYSCALL_CLOSE_RANGE
                    | FILE_SYSCALL_FCHDIR
                    | FILE_SYSCALL_FTRUNCATE
            )
        {
            let process_key = ProcessFileKey {
                trace_id: event.trace_id,
                process: self.intern_process(&process),
            };
            let result = event.result;
            let observed_ktime_ns = event.observed_ktime_ns;
            return self.apply_completed_operation(
                &process_key,
                event,
                result,
                observed_ktime_ns,
                consumers,
            );
        }
        match event.phase {
            FILE_PHASE_ENTER => {
                self.pending.insert(
                    pending_key(&event),
                    PendingFileOperation {
                        process: ProcessFileKey {
                            trace_id: event.trace_id,
                            process: self.intern_process(&process),
                        },
                        syscall: event,
                    },
                );
                None
            }
            FILE_PHASE_EXIT => {
                let key = pending_key(&event);
                let pending = self.pending.remove(&key)?;
                self.apply_completed_operation(
                    &pending.process,
                    pending.syscall,
                    event.result,
                    event.observed_ktime_ns,
                    consumers,
                )
            }
            _ => None,
        }
    }

    fn apply_completed_operation(
        &mut self,
        process: &ProcessFileKey,
        syscall: KernelFilePathEvent,
        result: i64,
        observed_ktime_ns: u64,
        consumers: FileContextConsumers,
    ) -> Option<FileSyscallOutcome> {
        if consumers.mcp_stdio {
            self.apply_mcp_context(process, &syscall, result, observed_ktime_ns);
        }
        if !consumers.file_paths {
            return None;
        }
        let primary_path = self.resolve_primary_path(process, &syscall, result);
        let secondary_path = self.resolve_secondary_path(process, &syscall, result);
        let fd_path =
            self.apply_successful_exit(process, &syscall, result, &primary_path, &secondary_path);
        Some(FileSyscallOutcome {
            syscall,
            result,
            primary_path,
            secondary_path,
            fd_path,
        })
    }
}
