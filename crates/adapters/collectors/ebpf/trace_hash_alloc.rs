use std::env;
use std::fs;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum TraceHashMapAlloc {
    Prealloc,
    Lazy,
}

impl TraceHashMapAlloc {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Prealloc => "prealloc",
            Self::Lazy => "lazy",
        }
    }

    pub(super) fn clang_define(self) -> Option<&'static str> {
        match self {
            Self::Prealloc => Some("-DACTRAIL_BPF_TRACE_HASH_PREALLOC"),
            Self::Lazy => None,
        }
    }
}

pub(super) struct Choice {
    pub(super) alloc: TraceHashMapAlloc,
    pub(super) reason: String,
}

pub(super) fn select() -> Choice {
    match env::var("ACTRAIL_BPF_TRACE_HASH_ALLOC") {
        Ok(value) => match value.as_str() {
            "auto" => auto(),
            "prealloc" => Choice {
                alloc: TraceHashMapAlloc::Prealloc,
                reason: "forced by ACTRAIL_BPF_TRACE_HASH_ALLOC".to_owned(),
            },
            "lazy" => Choice {
                alloc: TraceHashMapAlloc::Lazy,
                reason: "forced by ACTRAIL_BPF_TRACE_HASH_ALLOC".to_owned(),
            },
            _ => {
                panic!("ACTRAIL_BPF_TRACE_HASH_ALLOC must be auto, prealloc, or lazy; got {value}")
            }
        },
        Err(env::VarError::NotPresent) => auto(),
        Err(error) => panic!("invalid ACTRAIL_BPF_TRACE_HASH_ALLOC: {error}"),
    }
}

fn auto() -> Choice {
    let host = env::var("HOST").expect("HOST must be set");
    let target = env::var("TARGET").expect("TARGET must be set");
    if host != target {
        panic!(
            "ACTRAIL_BPF_TRACE_HASH_ALLOC=auto cannot infer the deployment kernel while cross-compiling from {host} to {target}; select prealloc or lazy explicitly"
        );
    }

    let release = fs::read_to_string("/proc/sys/kernel/osrelease")
        .or_else(|_| super::uname_release())
        .expect("cannot determine the local kernel release for trace hash map allocation");
    let (major, minor) = super::parse_kernel_major_minor(&release)
        .expect("cannot parse the local kernel release for trace hash map allocation");
    if major > 6 || (major == 6 && minor >= 1) {
        Choice {
            alloc: TraceHashMapAlloc::Lazy,
            reason: format!(
                "local kernel {major}.{minor} lifted the trace NO_PREALLOC restriction (6.1+)"
            ),
        }
    } else {
        Choice {
            alloc: TraceHashMapAlloc::Prealloc,
            reason: format!(
                "local kernel {major}.{minor} predates the trace NO_PREALLOC relaxation"
            ),
        }
    }
}
