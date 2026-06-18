//! OpenSSL/BoringSSL ABI hook handlers.

use std::collections::BTreeSet;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

mod interpose;

use crate::runtime::config::{self, RuntimePlan};
use crate::runtime::tls::dynamic::binding;
use crate::runtime::tls::dynamic::core::capture::{
    SslReadExFn, SslReadFn, SslWriteExFn, SslWriteFn, abort_runtime, ssl_read_ex_with,
    ssl_read_with, ssl_write_ex_with, ssl_write_with,
};
use crate::runtime::{hook, maps, output, rustls};
use interpose::{
    interposed_ssl_read, interposed_ssl_read_ex, interposed_ssl_write, interposed_ssl_write_ex,
};

static SSL_WRITE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static SSL_WRITE_EX_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static SSL_READ_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static SSL_READ_EX_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static INSTALL_STATE: OnceLock<Mutex<InstallState>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HookInstallStatus {
    Installed,
    DuplicateSkipped,
}

#[derive(Default)]
struct InstallState {
    installed_binaries: BTreeSet<PathBuf>,
    ssl_binary: Option<PathBuf>,
    openssl_interpose_binary: Option<PathBuf>,
    rustls_binary: Option<PathBuf>,
}

pub(super) fn install_plan(plan: &RuntimePlan) -> Result<HookInstallStatus, String> {
    let binary = canonical(&plan.binary);
    let uses_ssl = plan_uses_ssl_symbols(plan);
    let uses_rustls = plan_uses_rustls_symbols(plan);
    let installs_inline = should_install_inline_hooks(plan);
    let uses_interpose = uses_openssl_interpose(plan);
    if installs_inline && binding::is_audit_namespace()? {
        if let Some(config) = config::get() {
            config.register_plan(plan)?;
        }
        return Ok(HookInstallStatus::DuplicateSkipped);
    }
    let mut state = INSTALL_STATE
        .get_or_init(|| Mutex::new(InstallState::default()))
        .lock()
        .map_err(|_| "hook install state mutex poisoned".to_string())?;
    if (!installs_inline && state.openssl_interpose_binary.as_ref() == Some(&binary))
        || openssl_interpose_already_covers(&state, plan, &binary)
        || (installs_inline && state.installed_binaries.contains(&binary))
        || (installs_inline && uses_ssl && state.ssl_binary.is_some())
        || (uses_rustls && state.rustls_binary.is_some())
    {
        return Ok(HookInstallStatus::DuplicateSkipped);
    }
    let mut installed_inline_points = false;
    if installs_inline {
        installed_inline_points = install_plan_points(plan)?;
    }
    if installs_inline && installed_inline_points {
        state.installed_binaries.insert(binary.clone());
    }
    if uses_ssl {
        if uses_interpose {
            state.openssl_interpose_binary = Some(binary.clone());
        } else {
            state.ssl_binary = Some(binary.clone());
        }
    }
    if uses_rustls {
        state.rustls_binary = Some(binary);
    }
    if let Some(config) = config::get() {
        config.register_plan(plan)?;
    }
    if installs_inline && !installed_inline_points {
        Ok(HookInstallStatus::DuplicateSkipped)
    } else {
        Ok(HookInstallStatus::Installed)
    }
}

pub(super) fn dynamic_binding_covers_plan(plan: &RuntimePlan) -> bool {
    plan.provider == "openssl" && plan_uses_ssl_symbols(plan) && is_ssl_abi_shared_library(plan)
}

pub(super) fn register_dynamic_binding_plan(plan: &RuntimePlan) -> Result<(), String> {
    if dynamic_binding_covers_plan(plan) {
        let binary = canonical(&plan.binary);
        let mut state = INSTALL_STATE
            .get_or_init(|| Mutex::new(InstallState::default()))
            .lock()
            .map_err(|_| "hook install state mutex poisoned".to_string())?;
        state.openssl_interpose_binary = Some(binary);
    }
    if let Some(config) = config::get() {
        config.register_plan(plan)?;
    }
    Ok(())
}

fn should_install_inline_hooks(plan: &RuntimePlan) -> bool {
    !uses_openssl_interpose(plan)
}

fn uses_openssl_interpose(plan: &RuntimePlan) -> bool {
    plan.provider == "openssl" && !plan.requires_inline_hooks()
}

fn openssl_interpose_already_covers(
    state: &InstallState,
    plan: &RuntimePlan,
    binary: &Path,
) -> bool {
    plan.provider == "openssl"
        && plan.requires_inline_hooks()
        && state
            .openssl_interpose_binary
            .as_ref()
            .is_some_and(|path| path == binary)
}

fn install_plan_points(plan: &RuntimePlan) -> Result<bool, String> {
    let skip_ssl_read = plan.provider == "openssl"
        && plan
            .points
            .iter()
            .any(|point| point.symbol.as_str() == "SSL_read_ex");
    let mut installed = false;
    for point in &plan.points {
        if skip_ssl_read && point.symbol == "SSL_read" {
            continue;
        }
        let mut address = maps::runtime_address(&plan.binary, point.file_offset)?;
        if plan.provider == "openssl" && point.symbol == "SSL_read_ex" {
            address = openssl_ssl_read_ex_impl(address);
        }
        let trampoline = if rustls::can_handle(&point.symbol) {
            match rustls::install(&point.symbol, address)? {
                rustls::InstallStatus::Installed { trampoline } => trampoline,
                rustls::InstallStatus::DuplicateSkipped { owner } => {
                    if config::get().is_some_and(|config| config.should_print_target()) {
                        output::event_line(&format!(
                            "sync_hook: provider={} binary={} symbol={} direction={} address=0x{address:x} duplicate_owner=0x{owner:x}\n",
                            plan.provider,
                            plan.binary.display(),
                            point.symbol,
                            point.direction.as_str(),
                        ));
                    }
                    continue;
                }
            }
        } else {
            let replacement = replacement_for_symbol(&point.symbol)?;
            if let Some(owner) = hook::installed_actrail_jump_target(address) {
                if config::get().is_some_and(|config| config.should_print_target()) {
                    output::event_line(&format!(
                        "sync_hook: provider={} binary={} symbol={} direction={} address=0x{address:x} duplicate_owner=0x{owner:x}\n",
                        plan.provider,
                        plan.binary.display(),
                        point.symbol,
                        point.direction.as_str(),
                    ));
                }
                continue;
            }
            let trampoline =
                hook::install(address, replacement, |trampoline| {
                    set_original(&point.symbol, trampoline)
                })
                .map_err(|error| {
                    format!(
                        "install TLS hook provider={} symbol={} binary={} address=0x{address:x}: {error}",
                        plan.provider,
                        point.symbol,
                        plan.binary.display()
                    )
                })?;
            trampoline
        };
        installed = true;
        if config::get().is_some_and(|config| config.should_print_target()) {
            output::event_line(&format!(
                "sync_hook: provider={} binary={} symbol={} direction={} address=0x{address:x} trampoline=0x{trampoline:x}\n",
                plan.provider,
                plan.binary.display(),
                point.symbol,
                point.direction.as_str(),
            ));
        }
    }
    Ok(installed)
}

fn plan_uses_ssl_symbols(plan: &RuntimePlan) -> bool {
    plan.points.iter().any(|point| {
        matches!(
            point.symbol.as_str(),
            "SSL_write" | "SSL_write_ex" | "SSL_read" | "SSL_read_ex"
        )
    })
}

fn is_ssl_abi_shared_library(plan: &RuntimePlan) -> bool {
    let Some(file_name) = plan.binary.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    file_name == "libssl.so" || file_name.starts_with("libssl.so.")
}

fn plan_uses_rustls_symbols(plan: &RuntimePlan) -> bool {
    plan.points
        .iter()
        .any(|point| rustls::can_handle(&point.symbol))
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SSL_write(
    ssl: *mut c_void,
    buffer: *const c_void,
    length: libc::c_int,
) -> libc::c_int {
    let capture = dynamic_tls_capture_enabled();
    let original = unsafe { interposed_ssl_write(use_configured_openssl_symbol()) };
    if capture {
        unsafe { ssl_write_with(original, ssl, buffer, length) }
    } else {
        unsafe { original(ssl, buffer, length) }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SSL_write_ex(
    ssl: *mut c_void,
    buffer: *const c_void,
    length: usize,
    written: *mut usize,
) -> libc::c_int {
    let capture = dynamic_tls_capture_enabled();
    let original = unsafe { interposed_ssl_write_ex(use_configured_openssl_symbol()) };
    if capture {
        unsafe { ssl_write_ex_with(original, ssl, buffer, length, written) }
    } else {
        unsafe { original(ssl, buffer, length, written) }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SSL_read(
    ssl: *mut c_void,
    buffer: *mut c_void,
    length: libc::c_int,
) -> libc::c_int {
    let capture = dynamic_tls_capture_enabled();
    let original = unsafe { interposed_ssl_read(use_configured_openssl_symbol()) };
    if capture {
        unsafe { ssl_read_with(original, ssl, buffer, length) }
    } else {
        unsafe { original(ssl, buffer, length) }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SSL_read_ex(
    ssl: *mut c_void,
    buffer: *mut c_void,
    length: usize,
    read_bytes: *mut usize,
) -> libc::c_int {
    let capture = dynamic_tls_capture_enabled();
    let original = unsafe { interposed_ssl_read_ex(use_configured_openssl_symbol()) };
    if capture {
        unsafe { ssl_read_ex_with(original, ssl, buffer, length, read_bytes) }
    } else {
        unsafe { original(ssl, buffer, length, read_bytes) }
    }
}

fn dynamic_tls_capture_enabled() -> bool {
    super::retry_initialize_after_loader_event();
    config::get().is_some_and(|config| config.has_registered_plan())
}

fn use_configured_openssl_symbol() -> bool {
    configured_openssl_binary().is_some()
}

pub(in crate::runtime) fn is_exported_ssl_entry(address: usize) -> bool {
    address == SSL_write as *const () as usize
        || address == SSL_write_ex as *const () as usize
        || address == SSL_read as *const () as usize
        || address == SSL_read_ex as *const () as usize
}

fn openssl_ssl_read_ex_impl(address: usize) -> usize {
    let bytes = unsafe { std::slice::from_raw_parts(address as *const u8, 16) };
    let wrapper_prefix = [0xf3, 0x0f, 0x1e, 0xfa, 0x55, 0x48, 0x89, 0xe5, 0xe8];
    if !bytes.starts_with(&wrapper_prefix) {
        return address;
    }
    let mut displacement = [0_u8; 4];
    displacement.copy_from_slice(&bytes[9..13]);
    let relative = i32::from_le_bytes(displacement) as isize;
    address.wrapping_add(13).wrapping_add_signed(relative)
}

fn replacement_for_symbol(symbol: &str) -> Result<usize, String> {
    match symbol {
        "SSL_write" => Ok(hook_ssl_write as *const () as usize),
        "SSL_write_ex" => Ok(hook_ssl_write_ex as *const () as usize),
        "SSL_read" => Ok(hook_ssl_read as *const () as usize),
        "SSL_read_ex" => Ok(hook_ssl_read_ex as *const () as usize),
        _ => Err(format!(
            "sync native rewrite does not support hook symbol yet: {symbol}"
        )),
    }
}

fn set_original(symbol: &str, trampoline: usize) -> Result<(), String> {
    if trampoline == 0 {
        return Err(format!("refuse null original trampoline for {symbol}"));
    }
    match symbol {
        "SSL_write" => SSL_WRITE_ORIGINAL.store(trampoline, Ordering::Release),
        "SSL_write_ex" => SSL_WRITE_EX_ORIGINAL.store(trampoline, Ordering::Release),
        "SSL_read" => SSL_READ_ORIGINAL.store(trampoline, Ordering::Release),
        "SSL_read_ex" => SSL_READ_EX_ORIGINAL.store(trampoline, Ordering::Release),
        _ => return Err(format!("unknown original symbol slot: {symbol}")),
    }
    Ok(())
}

unsafe extern "C" fn hook_ssl_write(
    ssl: *mut c_void,
    buffer: *const c_void,
    length: libc::c_int,
) -> libc::c_int {
    let original = unsafe { original_ssl_write() };
    unsafe { ssl_write_with(original, ssl, buffer, length) }
}

unsafe extern "C" fn hook_ssl_write_ex(
    ssl: *mut c_void,
    buffer: *const c_void,
    length: usize,
    written: *mut usize,
) -> libc::c_int {
    let original = unsafe { original_ssl_write_ex() };
    unsafe { ssl_write_ex_with(original, ssl, buffer, length, written) }
}

unsafe extern "C" fn hook_ssl_read(
    ssl: *mut c_void,
    buffer: *mut c_void,
    length: libc::c_int,
) -> libc::c_int {
    let original = unsafe { original_ssl_read() };
    unsafe { ssl_read_with(original, ssl, buffer, length) }
}

unsafe extern "C" fn hook_ssl_read_ex(
    ssl: *mut c_void,
    buffer: *mut c_void,
    length: usize,
    read_bytes: *mut usize,
) -> libc::c_int {
    let original = unsafe { original_ssl_read_ex() };
    unsafe { ssl_read_ex_with(original, ssl, buffer, length, read_bytes) }
}

fn configured_openssl_binary() -> Option<PathBuf> {
    INSTALL_STATE
        .get()
        .and_then(|state| state.lock().ok())
        .and_then(|state| state.openssl_interpose_binary.clone())
}

unsafe fn original_ssl_write() -> SslWriteFn {
    let address = SSL_WRITE_ORIGINAL.load(Ordering::Acquire);
    if address == 0 {
        abort_runtime("SSL_write original is not installed");
    }
    unsafe { std::mem::transmute(address) }
}

unsafe fn original_ssl_write_ex() -> SslWriteExFn {
    let address = SSL_WRITE_EX_ORIGINAL.load(Ordering::Acquire);
    if address == 0 {
        abort_runtime("SSL_write_ex original is not installed");
    }
    unsafe { std::mem::transmute(address) }
}

unsafe fn original_ssl_read() -> SslReadFn {
    let address = SSL_READ_ORIGINAL.load(Ordering::Acquire);
    if address == 0 {
        abort_runtime("SSL_read original is not installed");
    }
    unsafe { std::mem::transmute(address) }
}

unsafe fn original_ssl_read_ex() -> SslReadExFn {
    let address = SSL_READ_EX_ORIGINAL.load(Ordering::Acquire);
    if address == 0 {
        abort_runtime("SSL_read_ex original is not installed");
    }
    unsafe { std::mem::transmute(address) }
}
