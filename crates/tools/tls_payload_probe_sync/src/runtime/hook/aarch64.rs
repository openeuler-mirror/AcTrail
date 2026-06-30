#[path = "aarch64_relocate.rs"]
mod aarch64_relocate;

use aarch64_relocate::{RelocationKind, RelocationPlan};

const INSTRUCTION_BYTES: usize = 4;
const JUMP_PATCH_BYTES: usize = 16;
const LOAD_IMMEDIATE_BYTES: usize = 16;
const ABSOLUTE_JUMP_BYTES: usize = 16;
const ABSOLUTE_CALL_BYTES: usize = 20;
const EXTERNAL_BRANCH_ISLAND_BYTES: usize = INSTRUCTION_BYTES + ABSOLUTE_JUMP_BYTES;
const TRAMPOLINE_TAIL_JUMP_BYTES: usize = ABSOLUTE_JUMP_BYTES;
const AARCH64_NOP: u32 = 0xd503_201f;
const LDR_X16_LITERAL_8: u32 = 0x5800_0050;
const LDR_X16_LITERAL_12: u32 = 0x5800_0070;
const BR_X16: u32 = 0xd61f_0200;
const BLR_X16: u32 = 0xd63f_0200;
const B_SKIP_LITERAL: u32 = 0x1400_0003;
const BRANCH26_IMMEDIATE_MASK: u32 = 0x03ff_ffff;
const BRANCH19_IMMEDIATE_MASK: u32 = 0x00ff_ffe0;
const TEST_BRANCH_IMMEDIATE_MASK: u32 = 0x0007_ffe0;
const BRANCH_CONDITION_MASK: u32 = 0x0000_000f;
const COMPARE_TEST_INVERT_BIT: u32 = 0x0100_0000;

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
        write_jump(trampoline + written, target + plan.stolen_len);
        clear_instruction_cache(trampoline, written + TRAMPOLINE_TAIL_JUMP_BYTES)?;
    }
    before_patch(trampoline)?;
    unsafe {
        patch_target(target, replacement, plan.stolen_len)?;
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
        if stolen > JUMP_PATCH_BYTES {
            write_nops(target + JUMP_PATCH_BYTES, stolen - JUMP_PATCH_BYTES)?;
        }
    }
    clear_instruction_cache(target, stolen)?;
    protect(page_start, page_len, libc::PROT_READ | libc::PROT_EXEC)?;
    Ok(())
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
                    INSTRUCTION_BYTES,
                );
            },
            RelocationKind::Adr { register, target } => unsafe {
                write_load_immediate(destination_address, register, target)?;
            },
            RelocationKind::Bl { target } => {
                if let Some(relocated_target) =
                    relocated_branch_target(destination, plan, target, "BL")?
                {
                    unsafe {
                        write_instruction(
                            destination_address,
                            encode_unconditional_branch(
                                destination_address,
                                instruction.raw,
                                relocated_target,
                                "BL",
                            )?,
                        );
                    }
                } else {
                    unsafe {
                        write_absolute_call(destination_address, target);
                    }
                }
            }
            RelocationKind::Branch { target } => {
                if let Some(relocated_target) =
                    relocated_branch_target(destination, plan, target, "B")?
                {
                    unsafe {
                        write_instruction(
                            destination_address,
                            encode_unconditional_branch(
                                destination_address,
                                instruction.raw,
                                relocated_target,
                                "B",
                            )?,
                        );
                    }
                } else {
                    unsafe {
                        write_jump(destination_address, target);
                    }
                }
            }
            RelocationKind::ConditionalBranch { target } => {
                if let Some(relocated_target) =
                    relocated_branch_target(destination, plan, target, "B.cond")?
                {
                    unsafe {
                        write_instruction(
                            destination_address,
                            encode_conditional_branch(
                                destination_address,
                                instruction.raw,
                                relocated_target,
                                "B.cond",
                            )?,
                        );
                    }
                } else {
                    unsafe {
                        write_external_conditional_branch(
                            destination_address,
                            instruction.raw,
                            target,
                            invert_condition,
                            encode_conditional_branch,
                            "B.cond",
                        )?;
                    }
                }
            }
            RelocationKind::CompareBranch { target } => {
                if let Some(relocated_target) =
                    relocated_branch_target(destination, plan, target, "CBZ/CBNZ")?
                {
                    unsafe {
                        write_instruction(
                            destination_address,
                            encode_compare_branch(
                                destination_address,
                                instruction.raw,
                                relocated_target,
                                "CBZ/CBNZ",
                            )?,
                        );
                    }
                } else {
                    unsafe {
                        write_external_conditional_branch(
                            destination_address,
                            instruction.raw,
                            target,
                            invert_compare_test_branch,
                            encode_compare_branch,
                            "CBZ/CBNZ",
                        )?;
                    }
                }
            }
            RelocationKind::TestBranch { target } => {
                if let Some(relocated_target) =
                    relocated_branch_target(destination, plan, target, "TBZ/TBNZ")?
                {
                    unsafe {
                        write_instruction(
                            destination_address,
                            encode_test_branch(
                                destination_address,
                                instruction.raw,
                                relocated_target,
                                "TBZ/TBNZ",
                            )?,
                        );
                    }
                } else {
                    unsafe {
                        write_external_conditional_branch(
                            destination_address,
                            instruction.raw,
                            target,
                            invert_compare_test_branch,
                            encode_test_branch,
                            "TBZ/TBNZ",
                        )?;
                    }
                }
            }
        }
    }
    Ok(plan.trampoline_code_len)
}

unsafe fn write_jump(address: usize, target: usize) {
    let cursor = address as *mut u8;
    unsafe {
        std::ptr::write_unaligned(cursor.cast::<u32>(), LDR_X16_LITERAL_8);
        std::ptr::write_unaligned(cursor.add(4).cast::<u32>(), BR_X16);
        std::ptr::write_unaligned(cursor.add(8).cast::<u64>(), target as u64);
    }
}

unsafe fn write_instruction(address: usize, instruction: u32) {
    unsafe {
        std::ptr::write_unaligned((address as *mut u8).cast::<u32>(), instruction);
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

unsafe fn write_external_conditional_branch(
    address: usize,
    instruction: u32,
    target: usize,
    invert: fn(u32, &str) -> Result<u32, String>,
    encode: fn(usize, u32, usize, &str) -> Result<u32, String>,
    label: &str,
) -> Result<(), String> {
    let skip_island = address
        .checked_add(EXTERNAL_BRANCH_ISLAND_BYTES)
        .ok_or_else(|| format!("{label} external island skip overflow"))?;
    let inverted = invert(instruction, label)?;
    unsafe {
        write_instruction(address, encode(address, inverted, skip_island, label)?);
        // x16/IP0 is the scratch register for generated absolute branch
        // islands. The fallthrough path skips this island and preserves x16.
        write_jump(address + INSTRUCTION_BYTES, target);
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

unsafe fn write_nops(address: usize, length: usize) -> Result<(), String> {
    if length % INSTRUCTION_BYTES != 0 {
        return Err(format!(
            "aarch64 NOP fill is not instruction aligned: {length} bytes"
        ));
    }
    let count = length / INSTRUCTION_BYTES;
    for index in 0..count {
        unsafe {
            write_instruction(address + index * INSTRUCTION_BYTES, AARCH64_NOP);
        }
    }
    Ok(())
}

fn relocated_branch_target(
    destination: usize,
    plan: &RelocationPlan,
    target: usize,
    label: &str,
) -> Result<Option<usize>, String> {
    let Some(offset) = plan.relocated_offset_for_target(target)? else {
        return Ok(None);
    };
    destination
        .checked_add(offset)
        .ok_or_else(|| format!("{label} relocated target overflow"))
        .map(Some)
}

fn encode_unconditional_branch(
    address: usize,
    instruction: u32,
    target: usize,
    label: &str,
) -> Result<u32, String> {
    let immediate = scaled_branch_immediate(address, target, 26, label)?;
    Ok((instruction & !BRANCH26_IMMEDIATE_MASK) | immediate)
}

fn encode_conditional_branch(
    address: usize,
    instruction: u32,
    target: usize,
    label: &str,
) -> Result<u32, String> {
    let immediate = scaled_branch_immediate(address, target, 19, label)?;
    Ok((instruction & !BRANCH19_IMMEDIATE_MASK) | (immediate << 5))
}

fn encode_compare_branch(
    address: usize,
    instruction: u32,
    target: usize,
    label: &str,
) -> Result<u32, String> {
    let immediate = scaled_branch_immediate(address, target, 19, label)?;
    Ok((instruction & !BRANCH19_IMMEDIATE_MASK) | (immediate << 5))
}

fn encode_test_branch(
    address: usize,
    instruction: u32,
    target: usize,
    label: &str,
) -> Result<u32, String> {
    let immediate = scaled_branch_immediate(address, target, 14, label)?;
    Ok((instruction & !TEST_BRANCH_IMMEDIATE_MASK) | (immediate << 5))
}

fn invert_condition(instruction: u32, label: &str) -> Result<u32, String> {
    let condition = instruction & BRANCH_CONDITION_MASK;
    if condition >= 0x0e {
        return Err(format!(
            "{label} condition 0x{condition:x} cannot be inverted for external island"
        ));
    }
    Ok((instruction & !BRANCH_CONDITION_MASK) | (condition ^ 1))
}

fn invert_compare_test_branch(instruction: u32, _label: &str) -> Result<u32, String> {
    Ok(instruction ^ COMPARE_TEST_INVERT_BIT)
}

fn scaled_branch_immediate(
    address: usize,
    target: usize,
    bits: u32,
    label: &str,
) -> Result<u32, String> {
    let displacement = (target as i128)
        .checked_sub(address as i128)
        .ok_or_else(|| format!("{label} displacement overflow"))?;
    if displacement % i128::from(INSTRUCTION_BYTES as u8) != 0 {
        return Err(format!(
            "{label} target is not instruction aligned: displacement={displacement}"
        ));
    }
    let immediate = displacement / i128::from(INSTRUCTION_BYTES as u8);
    let min = -(1_i128 << (bits - 1));
    let max = (1_i128 << (bits - 1)) - 1;
    if immediate < min || immediate > max {
        return Err(format!(
            "{label} displacement out of signed {bits}-bit branch range: {displacement}"
        ));
    }
    let mask = (1_i128 << bits) - 1;
    Ok((immediate & mask) as u32)
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
#[path = "aarch64_tests.rs"]
mod tests;
