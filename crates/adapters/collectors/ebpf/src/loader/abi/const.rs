// Should be same with ACTRAIL_PROC_EXEC in bpf/actrail_runtime.h.
pub(super) const PROC_EXEC_EVENT_KIND: u32 = 2;

// Should be same with struct actrail_endpoint in bpf/actrail_runtime.h.
pub(super) const KERNEL_ENDPOINT_SIZE: usize = 24;
// Should be same with the packed prefix before local/remote endpoints in struct actrail_event.
pub(super) const KERNEL_OBSERVATION_HEADER_SIZE: usize = 72;
// Should be same with sizeof(struct actrail_event) in bpf/actrail_runtime.h.
pub(super) const KERNEL_OBSERVATION_EVENT_SIZE: usize =
    KERNEL_OBSERVATION_HEADER_SIZE + KERNEL_ENDPOINT_SIZE * 2;

// Should be same with ACTRAIL_EXEC_FILENAME_ABI_MAX_BYTES in bpf/include/actrail_const.h.
pub(super) const EXEC_FILENAME_ABI_MAX_BYTES: usize = 512;
// Should be same with ACTRAIL_EXEC_FILENAME_FLAG_TRUNCATED in bpf/include/actrail_const.h.
pub(super) const EXEC_FILENAME_FLAG_TRUNCATED: u32 = 1;
// Should be same with struct actrail_exec_event layout in bpf/actrail_runtime.h.
pub(super) const EXEC_EVENT_FILENAME_SIZE_OFFSET: usize = KERNEL_OBSERVATION_EVENT_SIZE;
pub(super) const EXEC_EVENT_FILENAME_FLAGS_OFFSET: usize = EXEC_EVENT_FILENAME_SIZE_OFFSET + 4;
pub(super) const EXEC_EVENT_FILENAME_OFFSET: usize = EXEC_EVENT_FILENAME_FLAGS_OFFSET + 4;
pub(super) const EXEC_EVENT_SIZE: usize = EXEC_EVENT_FILENAME_OFFSET + EXEC_FILENAME_ABI_MAX_BYTES;
