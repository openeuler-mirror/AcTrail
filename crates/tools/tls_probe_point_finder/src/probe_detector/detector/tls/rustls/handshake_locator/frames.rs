use gimli::{BaseAddresses, EhFrame, EhFrameHdr, LittleEndian, UnwindSection};

use super::Rejection;
use crate::elf::ElfImage;

pub(super) struct FunctionLookup<'a> {
    image: &'a ElfImage,
    header: &'a [u8],
    frame: &'a [u8],
    bases: BaseAddresses,
}

impl<'a> FunctionLookup<'a> {
    pub(super) fn new(image: &'a ElfImage) -> Result<Self, Rejection> {
        let header = image
            .section_data(".eh_frame_hdr")
            .ok()
            .flatten()
            .ok_or(Rejection::MissingBoundaries)?;
        let frame = image
            .section_data(".eh_frame")
            .ok()
            .flatten()
            .ok_or(Rejection::MissingBoundaries)?;
        let mut bases = BaseAddresses::default()
            .set_eh_frame_hdr(
                image
                    .section_virtual_address(".eh_frame_hdr")
                    .ok_or(Rejection::MissingBoundaries)?,
            )
            .set_eh_frame(
                image
                    .section_virtual_address(".eh_frame")
                    .ok_or(Rejection::MissingBoundaries)?,
            );
        if let Some(address) = image.section_virtual_address(".text") {
            bases = bases.set_text(address);
        }
        if let Some(address) = image.section_virtual_address(".got") {
            bases = bases.set_got(address);
        }
        Ok(Self {
            image,
            header,
            frame,
            bases,
        })
    }

    pub(super) fn range(&self, address: u64) -> Result<(u64, u64), Rejection> {
        let header = EhFrameHdr::new(self.header, LittleEndian)
            .parse(&self.bases, 8)
            .map_err(|_| Rejection::MissingBoundaries)?;
        let table = header.table().ok_or(Rejection::MissingBoundaries)?;
        let frame = EhFrame::new(self.frame, LittleEndian);
        let fde = table
            .fde_for_address(&frame, &self.bases, address, EhFrame::cie_from_offset)
            .map_err(|_| Rejection::MissingBoundaries)?;
        let start = fde.initial_address();
        let end = start
            .checked_add(fde.len())
            .ok_or(Rejection::MissingBoundaries)?;
        if start % 4 != 0 || end <= start || end % 4 != 0 {
            return Err(Rejection::MissingBoundaries);
        }
        Ok((start, end))
    }

    pub(super) fn words(&self, start: u64, end: u64, max: usize) -> Result<Vec<u32>, Rejection> {
        let count = end.checked_sub(start).ok_or(Rejection::MissingBoundaries)? / 4;
        if count == 0 || count > max as u64 {
            return Err(Rejection::FunctionBudget);
        }
        let offset = self
            .image
            .file_offset_for_virtual_address(start)
            .map_err(|_| Rejection::MissingBoundaries)?;
        let last = self
            .image
            .file_offset_for_virtual_address(end - 1)
            .map_err(|_| Rejection::MissingBoundaries)?;
        if last.checked_sub(offset) != Some(end - start - 1) {
            return Err(Rejection::MissingBoundaries);
        }
        let begin = usize::try_from(offset).map_err(|_| Rejection::MissingBoundaries)?;
        let finish = usize::try_from(last + 1).map_err(|_| Rejection::MissingBoundaries)?;
        let bytes = self
            .image
            .data()
            .get(begin..finish)
            .ok_or(Rejection::MissingBoundaries)?;
        let words = bytes
            .chunks_exact(4)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
            .collect();
        self.image.release_scan_window(begin);
        Ok(words)
    }
}
