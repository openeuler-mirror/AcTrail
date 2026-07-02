const TLS_PAYLOAD_BPF: &str = include_str!("../bpf/actrail_tls_payload.h");

#[test]
fn bpf_copy_seccomp_fallback_requests_seccomp_capture_when_direct_copy_misses() {
    let (_, after_fallback_backend_check) = TLS_PAYLOAD_BPF
        .split_once(
            "payload_tls_capture_backend() == ACTRAIL_TLS_BACKEND_BPF_COPY_SECCOMP_FALLBACK",
        )
        .expect("BPF copy/seccomp fallback branch");
    let (fallback_block, _) = after_fallback_backend_check
        .split_once("payload_tls_capture_backend() == ACTRAIL_TLS_BACKEND_SECCOMP_USER_READ")
        .expect("seccomp user-read branch");

    assert!(
        fallback_block.contains("emit_tls_capture_request(ctx, &op, tgid, tid, op.requested_size)"),
        "BPF copy/seccomp fallback must emit a capture request after direct-copy misses"
    );
}
