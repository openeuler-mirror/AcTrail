const JUMP_PATCH_BYTES: usize = 12;
const ABSOLUTE_CALL_BYTES: usize = 13;

pub(super) fn install(target: usize, replacement: usize) -> Result<usize, String> {
    let stolen = stolen_len_for_detour(target)?;
    let trampoline = allocate_trampoline(stolen * ABSOLUTE_CALL_BYTES + JUMP_PATCH_BYTES)?;
    unsafe {
        let written = write_trampoline(target, trampoline, stolen)?;
        write_jump(trampoline + written, target + stolen);
        patch_target(target, replacement, stolen)?;
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
    stolen: usize,
) -> Result<usize, String> {
    let mut source_offset = 0usize;
    let mut destination_offset = 0usize;
    while source_offset < stolen {
        let address = source + source_offset;
        let length = instruction_len(address)?;
        if let Some(target) = relative_call_target(address, length) {
            unsafe {
                write_absolute_call(destination + destination_offset, target);
            }
            destination_offset += ABSOLUTE_CALL_BYTES;
        } else {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    address as *const u8,
                    (destination + destination_offset) as *mut u8,
                    length,
                );
            }
            destination_offset += length;
        }
        source_offset = source_offset
            .checked_add(length)
            .ok_or_else(|| "trampoline source offset overflow".to_string())?;
    }
    Ok(destination_offset)
}

unsafe fn patch_target(target: usize, replacement: usize, stolen: usize) -> Result<(), String> {
    let page_size = page_size()?;
    let page_start = target & !(page_size - 1);
    let page_end = (target + stolen + page_size - 1) & !(page_size - 1);
    let page_len = page_end - page_start;
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

fn stolen_len_for_detour(address: usize) -> Result<usize, String> {
    let mut offset = 0usize;
    while offset < JUMP_PATCH_BYTES {
        let length = instruction_len(address + offset)?;
        offset = offset
            .checked_add(length)
            .ok_or_else(|| "instruction length overflow".to_string())?;
    }
    Ok(offset)
}

fn instruction_len(address: usize) -> Result<usize, String> {
    let bytes = unsafe { std::slice::from_raw_parts(address as *const u8, 16) };
    let mut cursor = 0usize;
    while is_prefix(bytes[cursor]) {
        cursor += 1;
    }
    let opcode = bytes[cursor];
    cursor += 1;
    if opcode == 0xe8 {
        return Ok(cursor + 4);
    }
    if is_relative_branch(opcode) {
        return Err(format!(
            "relative branch/call cannot be relocated: 0x{opcode:02x}"
        ));
    }
    if (0x50..=0x5f).contains(&opcode) || opcode == 0x90 {
        return Ok(cursor);
    }
    if opcode == 0x68 {
        return Ok(cursor + 4);
    }
    if opcode == 0x6a {
        return Ok(cursor + 1);
    }
    if (0xb8..=0xbf).contains(&opcode) {
        return Ok(cursor + mov_imm_size(bytes));
    }
    if matches!(
        opcode,
        0x04 | 0x05
            | 0x0c
            | 0x0d
            | 0x14
            | 0x15
            | 0x1c
            | 0x1d
            | 0x24
            | 0x25
            | 0x2c
            | 0x2d
            | 0x34
            | 0x35
            | 0x3c
            | 0x3d
    ) {
        return Ok(cursor + accumulator_imm_size(opcode));
    }
    if opcode == 0x0f {
        return two_byte_instruction_len(bytes, cursor);
    }
    if opcode_requires_modrm(opcode) {
        return modrm_instruction_len(bytes, cursor, immediate_size(opcode), opcode == 0xff);
    }
    Err(format!(
        "unsupported x86_64 opcode for trampoline: 0x{opcode:02x}"
    ))
}

fn two_byte_instruction_len(bytes: &[u8], cursor: usize) -> Result<usize, String> {
    let opcode = bytes[cursor];
    if (0x80..=0x8f).contains(&opcode) {
        return Err(format!(
            "relative two-byte branch cannot be relocated: 0x0f{opcode:02x}"
        ));
    }
    if opcode == 0x1e && matches!(bytes.get(cursor + 1), Some(0xfa | 0xfb)) {
        return Ok(cursor + 2);
    }
    let next = cursor + 1;
    if matches!(
        opcode,
        0x10 | 0x11 | 0x1f | 0x28 | 0x29 | 0x2e | 0x2f | 0x38 | 0x3a | 0x44
            ..=0x4f | 0xaf | 0xb6 | 0xb7 | 0xbe | 0xbf
    ) {
        return modrm_instruction_len(bytes, next, 0, false);
    }
    Err(format!(
        "unsupported two-byte x86_64 opcode for trampoline: 0x0f{opcode:02x}"
    ))
}

fn modrm_instruction_len(
    bytes: &[u8],
    cursor: usize,
    immediate: usize,
    reject_indirect_branch: bool,
) -> Result<usize, String> {
    let modrm = bytes[cursor];
    let mode = modrm >> 6;
    let reg = (modrm >> 3) & 0x7;
    let rm = modrm & 0x7;
    if reject_indirect_branch && matches!(reg, 2 | 4) {
        return Err("indirect call/jmp cannot be relocated".to_string());
    }
    if mode == 0 && rm == 5 {
        return Err("rip-relative memory operand cannot be relocated".to_string());
    }
    let mut length = cursor + 1;
    if rm == 4 && mode != 3 {
        let sib = bytes[length];
        length += 1;
        if mode == 0 && (sib & 0x7) == 5 {
            length += 4;
        }
    }
    length += match mode {
        0 => 0,
        1 => 1,
        2 => 4,
        3 => 0,
        _ => unreachable!(),
    };
    Ok(length + immediate)
}

fn opcode_requires_modrm(opcode: u8) -> bool {
    matches!(
        opcode,
        0x00..=0x03
            | 0x08..=0x0b
            | 0x10..=0x13
            | 0x18..=0x1b
            | 0x20..=0x23
            | 0x28..=0x2b
            | 0x30..=0x33
            | 0x38..=0x3b
            | 0x63
            | 0x69
            | 0x6b
            | 0x80..=0x8f
            | 0xc0
            | 0xc1
            | 0xc6
            | 0xc7
            | 0xd0..=0xd3
            | 0xf6
            | 0xf7
            | 0xfe
            | 0xff
    )
}

fn immediate_size(opcode: u8) -> usize {
    match opcode {
        0x6b | 0x80 | 0x82 | 0x83 | 0xc0 | 0xc1 | 0xc6 => 1,
        0x69 | 0x81 | 0xc7 => 4,
        _ => 0,
    }
}

fn mov_imm_size(bytes: &[u8]) -> usize {
    if bytes.iter().take(4).any(|byte| *byte & 0xf8 == 0x48) {
        8
    } else {
        4
    }
}

fn accumulator_imm_size(opcode: u8) -> usize {
    if matches!(
        opcode,
        0x04 | 0x0c | 0x14 | 0x1c | 0x24 | 0x2c | 0x34 | 0x3c
    ) {
        1
    } else {
        4
    }
}

fn is_prefix(byte: u8) -> bool {
    matches!(
        byte,
        0x26 | 0x2e | 0x36 | 0x3e | 0x64 | 0x65 | 0x66 | 0x67 | 0xf0 | 0xf2 | 0xf3
    ) || (0x40..=0x4f).contains(&byte)
}

fn is_relative_branch(opcode: u8) -> bool {
    matches!(opcode, 0x70..=0x7f | 0xe0..=0xe3 | 0xe9 | 0xeb)
}

fn relative_call_target(address: usize, length: usize) -> Option<usize> {
    let bytes = unsafe { std::slice::from_raw_parts(address as *const u8, length) };
    let mut cursor = 0usize;
    while cursor < bytes.len() && is_prefix(bytes[cursor]) {
        cursor += 1;
    }
    if bytes.get(cursor).copied()? != 0xe8 {
        return None;
    }
    let displacement_start = cursor + 1;
    let displacement_end = displacement_start.checked_add(4)?;
    let displacement = i32::from_le_bytes(
        bytes
            .get(displacement_start..displacement_end)?
            .try_into()
            .ok()?,
    );
    let next = (address as isize).checked_add(length as isize)?;
    Some(next.checked_add(displacement as isize)? as usize)
}
