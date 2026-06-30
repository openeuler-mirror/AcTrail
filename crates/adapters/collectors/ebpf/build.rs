use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Copy, Eq, PartialEq)]
enum EventTransport {
    RingBuffer,
    PerfBuffer,
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

struct RingbufProbe {
    supported: bool,
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
    println!("cargo:rerun-if-changed=bpf/include/actrail_const.h");
    println!("cargo:rerun-if-changed=bpf/payload/actrail_socket_payload.h");
    println!("cargo:rerun-if-changed=bpf/payload/actrail_socket_payload_types.h");
    println!("cargo:rerun-if-changed=bpf/payload/actrail_stdio_payload.h");
    println!("cargo:rerun-if-changed=bpf/tls/actrail_tls_payload_capture.h");
    println!("cargo:rerun-if-changed=bpf/tls/actrail_tls_payload_completion.h");
    println!("cargo:rerun-if-changed=bpf/tls/actrail_tls_payload_diagnostics.h");
    println!("cargo:rerun-if-changed=bpf/tls/actrail_tls_payload_probes.h");
    println!("cargo:rerun-if-changed=/proc/sys/kernel/osrelease");
    println!("cargo:rerun-if-changed=/sys/kernel/btf/vmlinux");
    println!("cargo:rustc-check-cfg=cfg(actrail_event_transport_perf)");
    println!(
        "cargo:rustc-env=ACTRAIL_EBPF_OBJECT={}",
        object_path.display()
    );

    let transport = select_event_transport();
    println!(
        "cargo:rustc-env=ACTRAIL_EBPF_EVENT_TRANSPORT={}",
        transport.transport.as_str()
    );
    println!(
        "cargo:warning=AcTrail eBPF event transport: {} ({})",
        transport.transport.as_str(),
        transport.reason
    );

    let mut clang_args = vec!["-I", "bpf", bpf_target_arch];
    if transport.transport == EventTransport::PerfBuffer {
        println!("cargo:rustc-cfg=actrail_event_transport_perf");
        clang_args.push("-DACTRAIL_EVENT_TRANSPORT_PERF");
    }

    libbpf_cargo::SkeletonBuilder::new()
        .source("bpf/live_observation.bpf.c")
        .obj(&object_path)
        .clang_args(clang_args)
        .build()
        .expect("failed to compile eBPF object");
}

fn select_event_transport() -> TransportChoice {
    if env::var_os("CARGO_FEATURE_PERF_BUFFER").is_some() {
        return TransportChoice {
            transport: EventTransport::PerfBuffer,
            reason: "forced by Cargo feature perf-buffer".to_owned(),
        };
    }

    if let Some(probe) = probe_ringbuf_with_bpftool() {
        return choice_from_probe(probe);
    }

    if let Some(probe) = probe_ringbuf_with_vmlinux_btf() {
        return choice_from_probe(probe);
    }

    if let Some(probe) = probe_ringbuf_with_kernel_release() {
        return choice_from_probe(probe);
    }

    TransportChoice {
        transport: EventTransport::PerfBuffer,
        reason: "ringbuf support could not be detected".to_owned(),
    }
}

fn choice_from_probe(probe: RingbufProbe) -> TransportChoice {
    if probe.supported {
        TransportChoice {
            transport: EventTransport::RingBuffer,
            reason: probe.reason,
        }
    } else {
        TransportChoice {
            transport: EventTransport::PerfBuffer,
            reason: probe.reason,
        }
    }
}

fn probe_ringbuf_with_bpftool() -> Option<RingbufProbe> {
    let output = Command::new("bpftool")
        .args(["feature", "probe", "kernel", "unprivileged"])
        .output()
        .ok()?;
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
            supported: true,
            reason: "bpftool reported ringbuf map and helpers".to_owned(),
        })
    } else if has_map {
        Some(RingbufProbe {
            supported: true,
            reason: "bpftool reported ringbuf map support".to_owned(),
        })
    } else if lacks_map {
        Some(RingbufProbe {
            supported: false,
            reason: "bpftool reported ringbuf map is unavailable".to_owned(),
        })
    } else {
        None
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
        supported,
        reason: if supported {
            "vmlinux BTF contains ringbuf map and helper symbols".to_owned()
        } else {
            "vmlinux BTF does not contain ringbuf map/helper symbols".to_owned()
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
        supported,
        reason: if supported {
            format!("kernel release {major}.{minor} is >= 5.8")
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
