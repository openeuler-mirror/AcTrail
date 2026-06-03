#include "tls_payload_probe_capture.h"

SEC("uprobe")
int handle_ssl_write(struct pt_regs *ctx) {
    struct tls_probe_emit_op op = {
        .buffer_ptr = TLS_PROBE_ARG2(ctx),
        .requested_size = tls_probe_positive_i32_size(TLS_PROBE_ARG3(ctx)),
        .stream_key = TLS_PROBE_ARG1(ctx),
        .symbol = TLS_PROBE_SYMBOL_SSL_WRITE,
        .direction = TLS_PROBE_DIRECTION_OUTBOUND,
        .flags = 0,
    };
    return emit_payload(&op);
}

SEC("uprobe")
int handle_ssl_write_ex(struct pt_regs *ctx) {
    struct tls_probe_emit_op op = {
        .buffer_ptr = TLS_PROBE_ARG2(ctx),
        .requested_size = TLS_PROBE_ARG3(ctx),
        .stream_key = TLS_PROBE_ARG1(ctx),
        .symbol = TLS_PROBE_SYMBOL_SSL_WRITE_EX,
        .direction = TLS_PROBE_DIRECTION_OUTBOUND,
        .flags = 0,
    };
    return emit_payload(&op);
}

SEC("uprobe")
int handle_ssl_read_enter(struct pt_regs *ctx) {
    struct tls_probe_pending_op op = {
        .buffer_ptr = TLS_PROBE_ARG2(ctx),
        .requested_size = tls_probe_positive_i32_size(TLS_PROBE_ARG3(ctx)),
        .size_ptr = TLS_PROBE_EMPTY_SIZE_POINTER,
        .stream_key = TLS_PROBE_ARG1(ctx),
        .symbol = TLS_PROBE_SYMBOL_SSL_READ,
        .direction = TLS_PROBE_DIRECTION_INBOUND,
    };
    return store_pending(&op);
}

SEC("uretprobe")
int handle_ssl_read_return(struct pt_regs *ctx) {
    __u64 completed_size = tls_probe_positive_i32_size(TLS_PROBE_RET(ctx));

    if (!completed_size) {
        __u64 key = bpf_get_current_pid_tgid();
        bpf_map_delete_elem(&pending_ops, &key);
        return 0;
    }
    return emit_pending_return(completed_size);
}

SEC("uprobe")
int handle_ssl_read_ex_enter(struct pt_regs *ctx) {
    struct tls_probe_pending_op op = {
        .buffer_ptr = TLS_PROBE_ARG2(ctx),
        .requested_size = TLS_PROBE_ARG3(ctx),
        .size_ptr = TLS_PROBE_ARG4(ctx),
        .stream_key = TLS_PROBE_ARG1(ctx),
        .symbol = TLS_PROBE_SYMBOL_SSL_READ_EX,
        .direction = TLS_PROBE_DIRECTION_INBOUND,
    };
    return store_pending(&op);
}

SEC("uretprobe")
int handle_ssl_read_ex_return(struct pt_regs *ctx) {
    __u64 key = bpf_get_current_pid_tgid();
    struct tls_probe_pending_op *op = bpf_map_lookup_elem(&pending_ops, &key);
    __u64 completed_size = 0;
    long result = (long)TLS_PROBE_RET(ctx);

    if (result != TLS_PROBE_OPENSSL_EX_SUCCESS || !op || !op->size_ptr) {
        bpf_map_delete_elem(&pending_ops, &key);
        return 0;
    }
    if (bpf_probe_read_user(
            &completed_size,
            sizeof(completed_size),
            (void *)(unsigned long)op->size_ptr
        ) != 0) {
        bpf_map_delete_elem(&pending_ops, &key);
        return 0;
    }
    return emit_pending_return(completed_size);
}

SEC("uprobe")
int handle_rustls_buffer_plaintext(struct pt_regs *ctx) {
    return emit_rustls_payload(
        TLS_PROBE_ARG1(ctx),
        TLS_PROBE_ARG2(ctx),
        TLS_PROBE_SYMBOL_RUSTLS_BUFFER_PLAINTEXT
    );
}

SEC("uprobe")
int handle_rustls_take_received_plaintext(struct pt_regs *ctx) {
    return emit_rustls_payload(
        TLS_PROBE_ARG1(ctx),
        TLS_PROBE_ARG2(ctx),
        TLS_PROBE_SYMBOL_RUSTLS_TAKE_RECEIVED_PLAINTEXT
    );
}

char LICENSE[] SEC("license") = "GPL";
