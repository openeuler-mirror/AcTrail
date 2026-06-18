const JUMP_PATCH_BYTES: usize = 16;
const STOLEN_BYTES: usize = 16;
const STOLEN_INSTRUCTIONS: usize = STOLEN_BYTES / 4;
const LOAD_IMMEDIATE_BYTES: usize = 16;
const ABSOLUTE_CALL_BYTES: usize = 20;
const LDR_X16_LITERAL_8: u32 = 0x5800_0050;
const LDR_X16_LITERAL_12: u32 = 0x5800_0070;
const BR_X16: u32 = 0xd61f_0200;
const BLR_X16: u32 = 0xd63f_0200;
const B_SKIP_LITERAL: u32 = 0x1400_0003;

pub(super) fn install(target: usize, replacement: usize) -> Result<usize, String> {
    let trampoline =
        allocate_trampoline(STOLEN_INSTRUCTIONS * ABSOLUTE_CALL_BYTES + JUMP_PATCH_BYTES)?;
    unsafe {
        let written = write_trampoline(target, trampoline)?;
        write_jump(trampoline + written, target + STOLEN_BYTES);
        clear_instruction_cache(trampoline, written + JUMP_PATCH_BYTES)?;
        patch_target(target, replacement)?;
    }
    Ok(trampoline)
}

pub(super) fn installed_jump_target(target: usize) -> Option<usize> {
    let bytes = unsafe { std::slice::from_raw_parts(target as *const u8, JUMP_PATCH_BYTES) };
    let first = u32::from_le_bytes(bytes[0..4].try_into().ok()?);
    let second = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
    if first != LDR_X16_LITERAL_8 || second != BR_X16 {
        return None;
    }
    Some(u64::from_le_bytes(bytes[8..16].try_into().ok()?) as usize)
}

unsafe fn patch_target(target: usize, replacement: usize) -> Result<(), String> {
    let page_size = page_size()?;
    let page_start = target & !(page_size - 1);
    let page_end = target
        .checked_add(STOLEN_BYTES)
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
    clear_instruction_cache(target, STOLEN_BYTES)?;
    protect(page_start, page_len, libc::PROT_READ | libc::PROT_EXEC)?;
    Ok(())
}

unsafe fn write_trampoline(source: usize, destination: usize) -> Result<usize, String> {
    if source % 4 != 0 {
        return Err(format!(
            "aarch64 hook target is not 4-byte aligned: 0x{source:x}"
        ));
    }
    let mut source_offset = 0usize;
    let mut destination_offset = 0usize;
    while source_offset < STOLEN_BYTES {
        let address = source + source_offset;
        let instruction = unsafe { std::ptr::read_unaligned(address as *const u32) }.to_le();
        if let Some(target) = adr_target(address, instruction) {
            unsafe {
                write_load_immediate(
                    destination + destination_offset,
                    register(instruction),
                    target,
                )?;
            }
            destination_offset += LOAD_IMMEDIATE_BYTES;
        } else if let Some(target) = bl_target(address, instruction) {
            unsafe {
                write_absolute_call(destination + destination_offset, target);
            }
            destination_offset += ABSOLUTE_CALL_BYTES;
        } else if let Some(reason) = relocation_error(instruction) {
            return Err(format!(
                "aarch64 instruction cannot be relocated in trampoline at +{}: {reason} 0x{instruction:08x}",
                source_offset
            ));
        } else {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    address as *const u8,
                    (destination + destination_offset) as *mut u8,
                    4,
                );
            }
            destination_offset += 4;
        }
        source_offset += 4;
    }
    Ok(destination_offset)
}

unsafe fn write_jump(address: usize, target: usize) {
    let cursor = address as *mut u8;
    unsafe {
        std::ptr::write_unaligned(cursor.cast::<u32>(), LDR_X16_LITERAL_8);
        std::ptr::write_unaligned(cursor.add(4).cast::<u32>(), BR_X16);
        std::ptr::write_unaligned(cursor.add(8).cast::<u64>(), target as u64);
    }
}

unsafe fn write_load_immediate(address: usize, register: u8, value: usize) -> Result<(), String> {
    if register > 30 {
        return Err("cannot rewrite adr/adrp targeting xzr/sp".to_string());
    }
    let cursor = address as *mut u8;
    unsafe {
        std::ptr::write_unaligned(cursor.cast::<u32>(), ldr_x_literal_8(register));
        std::ptr::write_unaligned(cursor.add(4).cast::<u32>(), B_SKIP_LITERAL);
        std::ptr::write_unaligned(cursor.add(8).cast::<u64>(), value as u64);
    }
    Ok(())
}

unsafe fn write_absolute_call(address: usize, target: usize) {
    let cursor = address as *mut u8;
    unsafe {
        std::ptr::write_unaligned(cursor.cast::<u32>(), LDR_X16_LITERAL_12);
        std::ptr::write_unaligned(cursor.add(4).cast::<u32>(), BLR_X16);
        std::ptr::write_unaligned(cursor.add(8).cast::<u32>(), B_SKIP_LITERAL);
        std::ptr::write_unaligned(cursor.add(12).cast::<u64>(), target as u64);
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

fn relocation_error(instruction: u32) -> Option<&'static str> {
    if (instruction & 0xfc00_0000) == 0x1400_0000 {
        return Some("relative branch");
    }
    if (instruction & 0xff00_0010) == 0x5400_0000 {
        return Some("conditional relative branch");
    }
    if (instruction & 0x7e00_0000) == 0x3400_0000 {
        return Some("compare relative branch");
    }
    if (instruction & 0x7e00_0000) == 0x3600_0000 {
        return Some("test relative branch");
    }
    if (instruction & 0xfe00_0000) == 0xd600_0000 {
        return Some("register branch");
    }
    if (instruction & 0x3b00_0000) == 0x1800_0000 {
        return Some("literal pc-relative load");
    }
    None
}

fn adr_target(address: usize, instruction: u32) -> Option<usize> {
    if (instruction & 0x1f00_0000) != 0x1000_0000 {
        return None;
    }
    let immediate = adr_immediate(instruction);
    let target = if (instruction & 0x8000_0000) == 0 {
        address as i128 + immediate
    } else {
        (address & !0xfff) as i128 + (immediate << 12)
    };
    checked_target(target)
}

fn adr_immediate(instruction: u32) -> i128 {
    let immediate = (((instruction >> 5) & 0x7ffff) << 2) | ((instruction >> 29) & 0x3);
    sign_extend(immediate as u64, 21)
}

fn bl_target(address: usize, instruction: u32) -> Option<usize> {
    if (instruction & 0xfc00_0000) != 0x9400_0000 {
        return None;
    }
    let immediate = sign_extend((instruction & 0x03ff_ffff) as u64, 26) << 2;
    checked_target(address as i128 + immediate)
}

fn checked_target(target: i128) -> Option<usize> {
    if target < 0 || target > usize::MAX as i128 {
        return None;
    }
    Some(target as usize)
}

fn sign_extend(value: u64, bits: u32) -> i128 {
    let shift = 128 - bits;
    ((value as i128) << shift) >> shift
}

fn register(instruction: u32) -> u8 {
    (instruction & 0x1f) as u8
}

fn ldr_x_literal_8(register: u8) -> u32 {
    0x5800_0040 | u32::from(register)
}

fn clear_instruction_cache(start: usize, length: usize) -> Result<(), String> {
    if length == 0 {
        return Ok(());
    }
    let end = start
        .checked_add(length)
        .ok_or_else(|| "instruction cache range overflow".to_string())?;
    let data_line = data_cache_line_size();
    let mut current = start & !(data_line - 1);
    while current < end {
        unsafe {
            std::arch::asm!("dc cvau, {}", in(reg) current, options(nostack, preserves_flags));
        }
        current = current
            .checked_add(data_line)
            .ok_or_else(|| "data cache line walk overflow".to_string())?;
    }
    unsafe {
        std::arch::asm!("dsb ish", options(nostack, preserves_flags));
    }
    let instruction_line = instruction_cache_line_size();
    let mut current = start & !(instruction_line - 1);
    while current < end {
        unsafe {
            std::arch::asm!("ic ivau, {}", in(reg) current, options(nostack, preserves_flags));
        }
        current = current
            .checked_add(instruction_line)
            .ok_or_else(|| "instruction cache line walk overflow".to_string())?;
    }
    unsafe {
        std::arch::asm!("dsb ish", options(nostack, preserves_flags));
        std::arch::asm!("isb", options(nostack, preserves_flags));
    }
    Ok(())
}

fn data_cache_line_size() -> usize {
    4usize << ((ctr_el0() >> 16) & 0xf)
}

fn instruction_cache_line_size() -> usize {
    4usize << (ctr_el0() & 0xf)
}

fn ctr_el0() -> usize {
    let value: usize;
    unsafe {
        std::arch::asm!("mrs {}, ctr_el0", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jump_patch_loads_absolute_target_and_branches_via_x16() {
        let mut bytes = [0_u8; 16];
        let target = 0x1122_3344_5566_7788usize;

        unsafe {
            write_jump(bytes.as_mut_ptr() as usize, target);
        }

        assert_eq!(
            u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            LDR_X16_LITERAL_8
        );
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), BR_X16);
        assert_eq!(
            u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            target as u64
        );
    }

    #[test]
    fn load_immediate_patch_loads_literal_and_skips_over_data() {
        let mut bytes = [0_u8; 16];
        let target = 0x8877_6655_4433_2211usize;

        unsafe {
            write_load_immediate(bytes.as_mut_ptr() as usize, 3, target).unwrap();
        }

        assert_eq!(
            u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            ldr_x_literal_8(3)
        );
        assert_eq!(
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            B_SKIP_LITERAL
        );
        assert_eq!(
            u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            target as u64
        );
    }

    #[test]
    fn absolute_call_patch_returns_to_code_after_literal() {
        let mut bytes = [0_u8; 20];
        let target = 0x1122_3344_5566_7788usize;

        unsafe {
            write_absolute_call(bytes.as_mut_ptr() as usize, target);
        }

        assert_eq!(
            u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            LDR_X16_LITERAL_12
        );
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), BLR_X16);
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            B_SKIP_LITERAL
        );
        assert_eq!(
            u64::from_le_bytes(bytes[12..20].try_into().unwrap()),
            target as u64
        );
    }

    #[test]
    fn relocation_decodes_openssl_adrp_and_bl_targets() {
        assert_eq!(adr_target(0x364e4, 0xb000_0343), Some(0x9f000));
        assert_eq!(bl_target(0x36578, 0x97ff_ff66), Some(0x36310));
    }

    #[test]
    fn relocation_filter_rejects_unsupported_control_flow_and_literal_loads() {
        for instruction in [
            0x1400_0000, // b
            0x5400_0000, // b.cond
            0x3400_0000, // cbz
            0x3700_0000, // tbnz
            0xd61f_0000, // br x0
            0x5800_0000, // ldr literal
        ] {
            assert!(
                relocation_error(instruction).is_some(),
                "instruction 0x{instruction:08x} should be rejected"
            );
        }
    }

    #[test]
    fn relocation_filter_allows_common_frame_setup_instructions() {
        for instruction in [
            0xa9bf_7bfd, // stp x29, x30, [sp, #-16]!
            0x9100_03fd, // mov x29, sp
            0x9400_0000, // bl
            0x9000_0000, // adrp
            0xf81f_0ff3, // str x19, [sp, #-16]!
            0xaa00_03f3, // mov x19, x0
        ] {
            assert!(
                relocation_error(instruction).is_none(),
                "instruction 0x{instruction:08x} should be allowed"
            );
        }
    }
}
