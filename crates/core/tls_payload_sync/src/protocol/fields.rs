use crate::{SyncError, SyncResult};
use std::io::Write;

pub(super) struct FieldWriter<W> {
    writer: W,
    bytes: usize,
}
impl<W: Write> FieldWriter<W> {
    pub(super) fn new(writer: W) -> Self {
        Self { writer, bytes: 0 }
    }
    pub(super) fn len(&self) -> usize {
        self.bytes
    }
    fn raw(&mut self, bytes: &[u8]) -> SyncResult<()> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .ok_or_else(|| SyncError::new("TLS frame length overflow"))?;
        self.writer.write_all(bytes)?;
        Ok(())
    }
    pub(super) fn u8(&mut self, value: u8) -> SyncResult<()> {
        self.raw(&[value])
    }
    pub(super) fn u16(&mut self, value: u16) -> SyncResult<()> {
        self.raw(&value.to_le_bytes())
    }
    pub(super) fn u32(&mut self, value: u32) -> SyncResult<()> {
        self.raw(&value.to_le_bytes())
    }
    pub(super) fn u64(&mut self, value: u64) -> SyncResult<()> {
        self.raw(&value.to_le_bytes())
    }
    pub(super) fn bytes(&mut self, value: &[u8]) -> SyncResult<()> {
        self.u32(
            u32::try_from(value.len()).map_err(|_| SyncError::new("TLS field length overflow"))?,
        )?;
        self.raw(value)
    }
    pub(super) fn text(&mut self, value: &str) -> SyncResult<()> {
        self.bytes(value.as_bytes())
    }
}

pub(super) struct FieldReader<'a> {
    remaining: &'a [u8],
}
impl<'a> FieldReader<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self { remaining: bytes }
    }
    fn take(&mut self, len: usize) -> SyncResult<&'a [u8]> {
        if len > self.remaining.len() {
            return Err(SyncError::new("truncated TLS frame field"));
        }
        let (value, rest) = self.remaining.split_at(len);
        self.remaining = rest;
        Ok(value)
    }
    pub(super) fn u8(&mut self) -> SyncResult<u8> {
        Ok(self.take(1)?[0])
    }
    pub(super) fn u16(&mut self) -> SyncResult<u16> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().expect("fixed field"),
        ))
    }
    pub(super) fn u32(&mut self) -> SyncResult<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("fixed field"),
        ))
    }
    pub(super) fn u64(&mut self) -> SyncResult<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("fixed field"),
        ))
    }
    pub(super) fn bytes(&mut self) -> SyncResult<&'a [u8]> {
        let len = self.u32()? as usize;
        self.take(len)
    }
    pub(super) fn text(&mut self) -> SyncResult<String> {
        std::str::from_utf8(self.bytes()?)
            .map(str::to_owned)
            .map_err(|_| SyncError::new("invalid UTF-8 TLS text field"))
    }
    pub(super) fn finish(self) -> SyncResult<()> {
        if self.remaining.is_empty() {
            Ok(())
        } else {
            Err(SyncError::new("trailing TLS frame bytes"))
        }
    }
}
