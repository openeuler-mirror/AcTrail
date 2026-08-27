use crate::DeliveryCodecError;

pub(super) struct PayloadWriter {
    bytes: Vec<u8>,
}

impl PayloadWriter {
    pub(super) fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    pub(super) fn write_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub(super) fn write_u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    pub(super) fn write_u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    pub(super) fn write_fixed(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }

    pub(super) fn write_required_u16_string(
        &mut self,
        field: &'static str,
        value: &str,
        configured_max: usize,
    ) -> Result<(), DeliveryCodecError> {
        if value.is_empty() {
            return Err(DeliveryCodecError::new(
                "atap_encode",
                format!("{field} must not be empty"),
            ));
        }
        self.write_u16_string(field, value, configured_max)
    }

    pub(super) fn write_u16_string(
        &mut self,
        field: &'static str,
        value: &str,
        configured_max: usize,
    ) -> Result<(), DeliveryCodecError> {
        let length = checked_length(field, value.len(), configured_max, u16::MAX as usize)?;
        self.bytes.extend_from_slice(&(length as u16).to_be_bytes());
        self.bytes.extend_from_slice(value.as_bytes());
        Ok(())
    }

    pub(super) fn write_u32_bytes(
        &mut self,
        field: &'static str,
        value: &[u8],
    ) -> Result<(), DeliveryCodecError> {
        let length = u32::try_from(value.len()).map_err(|_| {
            DeliveryCodecError::new("atap_encode", format!("{field} length does not fit u32"))
        })?;
        self.bytes.extend_from_slice(&length.to_be_bytes());
        self.bytes.extend_from_slice(value);
        Ok(())
    }

    pub(super) fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

pub(super) struct PayloadCursor<'a> {
    payload: &'a [u8],
    offset: usize,
}

impl<'a> PayloadCursor<'a> {
    pub(super) fn new(payload: &'a [u8]) -> Self {
        Self { payload, offset: 0 }
    }

    pub(super) fn read_u8(&mut self, field: &'static str) -> Result<u8, DeliveryCodecError> {
        Ok(self.take(field, 1)?[0])
    }

    pub(super) fn read_u32(&mut self, field: &'static str) -> Result<u32, DeliveryCodecError> {
        Ok(u32::from_be_bytes(
            self.take(field, 4)?.try_into().expect("checked u32 field"),
        ))
    }

    pub(super) fn read_u64(&mut self, field: &'static str) -> Result<u64, DeliveryCodecError> {
        Ok(u64::from_be_bytes(
            self.take(field, 8)?.try_into().expect("checked u64 field"),
        ))
    }

    pub(super) fn read_fixed<const N: usize>(
        &mut self,
        field: &'static str,
    ) -> Result<[u8; N], DeliveryCodecError> {
        self.take(field, N)?
            .try_into()
            .map_err(|_| DeliveryCodecError::new("atap_decode", format!("invalid {field} size")))
    }

    pub(super) fn read_required_u16_string(
        &mut self,
        field: &'static str,
        configured_max: usize,
    ) -> Result<String, DeliveryCodecError> {
        let value = self.read_u16_string(field, configured_max)?;
        if value.is_empty() {
            return Err(DeliveryCodecError::new(
                "atap_decode",
                format!("{field} must not be empty"),
            ));
        }
        Ok(value)
    }

    pub(super) fn read_u16_string(
        &mut self,
        field: &'static str,
        configured_max: usize,
    ) -> Result<String, DeliveryCodecError> {
        let length = u16::from_be_bytes(self.take(field, 2)?.try_into().expect("checked u16 field"))
            as usize;
        if length > configured_max {
            return Err(DeliveryCodecError::new(
                "atap_decode",
                format!("{field} length {length} exceeds configured limit {configured_max}"),
            ));
        }
        let bytes = self.take(field, length)?;
        String::from_utf8(bytes.to_vec()).map_err(|error| {
            DeliveryCodecError::new("atap_decode", format!("{field} is not UTF-8: {error}"))
        })
    }

    pub(super) fn read_u32_bytes(
        &mut self,
        field: &'static str,
        configured_max: usize,
    ) -> Result<&'a [u8], DeliveryCodecError> {
        let length = self.read_u32(field)? as usize;
        if length > configured_max {
            return Err(DeliveryCodecError::new(
                "atap_decode",
                format!("{field} length {length} exceeds configured limit {configured_max}"),
            ));
        }
        self.take(field, length)
    }

    pub(super) fn finish(self) -> Result<(), DeliveryCodecError> {
        if self.offset == self.payload.len() {
            Ok(())
        } else {
            Err(DeliveryCodecError::new(
                "atap_decode",
                format!(
                    "payload contains {} trailing bytes",
                    self.payload.len() - self.offset
                ),
            ))
        }
    }

    fn take(&mut self, field: &'static str, length: usize) -> Result<&'a [u8], DeliveryCodecError> {
        let end = self.offset.checked_add(length).ok_or_else(|| {
            DeliveryCodecError::new("atap_decode", format!("{field} offset overflow"))
        })?;
        if end > self.payload.len() {
            return Err(DeliveryCodecError::new(
                "atap_decode",
                format!("{field} exceeds payload boundary"),
            ));
        }
        let field_bytes = &self.payload[self.offset..end];
        self.offset = end;
        Ok(field_bytes)
    }
}

fn checked_length(
    field: &'static str,
    length: usize,
    configured_max: usize,
    protocol_max: usize,
) -> Result<usize, DeliveryCodecError> {
    if length > configured_max {
        return Err(DeliveryCodecError::new(
            "atap_encode",
            format!("{field} length {length} exceeds configured limit {configured_max}"),
        ));
    }
    if length > protocol_max {
        return Err(DeliveryCodecError::new(
            "atap_encode",
            format!("{field} length {length} exceeds protocol limit {protocol_max}"),
        ));
    }
    Ok(length)
}
