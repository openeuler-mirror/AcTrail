/* Programs owned by the file/open loading node. */
#include "../definitions.h"

SEC("tracepoint/syscalls/sys_enter_chdir")
int handle_sys_enter_chdir(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_chdir_enter(ctx);
}

SEC("tracepoint/syscalls/sys_enter_creat")
int handle_sys_enter_creat(struct trace_event_raw_sys_enter *ctx) {
    fd_open_enter(ctx);
    return capture_file_open_enter(ctx, ACTRAIL_FILE_SYSCALL_CREAT, 0);
}

SEC("tracepoint/syscalls/sys_enter_fchdir")
int handle_sys_enter_fchdir(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_fchdir_enter(ctx);
}

SEC("tracepoint/syscalls/sys_enter_open")
int handle_sys_enter_open(struct trace_event_raw_sys_enter *ctx) {
    fd_open_enter(ctx);
    return capture_file_open_enter(ctx, ACTRAIL_FILE_SYSCALL_OPEN, 0);
}

SEC("tracepoint/syscalls/sys_enter_openat")
int handle_sys_enter_openat(struct trace_event_raw_sys_enter *ctx) {
    fd_open_enter(ctx);
    return capture_file_open_enter(ctx, ACTRAIL_FILE_SYSCALL_OPENAT, 0);
}

SEC("tracepoint/syscalls/sys_enter_openat2")
int handle_sys_enter_openat2(struct trace_event_raw_sys_enter *ctx) {
    struct actrail_open_how how = {};

    fd_open_enter(ctx);
    if (!file_context_capture_enabled(ACTRAIL_FILE_SYSCALL_OPENAT2)) {
        return 0;
    }
    read_file_open_how(ctx, &how);
    return capture_file_open_enter(ctx, ACTRAIL_FILE_SYSCALL_OPENAT2, &how);
}

SEC("tracepoint/syscalls/sys_exit_chdir")
int handle_sys_exit_chdir(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_CHDIR);
}

SEC("tracepoint/syscalls/sys_exit_creat")
int handle_sys_exit_creat(struct trace_event_raw_sys_exit *ctx) {
    fd_register_open_exit(ctx);
    return emit_file_open_exit(ctx, ACTRAIL_FILE_SYSCALL_CREAT);
}

SEC("tracepoint/syscalls/sys_exit_fchdir")
int handle_sys_exit_fchdir(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_completion_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_FCHDIR);
}

SEC("tracepoint/syscalls/sys_exit_open")
int handle_sys_exit_open(struct trace_event_raw_sys_exit *ctx) {
    fd_register_open_exit(ctx);
    return emit_file_open_exit(ctx, ACTRAIL_FILE_SYSCALL_OPEN);
}

SEC("tracepoint/syscalls/sys_exit_openat")
int handle_sys_exit_openat(struct trace_event_raw_sys_exit *ctx) {
    fd_register_open_exit(ctx);
    return emit_file_open_exit(ctx, ACTRAIL_FILE_SYSCALL_OPENAT);
}

SEC("tracepoint/syscalls/sys_exit_openat2")
int handle_sys_exit_openat2(struct trace_event_raw_sys_exit *ctx) {
    fd_register_open_exit(ctx);
    return emit_file_open_exit(ctx, ACTRAIL_FILE_SYSCALL_OPENAT2);
}

#ifndef ACTRAIL_COMBINED_OBJECT
char LICENSE[] SEC("license") = "GPL";
#endif
