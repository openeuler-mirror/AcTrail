use std::env;
use std::path::PathBuf;
use std::process::Command;

struct LsmObjectBuilder {
    out: PathBuf,
    system_include: PathBuf,
}

impl LsmObjectBuilder {
    fn from_environment() -> Self {
        let arch = env::var("CARGO_CFG_TARGET_ARCH").expect("target architecture");
        let default_include = match arch.as_str() {
            "x86_64" => "/usr/include/x86_64-linux-gnu",
            "aarch64" => "/usr/include/aarch64-linux-gnu",
            other => panic!("LSM BPF does not support target architecture {other}"),
        };
        Self {
            out: PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")),
            system_include: env::var_os("ACTRAIL_BPF_SYSTEM_INCLUDE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(default_include)),
        }
    }

    fn compile(&self, variant: &str, hash: bool) {
        let path = self.out.join(format!("access_control_{variant}.bpf.o"));
        let mut command = Command::new("clang");
        command.args(["-target", "bpf", "-O2", "-g", "-Wall", "-Werror"]);
        command.arg("-I").arg(&self.system_include);
        if hash {
            command.arg("-DAC_PENDING_HASH");
        }
        let status = command
            .args(["-c", "bpf/access_control.bpf.c", "-o"])
            .arg(&path)
            .status()
            .expect("run clang for LSM BPF object");
        assert!(status.success(), "LSM BPF {variant} compilation failed");
        println!(
            "cargo:rustc-env=ACTRAIL_LSM_{}_OBJECT={}",
            variant.to_uppercase(),
            path.display()
        );
    }
}

fn main() {
    println!("cargo:rerun-if-changed=bpf");
    println!("cargo:rerun-if-changed=../adapters/collectors/ebpf/bpf/common");
    println!("cargo:rerun-if-env-changed=ACTRAIL_BPF_SYSTEM_INCLUDE");
    let builder = LsmObjectBuilder::from_environment();
    builder.compile("task", false);
    builder.compile("hash", true);
}
