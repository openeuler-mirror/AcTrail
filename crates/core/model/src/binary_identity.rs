use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u16)]
pub enum BinaryIdentityTypeCode {
    GnuBuildId = 1,
    ElfExecutableSampleSha256V1 = 2,
}

impl BinaryIdentityTypeCode {
    pub fn code(self) -> u16 {
        self as u16
    }

    pub fn parse(code: u16) -> Result<Self, BinaryIdentityError> {
        match code {
            1 => Ok(Self::GnuBuildId),
            2 => Ok(Self::ElfExecutableSampleSha256V1),
            _ => Err(BinaryIdentityError::new(format!(
                "unknown binary identity type code {code}"
            ))),
        }
    }
}

impl Display for BinaryIdentityTypeCode {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.code())
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BinaryIdentity {
    pub identity_type_code: BinaryIdentityTypeCode,
    pub identity: String,
}

impl BinaryIdentity {
    pub fn try_new(
        identity_type_code: BinaryIdentityTypeCode,
        identity: impl Into<String>,
    ) -> Result<Self, BinaryIdentityError> {
        let identity = identity.into().to_ascii_lowercase();
        if identity.is_empty()
            || identity.len() % 2 != 0
            || !identity.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(BinaryIdentityError::new(
                "binary identity must contain complete hexadecimal bytes",
            ));
        }
        if identity_type_code == BinaryIdentityTypeCode::ElfExecutableSampleSha256V1
            && identity.len() != 64
        {
            return Err(BinaryIdentityError::new(
                "ELF executable sample SHA-256 identity must contain 32 bytes",
            ));
        }
        Ok(Self {
            identity_type_code,
            identity,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryIdentityError {
    message: String,
}

impl BinaryIdentityError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for BinaryIdentityError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for BinaryIdentityError {}
