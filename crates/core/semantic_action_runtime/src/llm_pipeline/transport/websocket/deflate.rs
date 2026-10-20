//! Negotiated per-message deflate with a bounded decoded result.
use flate2::{Decompress, FlushDecompress, Status};
pub(super) struct DeflateDecoder {
    enabled: bool,
    no_context_takeover: bool,
    decoder: Decompress,
    max_bytes: usize,
}
impl DeflateDecoder {
    pub(super) fn new(enabled: bool, no_context_takeover: bool, max_bytes: usize) -> Self {
        Self {
            enabled,
            no_context_takeover,
            decoder: Decompress::new(false),
            max_bytes,
        }
    }
    pub(super) fn decode(&mut self, mut input: Vec<u8>) -> Result<Vec<u8>, ()> {
        if !self.enabled {
            return Err(());
        }
        input.extend_from_slice(&[0, 0, 255, 255]);
        let mut cursor = 0;
        let mut output = Vec::new();
        loop {
            let mut chunk = [0u8; 4096];
            let before_in = self.decoder.total_in();
            let before_out = self.decoder.total_out();
            let status = self
                .decoder
                .decompress(&input[cursor..], &mut chunk, FlushDecompress::Sync)
                .map_err(|_| ())?;
            let consumed = usize::try_from(self.decoder.total_in() - before_in).map_err(|_| ())?;
            let produced =
                usize::try_from(self.decoder.total_out() - before_out).map_err(|_| ())?;
            cursor += consumed;
            if output.len().saturating_add(produced) > self.max_bytes {
                return Err(());
            }
            output.extend_from_slice(&chunk[..produced]);
            if cursor == input.len() && produced < chunk.len() {
                break;
            }
            if consumed == 0 && produced == 0 {
                if cursor == input.len() && status == Status::BufError {
                    break;
                }
                return Err(());
            }
        }
        if self.no_context_takeover {
            self.decoder.reset(false);
        }
        Ok(output)
    }
}
