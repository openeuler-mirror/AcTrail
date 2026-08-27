use std::env;
use std::path::PathBuf;

const BPF_SOURCES: &[&str] = &["bpf/sandbox_io.bpf.c", "bpf/sandbox_bpf_helpers.h"];

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR must be set"));
    let object_path = out_dir.join("sandbox_io.bpf.o");
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").expect("target architecture must be set");
    let bpf_target_arch = match target_arch.as_str() {
        "x86_64" => "-D__TARGET_ARCH_x86",
        "aarch64" => "-D__TARGET_ARCH_arm64",
        other => panic!("unsupported sandbox eBPF target architecture {other}"),
    };

    for source in BPF_SOURCES {
        println!("cargo:rerun-if-changed={source}");
    }
    println!("cargo:rerun-if-env-changed=ACTRAIL_BPF_SYSTEM_INCLUDE");
    println!(
        "cargo:rustc-env=ACTRAIL_SANDBOX_BPF_OBJECT={}",
        object_path.display()
    );

    let mut clang_args = vec![
        "-I".to_owned(),
        "bpf".to_owned(),
        bpf_target_arch.to_owned(),
    ];
    if let Some(include) = target_system_include(&target_arch) {
        clang_args.push(format!("-I{}", include.display()));
    }
    libbpf_cargo::SkeletonBuilder::new()
        .source("bpf/sandbox_io.bpf.c")
        .obj(&object_path)
        .clang_args(clang_args)
        .build()
        .expect("failed to compile sandbox Guest eBPF object");
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
