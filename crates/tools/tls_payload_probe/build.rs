use std::env;
use std::io::Write;
use std::path::PathBuf;

const BPF_SOURCES: &[&str] = &[
    "bpf/tls_payload_probe.bpf.c",
    "bpf/tls_payload_probe_capture.h",
    "bpf/tls_payload_probe_helpers.h",
    "bpf/tls_payload_probe_maps.h",
    "bpf/tls_payload_probe_types.h",
];

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR must be set"));
    let object_path = out_dir.join("tls_payload_probe.bpf.o");
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").expect("CARGO_CFG_TARGET_ARCH must be set");
    let bpf_target_arch = match target_arch.as_str() {
        "x86_64" => "-D__TARGET_ARCH_x86",
        "aarch64" => "-D__TARGET_ARCH_arm64",
        other => panic!("unsupported eBPF target architecture {other}"),
    };

    let mut stdout = std::io::stdout().lock();
    for source in BPF_SOURCES {
        let _ = writeln!(stdout, "cargo:rerun-if-changed={source}");
    }
    let _ = writeln!(
        stdout,
        "cargo:rustc-env=TLS_PAYLOAD_PROBE_BPF_OBJECT={}",
        object_path.display()
    );
    let use_perf_buffer = env::var_os("CARGO_FEATURE_PERF_BUFFER").is_some();
    let mut clang_args = vec!["-I", "bpf", bpf_target_arch];
    if use_perf_buffer {
        clang_args.push("-DTLS_PROBE_EVENT_TRANSPORT_PERF");
    }
    let _ = writeln!(
        stdout,
        "cargo:warning=tls-payload-probe event transport: {}",
        if use_perf_buffer {
            "perf-buffer"
        } else {
            "ring-buffer"
        }
    );

    libbpf_cargo::SkeletonBuilder::new()
        .source("bpf/tls_payload_probe.bpf.c")
        .obj(&object_path)
        .clang_args(clang_args)
        .build()
        .expect("failed to compile tls-payload-probe eBPF object");
}
