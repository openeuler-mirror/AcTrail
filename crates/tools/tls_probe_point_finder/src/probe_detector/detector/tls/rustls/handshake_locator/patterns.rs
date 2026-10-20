use crate::elf::ElfImage;

use super::Rejection;
use super::instruction::Instruction;

// Candidate seeds for the verified client compiler shapes. The field offsets
// and branch displacements are deliberately outside the exact patterns.
// Omit LDR's destination register byte; verify its x1 base at each hit.
const HANDSHAKE: &[u8] = &[0x00, 0x40, 0xf9, 0x28, 0x00, 0x80, 0x52];
const ADD_MOV_X23: &[u8] = &[0x91, 0xe0, 0x03, 0x17, 0xaa];
const ADD_MOV_X24: &[u8] = &[0x91, 0xe0, 0x03, 0x18, 0xaa];
// MOV x0,x23 followed by the first byte of ADD x1,x19,#imm. This
// also admits other Rn values with the same low bits; dataflow checks them.
const MOV_ADD: &[u8] = &[0xe0, 0x03, 0x17, 0xaa, 0x61];
const PATTERNS: &[&[u8]] = &[HANDSHAKE, ADD_MOV_X23, ADD_MOV_X24, MOV_ADD];

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
        self.offsets[0].iter().filter_map(|&offset| {
            let start = offset.checked_sub(1)?;
            let valid_base = self.image.data()[start] & 0xe0 == 0x20;
            self.image.release_scan_window(start);
            if !valid_base {
                return None;
            }
            match self.image.virtual_address_for_file_offset(start as u64) {
                Ok(address) => (address % 4 == 0).then_some(Ok(address)),
                Err(_) => Some(Err(Rejection::InvalidImage)),
            }
        })
    }

    pub(super) fn visit_calls(
        &self,
        mut visit: impl FnMut(u64, u64) -> Result<(), Rejection>,
    ) -> Result<(), Rejection> {
        // Visit all caller shapes while each scan window is resident.
        // The cached offsets are already ordered; only three slice cursors are needed.
        let mut remaining: [&[usize]; 3] =
            std::array::from_fn(|index| self.offsets[index + 1].as_slice());
        let chunk = self.image.scan_chunk_bytes().max(1);
        while let Some(&first) = remaining.iter().filter_map(|offsets| offsets.first()).min() {
            let begin = first / chunk * chunk;
            let end = begin.saturating_add(chunk).min(self.image.data().len());
            let result = remaining
                .iter_mut()
                .enumerate()
                .try_for_each(|(index, offsets)| {
                    let count = offsets.iter().take_while(|&&offset| offset < end).count();
                    self.visit_sites(&offsets[..count], index != 2, &mut visit)?;
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
        add_first: bool,
        visit: &mut impl FnMut(u64, u64) -> Result<(), Rejection>,
    ) -> Result<(), Rejection> {
        for &offset in offsets {
            let Some(start) = offset.checked_sub(if add_first { 3 } else { 0 }) else {
                continue;
            };
            let Some(bytes) = self.image.data().get(start..start + 12) else {
                continue;
            };
            let add_offset = if add_first { 0 } else { 4 };
            let add = u32::from_le_bytes(bytes[add_offset..add_offset + 4].try_into().unwrap());
            let call = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
            if add & 0xff80001f != 0x91000001 || call & 0xfc000000 != 0x94000000 {
                continue;
            }
            let pc = self
                .image
                .virtual_address_for_file_offset((start + 8) as u64)
                .map_err(|_| Rejection::InvalidImage)?;
            if pc % 4 != 0 {
                continue;
            }
            if let Instruction::Call(target) = Instruction::decode(call, pc) {
                visit(pc, target)?;
            }
        }
        Ok(())
    }
}
