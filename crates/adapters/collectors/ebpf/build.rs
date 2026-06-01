use std::env;
use std::path::PathBuf;

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
    println!(
        "cargo:rustc-env=ACTRAIL_EBPF_OBJECT={}",
        object_path.display()
    );

    libbpf_cargo::SkeletonBuilder::new()
        .source("bpf/live_observation.bpf.c")
        .obj(&object_path)
        .clang_args(["-I", "bpf", bpf_target_arch])
        .build()
        .expect("failed to compile eBPF object");
}
