//! File operation names and stable host ABI codes.
use actrail_plugin_abi::control as control_abi;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FilePolicyOperation {
    Any,
    Open,
    Read,
    Write,
    Delete,
    Attr,
    Rename,
    Link,
    Mkdir,
    Rmdir,
}

impl FilePolicyOperation {
    pub fn code(self) -> u8 {
        match self {
            Self::Any => control_abi::file_policy::operation_code::ANY,
            Self::Open => control_abi::file_policy::operation_code::OPEN,
            Self::Read => control_abi::file_policy::operation_code::READ,
            Self::Write => control_abi::file_policy::operation_code::WRITE,
            Self::Delete => control_abi::file_policy::operation_code::DELETE,
            Self::Attr => control_abi::file_policy::operation_code::ATTR,
            Self::Rename => control_abi::file_policy::operation_code::RENAME,
            Self::Link => control_abi::file_policy::operation_code::LINK,
            Self::Mkdir => control_abi::file_policy::operation_code::MKDIR,
            Self::Rmdir => control_abi::file_policy::operation_code::RMDIR,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Open => "open",
            Self::Read => "read",
            Self::Write => "write",
            Self::Delete => "delete",
            Self::Attr => "attr",
            Self::Rename => "rename",
            Self::Link => "link",
            Self::Mkdir => "mkdir",
            Self::Rmdir => "rmdir",
        }
    }

    pub fn from_code(code: u8) -> Result<Self, String> {
        match code {
            control_abi::file_policy::operation_code::ANY => Ok(Self::Any),
            control_abi::file_policy::operation_code::OPEN => Ok(Self::Open),
            control_abi::file_policy::operation_code::READ => Ok(Self::Read),
            control_abi::file_policy::operation_code::WRITE => Ok(Self::Write),
            control_abi::file_policy::operation_code::DELETE => Ok(Self::Delete),
            control_abi::file_policy::operation_code::ATTR => Ok(Self::Attr),
            control_abi::file_policy::operation_code::RENAME => Ok(Self::Rename),
            control_abi::file_policy::operation_code::LINK => Ok(Self::Link),
            control_abi::file_policy::operation_code::MKDIR => Ok(Self::Mkdir),
            control_abi::file_policy::operation_code::RMDIR => Ok(Self::Rmdir),
            _ => Err(format!("unsupported file policy operation code {code}")),
        }
    }

    pub fn from_wire(value: &str) -> Result<Self, String> {
        match value {
            "any" => Ok(Self::Any),
            "open" => Ok(Self::Open),
            "read" => Ok(Self::Read),
            "write" => Ok(Self::Write),
            "delete" => Ok(Self::Delete),
            "attr" => Ok(Self::Attr),
            "rename" => Ok(Self::Rename),
            "link" => Ok(Self::Link),
            "mkdir" => Ok(Self::Mkdir),
            "rmdir" => Ok(Self::Rmdir),
            other => Err(format!(
                "unsupported file policy operation {other}; expected any, open, read, write, delete, attr, rename, link, mkdir, or rmdir"
            )),
        }
    }

    pub fn matches(self, actual: Self) -> bool {
        self == Self::Any || self == actual
    }
}
