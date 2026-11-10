//! Select kernel variants and compile observation objects.
use std::{env, path::PathBuf};

#[path = "build/environment.rs"]
mod environment;
mod trace_hash_alloc;
use environment::*;

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR must be set"));
    let object_path = out_dir.join("live_observation.bpf.o");
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").expect("CARGO_CFG_TARGET_ARCH must be set");
    let bpf_target_arch = match target_arch.as_str() {
        "x86_64" => "-D__TARGET_ARCH_x86",
        "aarch64" => "-D__TARGET_ARCH_arm64",
        other => panic!("unsupported eBPF target architecture {other}"),
    };

    println!("cargo:rerun-if-changed=bpf");
    println!("cargo:rerun-if-changed=/sys/kernel/btf/vmlinux");
    // /proc/sys/kernel/osrelease reports a synthetic mtime, so watching it made
    // every build re-run this script. The kernel release is still read at build
    // time; ACTRAIL_BPF_KERNEL_RELEASE is the explicit override for rebuilds
    // after a live kernel change.
    println!("cargo:rerun-if-env-changed=ACTRAIL_BPF_KERNEL_RELEASE");
    println!("cargo:rerun-if-env-changed=ACTRAIL_BPF_SYSTEM_INCLUDE");
    println!("cargo:rerun-if-env-changed=ACTRAIL_EBPF_EVENT_TRANSPORT");
    println!("cargo:rerun-if-env-changed=ACTRAIL_LAUNCH_BINDING_BACKEND");
    println!("cargo:rerun-if-env-changed=ACTRAIL_BPF_ONCE_BACKEND");
    println!("cargo:rerun-if-env-changed=ACTRAIL_BPF_TRACE_HASH_ALLOC");
    println!("cargo:rerun-if-env-changed=ACTRAIL_TLS_DIRECT_COPY_MAX_CHUNKS");
    println!("cargo:rustc-check-cfg=cfg(actrail_event_transport_perf)");
    println!("cargo:rustc-check-cfg=cfg(actrail_launch_binding_task_storage)");
    println!("cargo:rustc-check-cfg=cfg(actrail_launch_binding_pid_generation_hash)");
    println!("cargo:rustc-check-cfg=cfg(actrail_bpf_once_cas)");
    println!(
        "cargo:rustc-env=ACTRAIL_EBPF_OBJECT={}",
        object_path.display()
    );

    let transport = select_event_transport();
    let launch_binding = select_launch_binding_backend();
    let once_backend = select_bpf_once_backend();
    let trace_hash_alloc = trace_hash_alloc::select();
    let bpf_loop_supported = supports_bpf_loop();
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
    println!(
        "cargo:rustc-env=ACTRAIL_BPF_ONCE_BACKEND={}",
        once_backend.backend.as_str()
    );
    if once_backend.backend == OnceBackend::Cas {
        println!("cargo:rustc-cfg=actrail_bpf_once_cas");
    }
    println!(
        "cargo:warning=AcTrail BPF once-only backend: {} ({})",
        once_backend.backend.as_str(),
        once_backend.reason
    );
    println!(
        "cargo:warning=AcTrail trace hash map allocation: {} ({})",
        trace_hash_alloc.alloc.as_str(),
        trace_hash_alloc.reason
    );

    let mut clang_args = vec![
        "-I".to_string(),
        "bpf".to_string(),
        bpf_target_arch.to_string(),
    ];
    let tls_direct_copy_max_chunks = env::var("ACTRAIL_TLS_DIRECT_COPY_MAX_CHUNKS")
        .expect("ACTRAIL_TLS_DIRECT_COPY_MAX_CHUNKS must be set by Cargo configuration");
    clang_args.push(format!(
        "-DACTRAIL_TLS_DIRECT_COPY_MAX_CHUNKS={tls_direct_copy_max_chunks}"
    ));
    if let Some(include) = target_system_include(&target_arch) {
        clang_args.push(format!("-I{}", include.display()));
    }
    if transport.transport == EventTransport::PerfBuffer {
        println!("cargo:rustc-cfg=actrail_event_transport_perf");
        clang_args.push("-DACTRAIL_EVENT_TRANSPORT_PERF".to_string());
    }
    if bpf_loop_supported {
        clang_args.push("-DACTRAIL_BPF_LOOP".to_string());
        println!("cargo:warning=AcTrail bounded iterators: bpf_loop");
    } else {
        println!("cargo:warning=AcTrail bounded iterators: verifier loop fallback");
    }
    clang_args.push(launch_binding.backend.clang_define().to_string());
    if let Some(define) = once_backend.backend.clang_define() {
        clang_args.push(define.to_string());
    }
    if let Some(define) = trace_hash_alloc.alloc.clang_define() {
        clang_args.push(define.to_string());
    }

    libbpf_cargo::SkeletonBuilder::new()
        .source("bpf/live_observation.bpf.c")
        .obj(&object_path)
        .clang_args(&clang_args)
        .build()
        .expect("failed to compile eBPF object");
}
