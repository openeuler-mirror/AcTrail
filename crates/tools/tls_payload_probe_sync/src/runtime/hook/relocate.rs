use super::{ABSOLUTE_CALL_BYTES, EXTERNAL_JCC_BYTES, JUMP_PATCH_BYTES, NEAR_JCC_BYTES};

pub(super) struct RelocationPlan {
    source: usize,
    pub(super) instructions: Vec<RelocatedInstruction>,
    pub(super) stolen_len: usize,
    pub(super) trampoline_code_len: usize,
}

impl RelocationPlan {
    pub(super) fn new(source: usize) -> Result<Self, String> {
        let mut instructions = decode_stolen_instructions(source)?;
        let stolen_len = stolen_len(&instructions)?;
        assign_trampoline_offsets(source, stolen_len, &mut instructions)?;
        let trampoline_code_len = trampoline_code_len(source, stolen_len, &instructions)?;
        Ok(Self {
            source,
            instructions,
            stolen_len,
            trampoline_code_len,
        })
    }

    pub(super) fn relocated_offset_for_target(
        &self,
        target: usize,
    ) -> Result<Option<usize>, String> {
        if !is_internal_target(self.source, self.stolen_len, target) {
            return Ok(None);
        }
        let source_offset = target
            .checked_sub(self.source)
            .ok_or_else(|| "internal JCC target underflows source".to_string())?;
        let Some(instruction) = self
            .instructions
            .iter()
            .find(|instruction| instruction.source_offset == source_offset)
        else {
            return Err(format!(
                "internal JCC target is not a relocated instruction boundary: +0x{source_offset:x}"
            ));
        };
        Ok(Some(instruction.emitted_offset))
    }
}

#[derive(Clone, Copy)]
pub(super) struct RelocatedInstruction {
    pub(super) source_offset: usize,
    pub(super) length: usize,
    pub(super) emitted_offset: usize,
    pub(super) kind: RelocationKind,
}

#[derive(Clone, Copy)]
pub(super) enum RelocationKind {
    Copy,
    CallRel32 { target: usize },
    Jcc { condition: u8, target: usize },
}

struct DecodedInstruction {
    length: usize,
    kind: RelocationKind,
}

fn decode_stolen_instructions(address: usize) -> Result<Vec<RelocatedInstruction>, String> {
    let mut instructions = Vec::new();
    let mut offset = 0usize;
    while offset < JUMP_PATCH_BYTES {
        let instruction_address = address
            .checked_add(offset)
            .ok_or_else(|| "instruction address overflow".to_string())?;
        let decoded = decode_instruction(instruction_address)?;
        let length = decoded.length;
        instructions.push(RelocatedInstruction {
            source_offset: offset,
            length,
            emitted_offset: 0,
            kind: decoded.kind,
        });
        offset = offset
            .checked_add(length)
            .ok_or_else(|| "instruction length overflow".to_string())?;
    }
    Ok(instructions)
}

fn assign_trampoline_offsets(
    source: usize,
    stolen: usize,
    instructions: &mut [RelocatedInstruction],
) -> Result<(), String> {
    let mut offset = 0usize;
    for instruction in instructions {
        instruction.emitted_offset = offset;
        offset = offset
            .checked_add(emitted_len(source, stolen, instruction))
            .ok_or_else(|| "trampoline emitted offset overflow".to_string())?;
    }
    Ok(())
}

fn emitted_len(source: usize, stolen: usize, instruction: &RelocatedInstruction) -> usize {
    match instruction.kind {
        RelocationKind::Copy => instruction.length,
        RelocationKind::CallRel32 { .. } => ABSOLUTE_CALL_BYTES,
        RelocationKind::Jcc { target, .. } if is_internal_target(source, stolen, target) => {
            NEAR_JCC_BYTES
        }
        RelocationKind::Jcc { .. } => EXTERNAL_JCC_BYTES,
    }
}

fn trampoline_code_len(
    source: usize,
    stolen: usize,
    instructions: &[RelocatedInstruction],
) -> Result<usize, String> {
    let Some(last) = instructions.last() else {
        return Err("trampoline has no relocated instructions".to_string());
    };
    last.emitted_offset
        .checked_add(emitted_len(source, stolen, last))
        .ok_or_else(|| "trampoline code length overflow".to_string())
}

fn stolen_len(instructions: &[RelocatedInstruction]) -> Result<usize, String> {
    let Some(last) = instructions.last() else {
        return Err("detour requires at least one stolen instruction".to_string());
    };
    last.source_offset
        .checked_add(last.length)
        .ok_or_else(|| "stolen instruction length overflow".to_string())
}

fn is_internal_target(source: usize, stolen: usize, target: usize) -> bool {
    let Some(stolen_end) = source.checked_add(stolen) else {
        return false;
    };
    target >= source && target < stolen_end
}

fn decode_instruction(address: usize) -> Result<DecodedInstruction, String> {
    let bytes = unsafe { std::slice::from_raw_parts(address as *const u8, 16) };
    let mut cursor = 0usize;
    while cursor < bytes.len() && is_prefix(bytes[cursor]) {
        cursor += 1;
    }
    let opcode = *bytes
        .get(cursor)
        .ok_or_else(|| "missing x86_64 opcode in trampoline decoder".to_string())?;
    cursor += 1;
    if opcode == 0xe8 {
        let length = cursor + 4;
        return Ok(DecodedInstruction {
            length,
            kind: RelocationKind::CallRel32 {
                target: relative_i32_target(address, bytes, cursor, length)?,
            },
        });
    }
    if (0x70..=0x7f).contains(&opcode) {
        let displacement = *bytes
            .get(cursor)
            .ok_or_else(|| "missing x86_64 short JCC displacement".to_string())?
            as i8;
        let length = cursor + 1;
        return Ok(DecodedInstruction {
            length,
            kind: RelocationKind::Jcc {
                condition: opcode & 0x0f,
                target: relative_target(address, length, i128::from(displacement))?,
            },
        });
    }
    if is_relative_branch(opcode) {
        return Err(format!(
            "relative branch/call cannot be relocated: 0x{opcode:02x}"
        ));
    }
    if (0x50..=0x5f).contains(&opcode) || opcode == 0x90 {
        return decoded_copy(cursor);
    }
    if opcode == 0x68 {
        return decoded_copy(cursor + 4);
    }
    if opcode == 0x6a {
        return decoded_copy(cursor + 1);
    }
    if (0xb8..=0xbf).contains(&opcode) {
        return decoded_copy(cursor + mov_imm_size(bytes));
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
        return decoded_copy(cursor + accumulator_imm_size(opcode));
    }
    if opcode == 0x0f {
        if let Some(instruction) = decode_two_byte_jcc(address, bytes, cursor)? {
            return Ok(instruction);
        }
        return decoded_copy(two_byte_instruction_len(bytes, cursor)?);
    }
    if opcode_requires_modrm(opcode) {
        return decoded_copy(modrm_instruction_len(
            bytes,
            cursor,
            immediate_size(opcode),
            opcode == 0xff,
        )?);
    }
    Err(format!(
        "unsupported x86_64 opcode for trampoline: 0x{opcode:02x}"
    ))
}

fn decoded_copy(length: usize) -> Result<DecodedInstruction, String> {
    Ok(DecodedInstruction {
        length,
        kind: RelocationKind::Copy,
    })
}

fn decode_two_byte_jcc(
    address: usize,
    bytes: &[u8],
    cursor: usize,
) -> Result<Option<DecodedInstruction>, String> {
    let opcode = *bytes
        .get(cursor)
        .ok_or_else(|| "missing two-byte x86_64 opcode".to_string())?;
    if !(0x80..=0x8f).contains(&opcode) {
        return Ok(None);
    }
    let displacement_start = cursor + 1;
    let length = displacement_start + 4;
    Ok(Some(DecodedInstruction {
        length,
        kind: RelocationKind::Jcc {
            condition: opcode & 0x0f,
            target: relative_i32_target(address, bytes, displacement_start, length)?,
        },
    }))
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
    matches!(opcode, 0xe0..=0xe3 | 0xe9 | 0xeb)
}

fn relative_i32_target(
    address: usize,
    bytes: &[u8],
    displacement_start: usize,
    length: usize,
) -> Result<usize, String> {
    let displacement_end = displacement_start
        .checked_add(4)
        .ok_or_else(|| "relative i32 displacement range overflow".to_string())?;
    let displacement = i32::from_le_bytes(
        bytes
            .get(displacement_start..displacement_end)
            .ok_or_else(|| "missing relative i32 displacement".to_string())?
            .try_into()
            .map_err(|_| "relative i32 displacement has invalid length".to_string())?,
    );
    relative_target(address, length, i128::from(displacement))
}

fn relative_target(address: usize, length: usize, displacement: i128) -> Result<usize, String> {
    let base = (address as i128)
        .checked_add(length as i128)
        .ok_or_else(|| "relative branch base overflow".to_string())?;
    let target = base
        .checked_add(displacement)
        .ok_or_else(|| "relative branch target overflow".to_string())?;
    checked_target(target, "relative branch target")
}

fn checked_target(target: i128, label: &str) -> Result<usize, String> {
    if target < 0 || target > usize::MAX as i128 {
        return Err(format!("{label} is outside user address range: {target}"));
    }
    Ok(target as usize)
}
