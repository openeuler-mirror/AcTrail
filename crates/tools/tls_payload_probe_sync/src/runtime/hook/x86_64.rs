mod relocate;

use relocate::{RelocationKind, RelocationPlan};

const JUMP_PATCH_BYTES: usize = 12;
const ABSOLUTE_CALL_BYTES: usize = 13;
const NEAR_JCC_BYTES: usize = 6;
const SHORT_JCC_BYTES: usize = 2;
const INDIRECT_ABSOLUTE_JUMP_BYTES: usize = 14;
const EXTERNAL_JCC_BYTES: usize = SHORT_JCC_BYTES + INDIRECT_ABSOLUTE_JUMP_BYTES;
const TRAMPOLINE_TAIL_JUMP_BYTES: usize = INDIRECT_ABSOLUTE_JUMP_BYTES;

pub(super) fn install(
    target: usize,
    replacement: usize,
    before_patch: impl FnOnce(usize) -> Result<(), String>,
) -> Result<usize, String> {
    let plan = RelocationPlan::new(target)?;
    let trampoline_size = plan
        .trampoline_code_len
        .checked_add(TRAMPOLINE_TAIL_JUMP_BYTES)
        .ok_or_else(|| "trampoline allocation size overflow".to_string())?;
    let trampoline = allocate_trampoline(trampoline_size)?;
    unsafe {
        let written = write_trampoline(target, trampoline, &plan)?;
        write_indirect_absolute_jump(trampoline + written, target + plan.stolen_len);
    }
    before_patch(trampoline)?;
    unsafe {
        patch_target(target, replacement, plan.stolen_len)?;
    }
    Ok(trampoline)
}

pub(super) fn installed_jump_target(target: usize) -> Option<usize> {
    let bytes = unsafe { std::slice::from_raw_parts(target as *const u8, JUMP_PATCH_BYTES) };
    if bytes[0] != 0x48 || bytes[1] != 0xb8 || bytes[10] != 0xff || bytes[11] != 0xe0 {
        return None;
    }
    let mut raw = [0_u8; 8];
    raw.copy_from_slice(&bytes[2..10]);
    Some(u64::from_le_bytes(raw) as usize)
}

unsafe fn write_trampoline(
    source: usize,
    destination: usize,
    plan: &RelocationPlan,
) -> Result<usize, String> {
    for instruction in &plan.instructions {
        let address = source
            .checked_add(instruction.source_offset)
            .ok_or_else(|| "trampoline source address overflow".to_string())?;
        let destination_address = destination
            .checked_add(instruction.emitted_offset)
            .ok_or_else(|| "trampoline destination address overflow".to_string())?;
        match instruction.kind {
            RelocationKind::Copy => unsafe {
                std::ptr::copy_nonoverlapping(
                    address as *const u8,
                    destination_address as *mut u8,
                    instruction.length,
                );
            },
            RelocationKind::CallRel32 { target } => unsafe {
                write_absolute_call(destination_address, target);
            },
            RelocationKind::Jcc { condition, target } => {
                if let Some(target_offset) = plan.relocated_offset_for_target(target)? {
                    let relocated_target = destination
                        .checked_add(target_offset)
                        .ok_or_else(|| "relocated JCC target overflow".to_string())?;
                    unsafe {
                        write_near_jcc(destination_address, condition, relocated_target)?;
                    }
                } else {
                    unsafe {
                        write_external_jcc(destination_address, condition, target);
                    }
                }
            }
        }
    }
    Ok(plan.trampoline_code_len)
}

unsafe fn patch_target(target: usize, replacement: usize, stolen: usize) -> Result<(), String> {
    let page_size = page_size()?;
    let page_start = target & !(page_size - 1);
    let page_end = target
        .checked_add(stolen)
        .and_then(|end| end.checked_add(page_size - 1))
        .map(|end| end & !(page_size - 1))
        .ok_or_else(|| "hook page range overflow".to_string())?;
    let page_len = page_end
        .checked_sub(page_start)
        .ok_or_else(|| "hook page range underflow".to_string())?;
    protect(
        page_start,
        page_len,
        libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
    )?;
    unsafe {
        write_jump(target, replacement);
    }
    if stolen > JUMP_PATCH_BYTES {
        unsafe {
            std::ptr::write_bytes(
                (target + JUMP_PATCH_BYTES) as *mut u8,
                0x90,
                stolen - JUMP_PATCH_BYTES,
            );
        }
    }
    protect(page_start, page_len, libc::PROT_READ | libc::PROT_EXEC)?;
    Ok(())
}

unsafe fn write_jump(address: usize, target: usize) {
    let cursor = address as *mut u8;
    unsafe {
        std::ptr::write(cursor, 0x48);
        std::ptr::write(cursor.add(1), 0xb8);
        std::ptr::write_unaligned(cursor.add(2).cast::<u64>(), target as u64);
        std::ptr::write(cursor.add(10), 0xff);
        std::ptr::write(cursor.add(11), 0xe0);
    }
}

unsafe fn write_absolute_call(address: usize, target: usize) {
    let cursor = address as *mut u8;
    unsafe {
        std::ptr::write(cursor, 0x49);
        std::ptr::write(cursor.add(1), 0xbb);
        std::ptr::write_unaligned(cursor.add(2).cast::<u64>(), target as u64);
        std::ptr::write(cursor.add(10), 0x41);
        std::ptr::write(cursor.add(11), 0xff);
        std::ptr::write(cursor.add(12), 0xd3);
    }
}

unsafe fn write_indirect_absolute_jump(address: usize, target: usize) {
    let cursor = address as *mut u8;
    unsafe {
        std::ptr::write(cursor, 0xff);
        std::ptr::write(cursor.add(1), 0x25);
        std::ptr::write_unaligned(cursor.add(2).cast::<u32>(), 0);
        std::ptr::write_unaligned(cursor.add(6).cast::<u64>(), target as u64);
    }
}

unsafe fn write_near_jcc(address: usize, condition: u8, target: usize) -> Result<(), String> {
    if condition > 0x0f {
        return Err(format!("invalid x86_64 JCC condition: 0x{condition:02x}"));
    }
    let next = address
        .checked_add(NEAR_JCC_BYTES)
        .ok_or_else(|| "near JCC next address overflow".to_string())?;
    let displacement = i32_displacement(next, target, "near JCC")?;
    let cursor = address as *mut u8;
    unsafe {
        std::ptr::write(cursor, 0x0f);
        std::ptr::write(cursor.add(1), 0x80 | condition);
        std::ptr::write_unaligned(cursor.add(2).cast::<i32>(), displacement);
    }
    Ok(())
}

unsafe fn write_external_jcc(address: usize, condition: u8, target: usize) {
    let cursor = address as *mut u8;
    unsafe {
        std::ptr::write(cursor, 0x70 | (condition ^ 1));
        std::ptr::write(cursor.add(1), INDIRECT_ABSOLUTE_JUMP_BYTES as u8);
        write_indirect_absolute_jump(address + SHORT_JCC_BYTES, target);
    }
}

fn allocate_trampoline(size: usize) -> Result<usize, String> {
    let pointer = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if pointer == libc::MAP_FAILED {
        return Err(format!(
            "allocate trampoline: {}",
            std::io::Error::last_os_error()
        ));
    }
    if pointer.is_null() {
        unsafe {
            libc::munmap(pointer, size);
        }
        return Err("allocate trampoline returned null address".to_string());
    }
    Ok(pointer as usize)
}

fn protect(address: usize, length: usize, protection: libc::c_int) -> Result<(), String> {
    let result = unsafe { libc::mprotect(address as *mut libc::c_void, length, protection) };
    if result != 0 {
        return Err(format!(
            "mprotect hook page: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn page_size() -> Result<usize, String> {
    let value = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if value <= 0 {
        return Err(format!(
            "sysconf page size: {}",
            std::io::Error::last_os_error()
        ));
    }
    usize::try_from(value).map_err(|error| format!("page size overflow: {error}"))
}

fn i32_displacement(next: usize, target: usize, label: &str) -> Result<i32, String> {
    let displacement = (target as i128)
        .checked_sub(next as i128)
        .ok_or_else(|| format!("{label} displacement overflow"))?;
    if displacement < i128::from(i32::MIN) || displacement > i128::from(i32::MAX) {
        return Err(format!(
            "{label} displacement out of i32 range: {displacement}"
        ));
    }
    Ok(displacement as i32)
}
