//! Incremental, constant-space chunk framing for HTTP summary projection.

use super::{SummaryChunkedProgress, SummaryChunkedState};

impl SummaryChunkedProgress {
    pub(super) fn consume(
        &mut self,
        bytes: &[u8],
        max_line_bytes: u64,
    ) -> Result<(usize, bool), String> {
        let mut cursor = 0usize;
        while cursor < bytes.len() {
            match &mut self.state {
                SummaryChunkedState::Size {
                    value,
                    saw_digit,
                    in_extension,
                    saw_cr,
                    line_bytes,
                } => {
                    let byte = bytes[cursor];
                    cursor += 1;
                    *line_bytes = line_bytes.saturating_add(1);
                    if *line_bytes > max_line_bytes {
                        return Err("HTTP chunk size line exceeds configured maximum".to_string());
                    }
                    if *saw_cr {
                        if byte != b'\n' || !*saw_digit {
                            return Err("invalid HTTP chunk size terminator".to_string());
                        }
                        self.state = if *value == 0 {
                            SummaryChunkedState::Trailers {
                                line_nonempty: false,
                                saw_cr: false,
                                line_bytes: 0,
                            }
                        } else {
                            SummaryChunkedState::Data(*value)
                        };
                        continue;
                    }
                    if byte == b'\r' {
                        *saw_cr = true;
                    } else if *in_extension {
                        if byte == b'\n' {
                            return Err("invalid HTTP chunk extension".to_string());
                        }
                    } else if byte == b';' && *saw_digit {
                        *in_extension = true;
                    } else if let Some(digit) = (byte as char).to_digit(16) {
                        *value = value
                            .checked_mul(16)
                            .and_then(|value| value.checked_add(u64::from(digit)))
                            .ok_or_else(|| "HTTP chunk size overflow".to_string())?;
                        *saw_digit = true;
                    } else {
                        return Err("invalid HTTP chunk size".to_string());
                    }
                }
                SummaryChunkedState::Data(remaining) => {
                    let consumed = usize::try_from(*remaining)
                        .unwrap_or(usize::MAX)
                        .min(bytes.len() - cursor);
                    cursor += consumed;
                    *remaining = remaining.saturating_sub(consumed as u64);
                    if *remaining == 0 {
                        self.state = SummaryChunkedState::DataTerminator(0);
                    }
                }
                SummaryChunkedState::DataTerminator(matched) => {
                    let expected = [b'\r', b'\n'];
                    if bytes[cursor] != expected[usize::from(*matched)] {
                        return Err("HTTP chunk missing CRLF terminator".to_string());
                    }
                    cursor += 1;
                    *matched += 1;
                    if *matched == 2 {
                        self.state = Self::default().state;
                    }
                }
                SummaryChunkedState::Trailers {
                    line_nonempty,
                    saw_cr,
                    line_bytes,
                } => {
                    let byte = bytes[cursor];
                    cursor += 1;
                    *line_bytes = line_bytes.saturating_add(1);
                    if *line_bytes > max_line_bytes {
                        return Err(
                            "HTTP chunk trailer line exceeds configured maximum".to_string()
                        );
                    }
                    if *saw_cr {
                        if byte != b'\n' {
                            return Err("invalid HTTP chunk trailer terminator".to_string());
                        }
                        if !*line_nonempty {
                            return Ok((cursor, true));
                        }
                        *line_nonempty = false;
                        *saw_cr = false;
                        *line_bytes = 0;
                    } else if byte == b'\r' {
                        *saw_cr = true;
                    } else if byte == b'\n' {
                        return Err("invalid HTTP chunk trailer".to_string());
                    } else {
                        *line_nonempty = true;
                    }
                }
            }
        }
        Ok((cursor, false))
    }
}
