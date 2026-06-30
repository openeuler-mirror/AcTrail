use super::{
    ABSOLUTE_CALL_BYTES, ABSOLUTE_JUMP_BYTES, EXTERNAL_BRANCH_ISLAND_BYTES, INSTRUCTION_BYTES,
    JUMP_PATCH_BYTES, LOAD_IMMEDIATE_BYTES,
};

pub(super) struct RelocationPlan {
    source: usize,
    pub(super) instructions: Vec<RelocatedInstruction>,
    pub(super) stolen_len: usize,
    pub(super) trampoline_code_len: usize,
}

impl RelocationPlan {
    pub(super) fn new(source: usize) -> Result<Self, String> {
        if source % INSTRUCTION_BYTES != 0 {
            return Err(format!(
                "aarch64 hook target is not 4-byte aligned: 0x{source:x}"
            ));
        }
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
            .ok_or_else(|| "internal aarch64 branch target underflows source".to_string())?;
        let Some(instruction) = self
            .instructions
            .iter()
            .find(|instruction| instruction.source_offset == source_offset)
        else {
            return Err(format!(
                "internal aarch64 branch target is not a relocated instruction boundary: +0x{source_offset:x}"
            ));
        };
        Ok(Some(instruction.emitted_offset))
    }
}

#[derive(Clone, Copy)]
pub(super) struct RelocatedInstruction {
    pub(super) source_offset: usize,
    pub(super) emitted_offset: usize,
    pub(super) raw: u32,
    pub(super) kind: RelocationKind,
}

#[derive(Clone, Copy)]
pub(super) enum RelocationKind {
    Copy,
    Adr { register: u8, target: usize },
    Bl { target: usize },
    Branch { target: usize },
    ConditionalBranch { target: usize },
    CompareBranch { target: usize },
    TestBranch { target: usize },
}

fn decode_stolen_instructions(address: usize) -> Result<Vec<RelocatedInstruction>, String> {
    let mut instructions = Vec::new();
    let mut offset = 0usize;
    while offset < JUMP_PATCH_BYTES {
        let instruction_address = address
            .checked_add(offset)
            .ok_or_else(|| "aarch64 instruction address overflow".to_string())?;
        let raw = unsafe { std::ptr::read_unaligned(instruction_address as *const u32) }.to_le();
        instructions.push(RelocatedInstruction {
            source_offset: offset,
            emitted_offset: 0,
            raw,
            kind: decode_instruction(instruction_address, raw).map_err(|error| {
                format!(
                    "aarch64 instruction cannot be relocated in trampoline at +{offset}: {error} 0x{raw:08x}"
                )
            })?,
        });
        offset = offset
            .checked_add(INSTRUCTION_BYTES)
            .ok_or_else(|| "aarch64 stolen instruction length overflow".to_string())?;
    }
    Ok(instructions)
}

fn decode_instruction(address: usize, instruction: u32) -> Result<RelocationKind, String> {
    if let Some(target) = adr_target(address, instruction) {
        return Ok(RelocationKind::Adr {
            register: register(instruction),
            target,
        });
    }
    if let Some(target) = bl_target(address, instruction) {
        return Ok(RelocationKind::Bl { target });
    }
    if let Some(target) = branch_target(address, instruction) {
        return Ok(RelocationKind::Branch { target });
    }
    if let Some(target) = conditional_branch_target(address, instruction) {
        return Ok(RelocationKind::ConditionalBranch { target });
    }
    if let Some(target) = compare_branch_target(address, instruction) {
        return Ok(RelocationKind::CompareBranch { target });
    }
    if let Some(target) = test_branch_target(address, instruction) {
        return Ok(RelocationKind::TestBranch { target });
    }
    if let Some(reason) = relocation_error(instruction) {
        return Err(reason.to_string());
    }
    Ok(RelocationKind::Copy)
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
            .ok_or_else(|| "aarch64 trampoline emitted offset overflow".to_string())?;
    }
    Ok(())
}

fn emitted_len(source: usize, stolen: usize, instruction: &RelocatedInstruction) -> usize {
    match instruction.kind {
        RelocationKind::Copy => INSTRUCTION_BYTES,
        RelocationKind::Adr { .. } => LOAD_IMMEDIATE_BYTES,
        RelocationKind::Bl { target } if is_internal_target(source, stolen, target) => {
            INSTRUCTION_BYTES
        }
        RelocationKind::Bl { .. } => ABSOLUTE_CALL_BYTES,
        RelocationKind::Branch { target } if is_internal_target(source, stolen, target) => {
            INSTRUCTION_BYTES
        }
        RelocationKind::Branch { .. } => ABSOLUTE_JUMP_BYTES,
        RelocationKind::ConditionalBranch { target }
        | RelocationKind::CompareBranch { target }
        | RelocationKind::TestBranch { target }
            if is_internal_target(source, stolen, target) =>
        {
            INSTRUCTION_BYTES
        }
        RelocationKind::ConditionalBranch { .. }
        | RelocationKind::CompareBranch { .. }
        | RelocationKind::TestBranch { .. } => EXTERNAL_BRANCH_ISLAND_BYTES,
    }
}

fn trampoline_code_len(
    source: usize,
    stolen: usize,
    instructions: &[RelocatedInstruction],
) -> Result<usize, String> {
    let Some(last) = instructions.last() else {
        return Err("aarch64 trampoline has no relocated instructions".to_string());
    };
    last.emitted_offset
        .checked_add(emitted_len(source, stolen, last))
        .ok_or_else(|| "aarch64 trampoline code length overflow".to_string())
}

fn stolen_len(instructions: &[RelocatedInstruction]) -> Result<usize, String> {
    let Some(last) = instructions.last() else {
        return Err("aarch64 detour requires at least one stolen instruction".to_string());
    };
    last.source_offset
        .checked_add(INSTRUCTION_BYTES)
        .ok_or_else(|| "aarch64 stolen instruction length overflow".to_string())
}

fn is_internal_target(source: usize, stolen: usize, target: usize) -> bool {
    let Some(stolen_end) = source.checked_add(stolen) else {
        return false;
    };
    target >= source && target < stolen_end
}

pub(super) fn relocation_error(instruction: u32) -> Option<&'static str> {
    if (instruction & 0xfe00_0000) == 0xd600_0000 {
        return Some("register branch");
    }
    if (instruction & 0x3b00_0000) == 0x1800_0000 {
        return Some("literal pc-relative load");
    }
    None
}

pub(super) fn adr_target(address: usize, instruction: u32) -> Option<usize> {
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

pub(super) fn bl_target(address: usize, instruction: u32) -> Option<usize> {
    if (instruction & 0xfc00_0000) != 0x9400_0000 {
        return None;
    }
    let immediate = sign_extend((instruction & 0x03ff_ffff) as u64, 26) << 2;
    checked_target(address as i128 + immediate)
}

fn branch_target(address: usize, instruction: u32) -> Option<usize> {
    if (instruction & 0xfc00_0000) != 0x1400_0000 {
        return None;
    }
    let immediate = sign_extend((instruction & 0x03ff_ffff) as u64, 26) << 2;
    checked_target(address as i128 + immediate)
}

fn conditional_branch_target(address: usize, instruction: u32) -> Option<usize> {
    if (instruction & 0xff00_0010) != 0x5400_0000 {
        return None;
    }
    let immediate = sign_extend(((instruction >> 5) & 0x7ffff) as u64, 19) << 2;
    checked_target(address as i128 + immediate)
}

fn compare_branch_target(address: usize, instruction: u32) -> Option<usize> {
    if (instruction & 0x7e00_0000) != 0x3400_0000 {
        return None;
    }
    let immediate = sign_extend(((instruction >> 5) & 0x7ffff) as u64, 19) << 2;
    checked_target(address as i128 + immediate)
}

fn test_branch_target(address: usize, instruction: u32) -> Option<usize> {
    if (instruction & 0x7e00_0000) != 0x3600_0000 {
        return None;
    }
    let immediate = sign_extend(((instruction >> 5) & 0x3fff) as u64, 14) << 2;
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
