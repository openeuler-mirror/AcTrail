use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const FRONTEND_DIR: &str = "frontend";
const FRONTEND_SRC_DIR: &str = "frontend/src";
const FRONTEND_INDEX: &str = "frontend/index.html";
const FRONTEND_PACKAGE: &str = "frontend/package.json";
const FRONTEND_LOCK: &str = "frontend/package-lock.json";
const FRONTEND_VITE_CONFIG: &str = "frontend/vite.config.js";
const DIST_INDEX: &str = "src/render/dist/index.html";

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    print_rerun_inputs(&manifest_dir);
    build_frontend(&manifest_dir);
    normalize_dist_index(&manifest_dir);
}

fn print_rerun_inputs(manifest_dir: &Path) {
    println!("cargo:rerun-if-env-changed=PATH");
    for path in [
        FRONTEND_INDEX,
        FRONTEND_PACKAGE,
        FRONTEND_LOCK,
        FRONTEND_VITE_CONFIG,
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    print_rerun_dir(&manifest_dir.join(FRONTEND_SRC_DIR));
}

fn print_rerun_dir(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
    let entries = fs::read_dir(path).unwrap_or_else(|error| {
        panic!("failed to read frontend input {}: {error}", path.display())
    });
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!(
                "failed to read frontend input under {}: {error}",
                path.display()
            )
        });
        let path = entry.path();
        if path.is_dir() {
            print_rerun_dir(&path);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

fn build_frontend(manifest_dir: &Path) {
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let status = Command::new(npm)
        .args(["run", "build"])
        .current_dir(manifest_dir.join(FRONTEND_DIR))
        .status()
        .unwrap_or_else(|error| panic!("failed to start npm frontend build: {error}"));
    if !status.success() {
        panic!("npm frontend build failed with status {status}");
    }
}

fn normalize_dist_index(manifest_dir: &Path) {
    let path = manifest_dir.join(DIST_INDEX);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read generated {}: {error}", path.display()));
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    fs::write(&path, normalized).unwrap_or_else(|error| {
        panic!("failed to normalize generated {}: {error}", path.display())
    });
}
