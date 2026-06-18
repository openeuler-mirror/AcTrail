pub(super) fn install(
    target: usize,
    replacement: usize,
    before_patch: impl FnOnce(usize) -> Result<(), String>,
) -> Result<usize, String> {
    let _ = (target, replacement, before_patch);
    Err(format!(
        "tls_payload_probe_sync native inline hooks are not implemented for {}",
        std::env::consts::ARCH
    ))
}

pub(super) fn installed_jump_target(target: usize) -> Option<usize> {
    let _ = target;
    None
}
