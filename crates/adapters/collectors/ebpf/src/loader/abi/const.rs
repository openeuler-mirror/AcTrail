// Should be same with enum actrail_proc_event_kind in bpf/abi/observation.h.
pub(super) const PROC_FORK_EVENT_KIND: u32 = 1;
pub(super) const PROC_EXEC_EVENT_KIND: u32 = 2;
pub(super) const PROC_EXIT_EVENT_KIND: u32 = 3;
pub(super) const PROC_SIGNAL_EVENT_KIND: u32 = 4;

// Should be same with ACTRAIL_EVENT_ABI_REVISION in bpf/abi/observation.h.
pub(super) const EVENT_ABI_REVISION: u16 = 1;
// Should be same with sizeof(struct actrail_event_header).
pub(super) const EVENT_HEADER_SIZE: usize = 40;
// Should be same with the typed process records in bpf/abi/process.h.
pub(super) const PROCESS_FORK_EVENT_SIZE: usize = EVENT_HEADER_SIZE + 16;
pub(super) const PROCESS_EXEC_EVENT_SIZE: usize =
    EVENT_HEADER_SIZE + 8 + EXEC_FILENAME_ABI_MAX_BYTES;
pub(super) const PROCESS_EXIT_EVENT_SIZE: usize = EVENT_HEADER_SIZE + 8;
pub(super) const PROCESS_SIGNAL_EVENT_SIZE: usize = EVENT_HEADER_SIZE + 16;
pub(super) const NETWORK_EVENT_SIZE: usize = EVENT_HEADER_SIZE + 52;
pub(super) const FD_IO_EVENT_SIZE: usize = EVENT_HEADER_SIZE + 60;
pub(super) const SOCKET_RELEASE_EVENT_SIZE: usize = EVENT_HEADER_SIZE + 12;

// Should be same with struct actrail_endpoint in bpf/abi/observation.h.
pub(super) const KERNEL_ENDPOINT_SIZE: usize = 24;
// Should be same with ACTRAIL_EXEC_FILENAME_ABI_MAX_BYTES in bpf/common/constants.h.
pub(super) const EXEC_FILENAME_ABI_MAX_BYTES: usize = 512;
// Should be same with ACTRAIL_EXEC_FILENAME_FLAG_TRUNCATED in bpf/common/constants.h.
pub(super) const EXEC_FILENAME_FLAG_TRUNCATED: u32 = 1;
// Should be same with struct actrail_process_exec_event layout in bpf/abi/process.h.
pub(super) const EXEC_EVENT_FILENAME_SIZE_OFFSET: usize = EVENT_HEADER_SIZE;
pub(super) const EXEC_EVENT_FILENAME_FLAGS_OFFSET: usize = EXEC_EVENT_FILENAME_SIZE_OFFSET + 4;
pub(super) const EXEC_EVENT_FILENAME_OFFSET: usize = EXEC_EVENT_FILENAME_FLAGS_OFFSET + 4;

// Should be same with struct actrail_launch_binding_failure_event in
// bpf/launch_binding/actrail_launch_binding.h.
pub(super) const LAUNCH_BINDING_FAILURE_EVENT_SIZE: usize = 16;
