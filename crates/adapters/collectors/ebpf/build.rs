use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Copy, Eq, PartialEq)]
enum EventTransport {
    RingBuffer,
    PerfBuffer,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum LaunchBindingBackend {
    TaskStorage,
    PidGenerationHash,
}

impl LaunchBindingBackend {
    fn as_str(self) -> &'static str {
        match self {
            Self::TaskStorage => "task-storage",
            Self::PidGenerationHash => "pid-generation-hash",
        }
    }

    fn clang_define(self) -> &'static str {
        match self {
            Self::TaskStorage => "-DACTRAIL_LAUNCH_BINDING_TASK_STORAGE",
            Self::PidGenerationHash => "-DACTRAIL_LAUNCH_BINDING_PID_GENERATION_HASH",
        }
    }

    fn rust_cfg(self) -> &'static str {
        match self {
            Self::TaskStorage => "actrail_launch_binding_task_storage",
            Self::PidGenerationHash => "actrail_launch_binding_pid_generation_hash",
        }
    }
}

struct LaunchBindingChoice {
    backend: LaunchBindingBackend,
    reason: String,
}

impl EventTransport {
    fn as_str(self) -> &'static str {
        match self {
            Self::RingBuffer => "ring-buffer",
            Self::PerfBuffer => "perf-buffer",
        }
    }
}

struct TransportChoice {
    transport: EventTransport,
    reason: String,
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

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR must be set"));
    let object_path = out_dir.join("live_observation.bpf.o");
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").expect("CARGO_CFG_TARGET_ARCH must be set");
    let bpf_target_arch = match target_arch.as_str() {
        "x86_64" => "-D__TARGET_ARCH_x86",
        "aarch64" => "-D__TARGET_ARCH_arm64",
        other => panic!("unsupported eBPF target architecture {other}"),
    };

    println!("cargo:rerun-if-changed=bpf/live_observation.bpf.c");
    println!("cargo:rerun-if-changed=bpf/actrail_helpers.h");
    println!("cargo:rerun-if-changed=bpf/actrail_file.h");
    println!("cargo:rerun-if-changed=bpf/actrail_net.h");
    println!("cargo:rerun-if-changed=bpf/actrail_proc.h");
    println!("cargo:rerun-if-changed=bpf/actrail_runtime.h");
    println!("cargo:rerun-if-changed=bpf/actrail_suppressed_fd.h");
    println!("cargo:rerun-if-changed=bpf/actrail_tls_payload.h");
    println!("cargo:rerun-if-changed=bpf/actrail_uprobe_regs.h");
    println!("cargo:rerun-if-changed=bpf/launch_binding/actrail_launch_binding.h");
    println!("cargo:rerun-if-changed=bpf/launch_binding/impl/task_storage.h");
    println!("cargo:rerun-if-changed=bpf/launch_binding/impl/pid_generation_hash.h");
    println!("cargo:rerun-if-changed=bpf/file/actrail_file_bulk_read_fast.h");
    println!("cargo:rerun-if-changed=bpf/file/actrail_file_path.h");
    println!("cargo:rerun-if-changed=bpf/include/actrail_const.h");
    println!("cargo:rerun-if-changed=bpf/payload/actrail_socket_fd_state.h");
    println!("cargo:rerun-if-changed=bpf/payload/actrail_socket_payload.h");
    println!("cargo:rerun-if-changed=bpf/payload/actrail_socket_tls.h");
    println!("cargo:rerun-if-changed=bpf/payload/actrail_socket_payload_types.h");
    println!("cargo:rerun-if-changed=bpf/payload/actrail_stdio_payload.h");
    println!("cargo:rerun-if-changed=bpf/tls/actrail_tls_payload_capture.h");
    println!("cargo:rerun-if-changed=bpf/tls/actrail_tls_payload_completion.h");
    println!("cargo:rerun-if-changed=bpf/tls/actrail_tls_payload_diagnostics.h");
    println!("cargo:rerun-if-changed=bpf/tls/actrail_tls_payload_probes.h");
    println!("cargo:rerun-if-changed=bpf/tls/actrail_tls_rustls_internal.h");
    println!("cargo:rerun-if-changed=/proc/sys/kernel/osrelease");
    println!("cargo:rerun-if-changed=/sys/kernel/btf/vmlinux");
    println!("cargo:rerun-if-env-changed=ACTRAIL_BPF_SYSTEM_INCLUDE");
    println!("cargo:rerun-if-env-changed=ACTRAIL_LAUNCH_BINDING_BACKEND");
    println!("cargo:rustc-check-cfg=cfg(actrail_event_transport_perf)");
    println!("cargo:rustc-check-cfg=cfg(actrail_launch_binding_task_storage)");
    println!("cargo:rustc-check-cfg=cfg(actrail_launch_binding_pid_generation_hash)");
    println!(
        "cargo:rustc-env=ACTRAIL_EBPF_OBJECT={}",
        object_path.display()
    );

    let transport = select_event_transport();
    let launch_binding = select_launch_binding_backend();
    println!(
        "cargo:rustc-env=ACTRAIL_EBPF_EVENT_TRANSPORT={}",
        transport.transport.as_str()
    );
    println!(
        "cargo:warning=AcTrail eBPF event transport: {} ({})",
        transport.transport.as_str(),
        transport.reason
    );
    println!(
        "cargo:rustc-env=ACTRAIL_LAUNCH_BINDING_BACKEND={}",
        launch_binding.backend.as_str()
    );
    println!("cargo:rustc-cfg={}", launch_binding.backend.rust_cfg());
    println!(
        "cargo:warning=AcTrail launch binding backend: {} ({})",
        launch_binding.backend.as_str(),
        launch_binding.reason
    );

    let mut clang_args = vec![
        "-I".to_string(),
        "bpf".to_string(),
        bpf_target_arch.to_string(),
    ];
    if let Some(include) = target_system_include(&target_arch) {
        clang_args.push(format!("-I{}", include.display()));
    }
    if transport.transport == EventTransport::PerfBuffer {
        println!("cargo:rustc-cfg=actrail_event_transport_perf");
        clang_args.push("-DACTRAIL_EVENT_TRANSPORT_PERF".to_string());
    }
    clang_args.push(launch_binding.backend.clang_define().to_string());

    libbpf_cargo::SkeletonBuilder::new()
        .source("bpf/live_observation.bpf.c")
        .obj(&object_path)
        .clang_args(clang_args)
        .build()
        .expect("failed to compile eBPF object");
}

fn select_launch_binding_backend() -> LaunchBindingChoice {
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

    let release = fs::read_to_string("/proc/sys/kernel/osrelease")
        .or_else(|_| uname_release())
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

fn target_system_include(target_arch: &str) -> Option<PathBuf> {
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

fn select_event_transport() -> TransportChoice {
    if env::var_os("CARGO_FEATURE_PERF_BUFFER").is_some() {
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
    let release = fs::read_to_string("/proc/sys/kernel/osrelease")
        .or_else(|_| uname_release())
        .ok()?;
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

fn uname_release() -> std::io::Result<String> {
    let output = Command::new("uname").arg("-r").output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(std::io::Error::other("uname -r failed"))
    }
}

fn parse_kernel_major_minor(release: &str) -> Option<(u32, u32)> {
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
