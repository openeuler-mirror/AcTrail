//! Network operations and transport protocols used by policy and grants.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkPolicyOperation {
    #[default]
    Connect,
    Send,
    Bind,
    Listen,
}

impl NetworkPolicyOperation {
    pub fn from_wire(value: &str) -> Result<Self, String> {
        match value {
            "connect" => Ok(Self::Connect),
            "send" => Ok(Self::Send),
            "bind" => Ok(Self::Bind),
            "listen" => Ok(Self::Listen),
            _ => Err(format!("invalid network operation {value}")),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connect => "connect",
            Self::Send => "send",
            Self::Bind => "bind",
            Self::Listen => "listen",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkPolicyProtocol {
    #[default]
    Any,
    Tcp,
    Udp,
}

impl NetworkPolicyProtocol {
    pub fn from_wire(value: &str) -> Result<Self, String> {
        match value {
            "any" => Ok(Self::Any),
            "tcp" => Ok(Self::Tcp),
            "udp" => Ok(Self::Udp),
            _ => Err(format!("invalid network protocol {value}")),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
    pub fn code(self) -> u8 {
        match self {
            Self::Any => 0,
            Self::Tcp => 6,
            Self::Udp => 17,
        }
    }
    pub fn covers(self, other: Self) -> bool {
        self == Self::Any || self == other
    }
}
