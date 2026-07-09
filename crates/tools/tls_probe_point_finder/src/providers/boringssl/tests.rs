use super::{
    BORINGSSL_IDENTITY_MARKERS, X86_64_READ_PATTERN, X86_64_WRITE_PATTERN, find_all,
    resolve_x86_64_offsets,
};

#[test]
fn x86_64_payload_only_plan_accepts_boringssl_identity() {
    let mut data = vec![0_u8; 0x14000];
    place(&mut data, 0x100, BORINGSSL_IDENTITY_MARKERS[0]);
    place(&mut data, 0x1000, X86_64_READ_PATTERN);
    place(&mut data, 0x1d00, X86_64_WRITE_PATTERN);
    place(&mut data, 0x12000, X86_64_WRITE_PATTERN);

    let resolved = resolve_x86_64_offsets(
        &data,
        &[],
        &find_all(&data, X86_64_READ_PATTERN),
        &find_all(&data, X86_64_WRITE_PATTERN),
    )
    .expect("BoringSSL identity permits payload-only read/write plan");

    assert_eq!(resolved.handshake, None);
    assert_eq!(resolved.read, 0x1000);
    assert_eq!(resolved.write, 0x1d00);
}

#[test]
fn x86_64_payload_only_plan_requires_boringssl_identity() {
    let mut data = vec![0_u8; 0x3000];
    place(&mut data, 0x1000, X86_64_READ_PATTERN);
    place(&mut data, 0x1d00, X86_64_WRITE_PATTERN);

    let error = resolve_x86_64_offsets(
        &data,
        &[],
        &find_all(&data, X86_64_READ_PATTERN),
        &find_all(&data, X86_64_WRITE_PATTERN),
    )
    .expect_err("payload-only plan must not match arbitrary similar code");

    assert_eq!(
        error.to_string(),
        "BoringSSL SSL_do_handshake pattern match count=0"
    );
}

fn place(data: &mut [u8], offset: usize, value: &[u8]) {
    data[offset..offset + value.len()].copy_from_slice(value);
}
