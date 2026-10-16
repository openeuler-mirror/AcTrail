use ebpf_collector::SOCKET_PAYLOAD_SYSCALL_SENDTO;

use super::*;

#[test]
fn captures_linear_suffix_before_initial_completion_is_applied() {
    let mut service = test_service();
    let stream_key = test_stream_key();
    let body_len = 120_499;
    let header = request_header(body_len, 961);
    let request_size = header.len() as u64 + body_len;
    let initial_ptr = 0x25_724f_40f00;
    let initial_key = test_capture_key(initial_ptr, request_size);
    let initial_update = service.http_message_capture_update(
        &stream_key,
        stream_key.fd_generation,
        &initial_key,
        Some((initial_ptr, request_size)),
        &header,
    );

    // The first notification installs provisional state before the syscall is resumed.
    service.prepare_capture_update(&initial_update);

    // This is the ordering from the reported trace: the retry notification is drained
    // before the first syscall completion is polled from the BPF ring buffer.
    let first_completed = 89_060;
    let suffix_size = request_size - first_completed;
    let suffix_update = service
        .continuation_capture_update(
            &stream_key,
            Some((initial_ptr + first_completed, suffix_size)),
        )
        .expect("the contiguous suffix must be captured while completion is pending");
    match &suffix_update {
        SocketCaptureUpdate::Continuation {
            read_limit,
            body_offset,
            header_bytes_remaining,
            ..
        } => {
            assert_eq!(*read_limit, suffix_size);
            assert_eq!(*body_offset, Some(first_completed - header.len() as u64));
            assert_eq!(*header_bytes_remaining, 0);
        }
        other => panic!("expected continuation update, got {other:?}"),
    }

    service
        .apply_capture_update(
            initial_update,
            first_completed,
            first_completed,
            first_completed,
        )
        .unwrap();
    let continuation = service
        .continuations
        .get(&stream_key)
        .expect("the suffix is still outstanding");
    assert_eq!(
        continuation.confirmed_body_end,
        first_completed - header.len() as u64
    );

    service
        .apply_capture_update(suffix_update, suffix_size, suffix_size, suffix_size)
        .unwrap();
    assert!(!service.continuations.contains_key(&stream_key));
}

#[test]
fn captures_relocated_suffix_while_initial_completion_is_pending() {
    let mut service = test_service();
    let stream_key = test_stream_key();
    let body_len = 135_200;
    let header = request_header(body_len, 961);
    let request_size = header.len() as u64 + body_len;
    let initial_ptr = 0x25_724f_40f00;
    let initial_key = test_capture_key(initial_ptr, request_size);
    let initial_update = service.http_message_capture_update(
        &stream_key,
        stream_key.fd_generation,
        &initial_key,
        Some((initial_ptr, request_size)),
        &header,
    );
    service.prepare_capture_update(&initial_update);

    let first_completed = 89_060;
    let suffix_size = request_size - first_completed;
    let suffix_update = service
        .continuation_capture_update(&stream_key, Some((0x99_0000, suffix_size)))
        .expect("same-stream body continuation must not depend on buffer identity");
    match &suffix_update {
        SocketCaptureUpdate::Continuation {
            read_limit,
            body_offset,
            header_bytes_remaining,
            ..
        } => {
            assert_eq!(*read_limit, body_len);
            assert_eq!(*body_offset, None);
            assert_eq!(*header_bytes_remaining, 0);
        }
        other => panic!("expected continuation update, got {other:?}"),
    }

    service
        .apply_capture_update(
            initial_update,
            first_completed,
            first_completed,
            first_completed,
        )
        .unwrap();
    service
        .apply_capture_update(suffix_update, suffix_size, suffix_size, suffix_size)
        .unwrap();
    assert!(!service.continuations.contains_key(&stream_key));
}

#[test]
fn same_key_captures_complete_in_notification_order() {
    let mut service = test_service();
    let key = test_capture_key(0x25_724f_40f00, 2);
    service.queue_capture(key.clone(), test_captured_operation(vec![1]));
    service.queue_capture(key.clone(), test_captured_operation(vec![2]));

    assert_eq!(service.take_capture(&key).unwrap().bytes, vec![1]);
    assert!(service.captures.contains_key(&key));
    assert_eq!(service.take_capture(&key).unwrap().bytes, vec![2]);
    assert!(!service.captures.contains_key(&key));
}

fn test_service() -> SeccompSocketService {
    SeccompSocketService {
        enabled: true,
        max_operation_bytes: 4 * 1024 * 1024,
        max_segment_bytes: 4095,
        http_sniff_max_bytes: 65_536,
        max_pending_operations: 1024,
        max_stream_states: 1024,
        captures: BTreeMap::new(),
        continuations: BTreeMap::new(),
    }
}

fn test_stream_key() -> SocketContinuationKey {
    SocketContinuationKey {
        trace_id: 1,
        kernel_tgid: 2_715_018,
        pid_generation: 1,
        fd: 19,
        fd_generation: 11,
        direction: SOCKET_PAYLOAD_DIRECTION_OUTBOUND,
    }
}

fn test_capture_key(buffer_ptr: u64, requested_size: u64) -> SocketCaptureKey {
    SocketCaptureKey {
        trace_id: TraceId::new(1),
        pid_generation: 1,
        fd_generation: 11,
        kernel_tgid: 2_715_018,
        observer_tid: 2_715_018,
        fd: 19,
        syscall: SOCKET_PAYLOAD_SYSCALL_SENDTO,
        buffer_ptr,
        requested_size,
    }
}

fn test_captured_operation(bytes: Vec<u8>) -> CapturedSocketOperation {
    CapturedSocketOperation {
        trace_id: TraceId::new(1),
        process: ProcessObservation::default(),
        bytes,
        protocol_hint: None,
        update: SocketCaptureUpdate::None,
    }
}

fn request_header(content_length: u64, header_len: usize) -> Vec<u8> {
    let mut header =
        format!("POST /v1/messages HTTP/1.1\r\nContent-Length: {content_length}\r\nX-Pad: ")
            .into_bytes();
    assert!(header.len() + 4 <= header_len);
    header.resize(header_len - 4, b'x');
    header.extend_from_slice(b"\r\n\r\n");
    assert_eq!(header.len(), header_len);
    header
}
