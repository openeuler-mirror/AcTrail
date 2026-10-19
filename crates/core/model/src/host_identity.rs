use std::{fmt, str::FromStr};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HostBootId([u8; 16]);

impl HostBootId {
    pub const BYTE_COUNT: usize = 16;
    pub const fn from_bytes(bytes: [u8; Self::BYTE_COUNT]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; Self::BYTE_COUNT] {
        &self.0
    }
}

impl FromStr for HostBootId {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let raw = raw.trim();
        if raw.len() != 36
            || raw.bytes().enumerate().any(|(index, byte)| {
                if matches!(index, 8 | 13 | 18 | 23) {
                    byte != b'-'
                } else {
                    !byte.is_ascii_hexdigit()
                }
            })
        {
            return Err("host boot ID must be a UUID".into());
        }
        let mut bytes = [0; 16];
        let mut digits = raw.chars().filter(|ch| *ch != '-');
        for byte in &mut bytes {
            let high = digits
                .next()
                .and_then(|ch| ch.to_digit(16))
                .ok_or("invalid UUID digit")?;
            let low = digits
                .next()
                .and_then(|ch| ch.to_digit(16))
                .ok_or("invalid UUID digit")?;
            *byte = ((high << 4) | low) as u8;
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for HostBootId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, byte) in self.0.iter().enumerate() {
            if matches!(index, 4 | 6 | 8 | 10) {
                formatter.write_str("-")?;
            }
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}
