//! Build-time selection of kernel-compatible object variants.
use std::{env, fs, path::PathBuf, process::Command};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum EventTransport {
    RingBuffer,
    PerfBuffer,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum LaunchBindingBackend {
    TaskStorage,
    PidGenerationHash,
}

impl LaunchBindingBackend {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::TaskStorage => "task-storage",
            Self::PidGenerationHash => "pid-generation-hash",
        }
    }

    pub(super) fn clang_define(self) -> &'static str {
        match self {
            Self::TaskStorage => "-DACTRAIL_LAUNCH_BINDING_TASK_STORAGE",
            Self::PidGenerationHash => "-DACTRAIL_LAUNCH_BINDING_PID_GENERATION_HASH",
        }
    }

    pub(super) fn rust_cfg(self) -> &'static str {
        match self {
            Self::TaskStorage => "actrail_launch_binding_task_storage",
            Self::PidGenerationHash => "actrail_launch_binding_pid_generation_hash",
        }
    }
}

pub(super) struct LaunchBindingChoice {
    pub(super) backend: LaunchBindingBackend,
    pub(super) reason: String,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum OnceBackend {
    Cas,
    MapOnce,
}

impl OnceBackend {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Cas => "cas",
            Self::MapOnce => "map-once",
        }
    }

    pub(super) fn clang_define(self) -> Option<&'static str> {
        match self {
            Self::Cas => Some("-DACTRAIL_BPF_ONCE_CAS"),
            Self::MapOnce => None,
        }
    }
}

pub(super) struct OnceChoice {
    pub(super) backend: OnceBackend,
    pub(super) reason: String,
}

impl EventTransport {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::RingBuffer => "ring-buffer",
            Self::PerfBuffer => "perf-buffer",
        }
    }
}

pub(super) struct TransportChoice {
    pub(super) transport: EventTransport,
    pub(super) reason: String,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ProbeVerdict {
    Supported,
    Unsupported,
    Inconclusive,
}

struct RingbufProbe {
    verdict: ProbeVerdict,
    reason: String,
}

pub(super) fn supports_bpf_loop() -> bool {
    if fs::read("/sys/kernel/btf/vmlinux")
        .ok()
        .is_some_and(|btf| contains_bytes(&btf, b"bpf_loop"))
    {
        return true;
    }
    let Some(release) = kernel_release() else {
        return false;
    };
    parse_kernel_major_minor(&release)
        .is_some_and(|(major, minor)| major > 5 || (major == 5 && minor >= 17))
}

pub(super) fn select_launch_binding_backend() -> LaunchBindingChoice {
    match env::var("ACTRAIL_LAUNCH_BINDING_BACKEND") {
        Ok(value) => match value.as_str() {
            "auto" => auto_launch_binding_backend(),
            "task-storage" => LaunchBindingChoice {
                backend: LaunchBindingBackend::TaskStorage,
                reason: "forced by ACTRAIL_LAUNCH_BINDING_BACKEND".to_owned(),
            },
            "pid-generation-hash" => LaunchBindingChoice {
                backend: LaunchBindingBackend::PidGenerationHash,
                reason: "forced by ACTRAIL_LAUNCH_BINDING_BACKEND".to_owned(),
            },
            _ => panic!(
                "ACTRAIL_LAUNCH_BINDING_BACKEND must be auto, task-storage, or pid-generation-hash; got {value}"
            ),
        },
        Err(env::VarError::NotPresent) => auto_launch_binding_backend(),
        Err(error) => panic!("invalid ACTRAIL_LAUNCH_BINDING_BACKEND: {error}"),
    }
}

fn auto_launch_binding_backend() -> LaunchBindingChoice {
    let host = env::var("HOST").expect("HOST must be set");
    let target = env::var("TARGET").expect("TARGET must be set");
    if host != target {
        panic!(
            "ACTRAIL_LAUNCH_BINDING_BACKEND=auto cannot infer the deployment kernel while cross-compiling from {host} to {target}; select task-storage or pid-generation-hash explicitly"
        );
    }

    if task_storage_reported_by_bpftool() {
        return LaunchBindingChoice {
            backend: LaunchBindingBackend::TaskStorage,
            reason: "privileged bpftool reported the task-storage map and required helpers"
                .to_owned(),
        };
    }
    if task_storage_reported_by_vmlinux_btf() {
        return LaunchBindingChoice {
            backend: LaunchBindingBackend::TaskStorage,
            reason: "vmlinux BTF contains the task-storage map and required helpers".to_owned(),
        };
    }

    let release = kernel_release()
        .expect("cannot determine the local kernel release for launch binding selection");
    let (major, minor) = parse_kernel_major_minor(&release)
        .expect("cannot parse the local kernel release for launch binding selection");
    if major < 5 || (major == 5 && minor < 11) {
        return LaunchBindingChoice {
            backend: LaunchBindingBackend::PidGenerationHash,
            reason: format!(
                "local kernel {major}.{minor} predates upstream BPF task-storage support"
            ),
        };
    }

    panic!(
        "kernel {major}.{minor} may support BPF task-storage, but neither privileged bpftool nor vmlinux BTF proved the required map and helpers; set ACTRAIL_LAUNCH_BINDING_BACKEND explicitly"
    );
}

pub(super) fn select_bpf_once_backend() -> OnceChoice {
    match env::var("ACTRAIL_BPF_ONCE_BACKEND") {
        Ok(value) => match value.as_str() {
            "auto" => auto_bpf_once_backend(),
            "cas" => OnceChoice {
                backend: OnceBackend::Cas,
                reason: "forced by ACTRAIL_BPF_ONCE_BACKEND".to_owned(),
            },
            "map-once" => OnceChoice {
                backend: OnceBackend::MapOnce,
                reason: "forced by ACTRAIL_BPF_ONCE_BACKEND".to_owned(),
            },
            _ => panic!("ACTRAIL_BPF_ONCE_BACKEND must be auto, cas, or map-once; got {value}"),
        },
        Err(env::VarError::NotPresent) => auto_bpf_once_backend(),
        Err(error) => panic!("invalid ACTRAIL_BPF_ONCE_BACKEND: {error}"),
    }
}

fn auto_bpf_once_backend() -> OnceChoice {
    let host = env::var("HOST").expect("HOST must be set");
    let target = env::var("TARGET").expect("TARGET must be set");
    if host != target {
        panic!(
            "ACTRAIL_BPF_ONCE_BACKEND=auto cannot infer the deployment kernel while cross-compiling from {host} to {target}; select cas or map-once explicitly"
        );
    }

    let release = kernel_release()
        .expect("cannot determine the local kernel release for BPF once-only backend selection");
    let (major, minor) = parse_kernel_major_minor(&release)
        .expect("cannot parse the local kernel release for BPF once-only backend selection");
    if major > 5 || (major == 5 && minor >= 12) {
        OnceChoice {
            backend: OnceBackend::Cas,
            reason: format!("local kernel {major}.{minor} supports BPF_ATOMIC cmpxchg (5.12+)"),
        }
    } else {
        OnceChoice {
            backend: OnceBackend::MapOnce,
            reason: format!("local kernel {major}.{minor} predates BPF_ATOMIC cmpxchg"),
        }
    }
}

fn task_storage_reported_by_bpftool() -> bool {
    let output = Command::new("bpftool")
        .args(["feature", "probe", "kernel"])
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let report = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    report.contains("eBPF map_type task_storage is available")
        && [
            "bpf_task_storage_get",
            "bpf_task_storage_delete",
            "bpf_get_current_task_btf",
        ]
        .into_iter()
        .all(|helper| report.contains(helper))
}

fn task_storage_reported_by_vmlinux_btf() -> bool {
    let Ok(btf) = fs::read("/sys/kernel/btf/vmlinux") else {
        return false;
    };
    [
        b"BPF_MAP_TYPE_TASK_STORAGE".as_slice(),
        b"BPF_FUNC_task_storage_get".as_slice(),
        b"BPF_FUNC_task_storage_delete".as_slice(),
        b"BPF_FUNC_get_current_task_btf".as_slice(),
    ]
    .into_iter()
    .all(|marker| contains_bytes(&btf, marker))
}

pub(super) fn target_system_include(target_arch: &str) -> Option<PathBuf> {
    if let Some(path) = env::var_os("ACTRAIL_BPF_SYSTEM_INCLUDE") {
        return Some(PathBuf::from(path));
    }
    let multiarch = match target_arch {
        "x86_64" => "x86_64-linux-gnu",
        "aarch64" => "aarch64-linux-gnu",
        _ => return None,
    };
    let path = PathBuf::from("/usr/include").join(multiarch);
    path.join("asm").is_dir().then_some(path)
}

pub(super) fn select_event_transport() -> TransportChoice {
    let perf_feature = env::var_os("CARGO_FEATURE_PERF_BUFFER").is_some();
    if let Some(requested) = env::var_os("ACTRAIL_EBPF_EVENT_TRANSPORT") {
        let requested = requested.to_string_lossy();
        match requested.as_ref() {
            "" | "auto" => {}
            "perf-buffer" => {
                return TransportChoice {
                    transport: EventTransport::PerfBuffer,
                    reason: "forced by ACTRAIL_EBPF_EVENT_TRANSPORT".to_owned(),
                };
            }
            "ring-buffer" if perf_feature => {
                panic!(
                    "ACTRAIL_EBPF_EVENT_TRANSPORT=ring-buffer conflicts with Cargo feature perf-buffer"
                );
            }
            "ring-buffer" => {
                return TransportChoice {
                    transport: EventTransport::RingBuffer,
                    reason: "forced by ACTRAIL_EBPF_EVENT_TRANSPORT".to_owned(),
                };
            }
            other => {
                panic!(
                    "unsupported ACTRAIL_EBPF_EVENT_TRANSPORT={other}; expected auto, perf-buffer, or ring-buffer"
                );
            }
        }
    }

    if perf_feature {
        return TransportChoice {
            transport: EventTransport::PerfBuffer,
            reason: "forced by Cargo feature perf-buffer".to_owned(),
        };
    }

    if let Some(probe) = probe_ringbuf_with_bpftool(false) {
        if let Some(choice) = choice_from_probe(probe) {
            return choice;
        }
    }

    if let Some(probe) = probe_ringbuf_with_bpftool(true) {
        if let Some(choice) = choice_from_probe(probe) {
            return choice;
        }
    }

    if let Some(probe) = probe_ringbuf_with_vmlinux_btf() {
        if let Some(choice) = choice_from_probe(probe) {
            return choice;
        }
    }

    if let Some(probe) = probe_ringbuf_with_kernel_release() {
        if let Some(choice) = choice_from_probe(probe) {
            return choice;
        }
    }

    TransportChoice {
        transport: EventTransport::PerfBuffer,
        reason: "ringbuf support could not be detected".to_owned(),
    }
}

fn choice_from_probe(probe: RingbufProbe) -> Option<TransportChoice> {
    match probe.verdict {
        ProbeVerdict::Supported => Some(TransportChoice {
            transport: EventTransport::RingBuffer,
            reason: probe.reason,
        }),
        ProbeVerdict::Unsupported => Some(TransportChoice {
            transport: EventTransport::PerfBuffer,
            reason: probe.reason,
        }),
        ProbeVerdict::Inconclusive => None,
    }
}

fn probe_ringbuf_with_bpftool(unprivileged: bool) -> Option<RingbufProbe> {
    let mut command = Command::new("bpftool");
    command.args(["feature", "probe", "kernel"]);
    if unprivileged {
        command.arg("unprivileged");
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let report = format!("{stdout}\n{stderr}");
    let has_map = report.contains("eBPF map_type ringbuf is available");
    let lacks_map = report.contains("eBPF map_type ringbuf is NOT available");
    let has_helpers = [
        "bpf_ringbuf_output",
        "bpf_ringbuf_reserve",
        "bpf_ringbuf_submit",
        "bpf_ringbuf_discard",
    ]
    .into_iter()
    .all(|helper| report.contains(helper));

    if has_map && has_helpers {
        Some(RingbufProbe {
            verdict: ProbeVerdict::Supported,
            reason: if unprivileged {
                "unprivileged bpftool reported ringbuf map and helpers".to_owned()
            } else {
                "privileged bpftool reported ringbuf map and helpers".to_owned()
            },
        })
    } else if has_map {
        Some(RingbufProbe {
            verdict: ProbeVerdict::Inconclusive,
            reason: if unprivileged {
                "unprivileged bpftool did not expose every required ringbuf helper".to_owned()
            } else {
                "privileged bpftool did not expose every required ringbuf helper".to_owned()
            },
        })
    } else if lacks_map {
        Some(RingbufProbe {
            verdict: if unprivileged {
                ProbeVerdict::Inconclusive
            } else {
                ProbeVerdict::Unsupported
            },
            reason: if unprivileged {
                "unprivileged bpftool cannot establish privileged ringbuf availability".to_owned()
            } else {
                "privileged bpftool reported ringbuf map is unavailable".to_owned()
            },
        })
    } else {
        Some(RingbufProbe {
            verdict: ProbeVerdict::Inconclusive,
            reason: if unprivileged {
                "unprivileged bpftool did not report ringbuf capability".to_owned()
            } else {
                "privileged bpftool did not report ringbuf capability".to_owned()
            },
        })
    }
}

fn probe_ringbuf_with_vmlinux_btf() -> Option<RingbufProbe> {
    let btf = fs::read("/sys/kernel/btf/vmlinux").ok()?;
    let markers: &[&[u8]] = &[
        b"BPF_MAP_TYPE_RINGBUF",
        b"bpf_ringbuf_output",
        b"bpf_ringbuf_reserve",
        b"bpf_ringbuf_submit",
        b"bpf_ringbuf_discard",
    ];
    let supported = markers.iter().all(|marker| contains_bytes(&btf, marker));
    Some(RingbufProbe {
        verdict: if supported {
            ProbeVerdict::Supported
        } else {
            ProbeVerdict::Inconclusive
        },
        reason: if supported {
            "vmlinux BTF contains ringbuf map and helper symbols".to_owned()
        } else {
            "vmlinux BTF cannot confirm every ringbuf map/helper symbol".to_owned()
        },
    })
}

fn probe_ringbuf_with_kernel_release() -> Option<RingbufProbe> {
    let release = kernel_release()?;
    let (major, minor) = parse_kernel_major_minor(&release)?;
    let supported = major > 5 || (major == 5 && minor >= 8);
    Some(RingbufProbe {
        verdict: if supported {
            ProbeVerdict::Inconclusive
        } else {
            ProbeVerdict::Unsupported
        },
        reason: if supported {
            format!("kernel release {major}.{minor} permits but does not confirm ringbuf")
        } else {
            format!("kernel release {major}.{minor} is < 5.8")
        },
    })
}

pub(super) fn uname_release() -> std::io::Result<String> {
    let output = Command::new("uname").arg("-r").output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(std::io::Error::other("uname -r failed"))
    }
}

/// Kernel release used for feature selection. `ACTRAIL_BPF_KERNEL_RELEASE`
/// overrides the probe so a rebuild can be forced after a live kernel change
/// without watching /proc, whose mtime is synthetic.
fn kernel_release() -> Option<String> {
    if let Ok(value) = env::var("ACTRAIL_BPF_KERNEL_RELEASE") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    fs::read_to_string("/proc/sys/kernel/osrelease")
        .or_else(|_| uname_release())
        .ok()
}

pub(super) fn parse_kernel_major_minor(release: &str) -> Option<(u32, u32)> {
    let mut parts = release
        .split(|value: char| !value.is_ascii_digit())
        .filter(|value| !value.is_empty());
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
