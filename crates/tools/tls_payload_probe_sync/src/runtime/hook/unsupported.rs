pub(super) fn install(target: usize, replacement: usize) -> Result<usize, String> {
    let _ = (target, replacement);
    Err(format!(
        "tls_payload_probe_sync native inline hooks are not implemented for {}",
        std::env::consts::ARCH
    ))
}
