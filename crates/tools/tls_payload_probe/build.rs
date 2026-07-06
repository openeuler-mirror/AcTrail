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
        "cargo:rerun-if-env-changed=ACTRAIL_BPF_SYSTEM_INCLUDE"
    );
    let _ = writeln!(
        stdout,
        "cargo:rustc-env=TLS_PAYLOAD_PROBE_BPF_OBJECT={}",
        object_path.display()
    );
    let use_perf_buffer = env::var_os("CARGO_FEATURE_PERF_BUFFER").is_some();
    let mut clang_args = vec![
        "-I".to_string(),
        "bpf".to_string(),
        bpf_target_arch.to_string(),
    ];
    if let Some(include) = target_system_include(&target_arch) {
        clang_args.push(format!("-I{}", include.display()));
    }
    if use_perf_buffer {
        clang_args.push("-DTLS_PROBE_EVENT_TRANSPORT_PERF".to_string());
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
