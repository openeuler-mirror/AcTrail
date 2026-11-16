use crate::elf::ElfImage;

use super::Rejection;
use super::instruction::Instruction;

// Candidate seeds for verified client compiler shapes. Structural checks
// independently recover and validate state fields and call targets.
// Omit LDR's destination register byte; verify its x1 base at each hit.
const HANDSHAKE: &[u8] = &[0x00, 0x40, 0xf9, 0x28, 0x00, 0x80, 0x52];
// Grok's direct nullable buffer ABI writes the verified send field before CBZ x1.
const DIRECT_HANDSHAKE: &[u8] = &[0x28, 0x00, 0x80, 0x52, 0x08, 0xd8, 0x0c, 0x39];
const ADD_MOV_X23: &[u8] = &[0x91, 0xe0, 0x03, 0x17, 0xaa];
const ADD_MOV_X24: &[u8] = &[0x91, 0xe0, 0x03, 0x18, 0xaa];
// MOV x0,x23 followed by the first byte of ADD x1,x19,#imm. This
// also admits other Rn values with the same low bits; dataflow checks them.
const MOV_ADD: &[u8] = &[0xe0, 0x03, 0x17, 0xaa, 0x61];
const STORE_X25: &[u8] = &[0x28, 0x00, 0x80, 0x52, 0x28, 0xdf, 0x0c, 0x39];
const STORE_X28: &[u8] = &[0x28, 0x00, 0x80, 0x52, 0x88, 0xdf, 0x0c, 0x39];
const PATTERNS: &[&[u8]] = &[
    HANDSHAKE,
    DIRECT_HANDSHAKE,
    ADD_MOV_X23,
    ADD_MOV_X24,
    MOV_ADD,
    STORE_X25,
    STORE_X28,
];

#[derive(Clone, Copy)]
enum CallerShape {
    AddMove,
    MoveAdd,
    Store,
}

const CALLER_SHAPES: [CallerShape; 5] = [
    CallerShape::AddMove,
    CallerShape::AddMove,
    CallerShape::MoveAdd,
    CallerShape::Store,
    CallerShape::Store,
];

pub(super) struct CandidatePatterns<'a> {
    image: &'a ElfImage,
    offsets: Vec<Vec<usize>>,
}

impl<'a> CandidatePatterns<'a> {
    pub(super) fn register(image: &ElfImage) {
        for pattern in PATTERNS {
            image.register_pattern_scan(pattern);
        }
    }

    pub(super) fn new(image: &'a ElfImage) -> Result<Self, Rejection> {
        let ranges = image
            .executable_file_ranges()
            .map_err(|_| Rejection::InvalidImage)?;
        Ok(Self {
            image,
            offsets: image.pattern_offsets_for(PATTERNS, &ranges),
        })
    }

    pub(super) fn handshake_entries(&self) -> impl Iterator<Item = Result<u64, Rejection>> + '_ {
        let wrapped = self.offsets[0].iter().filter_map(|&offset| {
            let start = offset.checked_sub(1)?;
            let valid = self
                .image
                .data()
                .get(start..start.checked_add(16)?)
                .is_some_and(|bytes| {
                    let load = u32::from_le_bytes(bytes[..4].try_into().unwrap());
                    let store = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
                    let branch = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
                    load & 0xffffffe0 == 0xf9400020
                        && store & 0xffc0001f == 0x39000008
                        && branch & 0xff00001f == (0xb4000000 | (load & 31))
                });
            self.image.release_scan_range(start, 16);
            if !valid {
                return None;
            }
            match self.image.virtual_address_for_file_offset(start as u64) {
                Ok(address) => (address % 4 == 0).then_some(Ok(address)),
                Err(_) => Some(Err(Rejection::InvalidImage)),
            }
        });
        let direct = self.offsets[1].iter().filter_map(|&offset| {
            let address = match self.image.virtual_address_for_file_offset(offset as u64) {
                Ok(address) => address,
                Err(_) => return Some(Err(Rejection::InvalidImage)),
            };
            let valid = address % 4 == 0
                && self
                    .image
                    .data()
                    .get(offset.checked_add(8)?..offset.checked_add(12)?)
                    .is_some_and(|bytes| {
                        let word = u32::from_le_bytes(bytes.try_into().unwrap());
                        word & 0xff00001f == 0xb4000001
                    });
            self.image.release_scan_range(offset, 12);
            valid.then_some(Ok(address))
        });
        wrapped.chain(direct)
    }

    pub(super) fn visit_calls(
        &self,
        mut visit: impl FnMut(u64, u64) -> Result<(), Rejection>,
    ) -> Result<(), Rejection> {
        // Visit all caller shapes while each scan window is resident.
        // The cached offsets are already ordered; advance one cursor per caller shape.
        let mut remaining: [&[usize]; CALLER_SHAPES.len()] =
            std::array::from_fn(|index| self.offsets[index + 2].as_slice());
        let chunk = self.image.scan_chunk_bytes().max(1);
        while let Some(&first) = remaining.iter().filter_map(|offsets| offsets.first()).min() {
            let begin = first / chunk * chunk;
            let end = begin.saturating_add(chunk).min(self.image.data().len());
            let result = remaining
                .iter_mut()
                .enumerate()
                .try_for_each(|(index, offsets)| {
                    let count = offsets.iter().take_while(|&&offset| offset < end).count();
                    self.visit_sites(&offsets[..count], CALLER_SHAPES[index], &mut visit)?;
                    *offsets = &offsets[count..];
                    Ok(())
                });
            let release_start = begin.saturating_sub(3);
            let release_end = end.saturating_add(12).min(self.image.data().len());
            self.image
                .release_scan_range(release_start, release_end - release_start);
            result?;
        }
        Ok(())
    }

    fn visit_sites(
        &self,
        offsets: &[usize],
        shape: CallerShape,
        visit: &mut impl FnMut(u64, u64) -> Result<(), Rejection>,
    ) -> Result<(), Rejection> {
        for &offset in offsets {
            let Some(start) = offset.checked_sub(if matches!(shape, CallerShape::AddMove) {
                3
            } else {
                0
            }) else {
                continue;
            };
            let Some(bytes) = self.image.data().get(start..start + 12) else {
                continue;
            };
            let call = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
            if call & 0xfc000000 != 0x94000000 {
                continue;
            }
            let pc = self
                .image
                .virtual_address_for_file_offset((start + 8) as u64)
                .map_err(|_| Rejection::InvalidImage)?;
            if pc % 4 != 0 {
                continue;
            }
            let valid = match shape {
                CallerShape::AddMove | CallerShape::MoveAdd => {
                    let add_offset = if matches!(shape, CallerShape::AddMove) {
                        0
                    } else {
                        4
                    };
                    let add =
                        u32::from_le_bytes(bytes[add_offset..add_offset + 4].try_into().unwrap());
                    add & 0xff80001f == 0x91000001
                }
                CallerShape::Store => matches!(
                    Instruction::decode(
                        u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
                        pc - 4
                    ),
                    Instruction::StoreByte {
                        value: 8,
                        base: 25 | 28,
                        ..
                    }
                ),
            };
            if !valid {
                continue;
            }
            if let Instruction::Call(target) = Instruction::decode(call, pc) {
                visit(pc, target)?;
            }
        }
        Ok(())
    }
}
