use std::{fmt, io, path::PathBuf};

#[derive(Debug)]
pub enum HostCgroupBindingError {
    ExpectedStartTimeMissing,
    PidReuse,
    MembershipUnstable,
    ContainerIdentityMissing,
    ContainerIdentityMismatch,
    RootMoved,
    Malformed(String),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Unsupported(String),
}

impl fmt::Display for HostCgroupBindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExpectedStartTimeMissing => {
                formatter.write_str("expected non-zero process start time is required")
            }
            Self::PidReuse => formatter.write_str("process start time changed (PID reuse)"),
            Self::MembershipUnstable => {
                formatter.write_str("process cgroup membership remained unstable while binding")
            }
            Self::ContainerIdentityMissing => {
                formatter.write_str("process is not in a recognized host container cgroup")
            }
            Self::ContainerIdentityMismatch => {
                formatter.write_str("container identity no longer matches the binding")
            }
            Self::RootMoved => formatter.write_str("process moved outside the bound cgroup"),
            Self::Malformed(detail) => write!(formatter, "malformed cgroup identity: {detail}"),
            Self::Io {
                operation,
                path,
                source,
            } => write!(formatter, "{operation} {}: {source}", path.display()),
            Self::Unsupported(detail) => formatter.write_str(detail),
        }
    }
}

impl std::error::Error for HostCgroupBindingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
