//! Payload direction shared by capture runtimes.

use std::str::FromStr;

use crate::{CoreError, CoreResult};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadDirection {
    Inbound,
    Outbound,
}

impl PayloadDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbound => "inbound",
            Self::Outbound => "outbound",
        }
    }
}

impl FromStr for PayloadDirection {
    type Err = CoreError;

    fn from_str(value: &str) -> CoreResult<Self> {
        match value {
            "inbound" => Ok(Self::Inbound),
            "outbound" => Ok(Self::Outbound),
            _ => Err(CoreError::new(format!(
                "unknown payload direction: {value}"
            ))),
        }
    }
}
