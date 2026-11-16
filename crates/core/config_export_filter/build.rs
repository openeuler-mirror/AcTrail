use std::{env, fs, path::PathBuf};

struct PolicyBundle;

impl PolicyBundle {
    fn write() {
        let directory =
            PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
        let policy = directory.join("allowlist.toml");
        println!("cargo:rerun-if-changed={}", policy.display());
        // Watching the directory also detects a policy added to a build that lacked it.
        println!("cargo:rerun-if-changed={}", directory.display());
        let contents = fs::read_to_string(&policy).map_err(|error| {
            format!("config export allowlist is unavailable: allowlist.toml: {error}")
        });
        let output = PathBuf::from(env::var_os("OUT_DIR").expect("build output directory"));
        fs::write(output.join("policy.rs"), format!("{contents:?}\n"))
            .expect("write compiled configuration policy");
    }
}

fn main() {
    PolicyBundle::write();
}
