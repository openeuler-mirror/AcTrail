use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const PREBUILT_ASSETS_ENV: &str = "ACTRAILWEB_PREBUILT_ASSETS_DIR";
const DIST_DIR_NAME: &str = "actrailweb-dist";
const FRONTEND_DIR: &str = "frontend";
const FRONTEND_SOURCE_DIR: &str = "src";
const FRONTEND_ENTRY_FILES: &[&str] = &[
    "index.html",
    "package.json",
    "package-lock.json",
    "vite.config.js",
];
const DIST_ASSETS: &[&str] = &["index.html", "assets/app.css", "assets/app.js"];

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let dist_dir = out_dir.join(DIST_DIR_NAME);

    println!("cargo:rerun-if-env-changed={PREBUILT_ASSETS_ENV}");

    match env::var_os(PREBUILT_ASSETS_ENV) {
        Some(path) => copy_prebuilt_assets(PathBuf::from(path), &dist_dir),
        None => build_frontend(&manifest_dir.join(FRONTEND_DIR), &dist_dir),
    }

    for relative_path in DIST_ASSETS {
        let asset = dist_dir.join(relative_path);
        if !asset.is_file() {
            panic!(
                "missing actrailweb build asset {}; expected npm build or {PREBUILT_ASSETS_ENV} to provide it",
                asset.display()
            );
        }
    }
}

fn copy_prebuilt_assets(source_dir: PathBuf, dist_dir: &Path) {
    if !source_dir.is_absolute() {
        panic!("{PREBUILT_ASSETS_ENV} must be an absolute path");
    }
    println!(
        "cargo:warning=using prebuilt actrailweb assets from {}",
        source_dir.display()
    );
    for relative_path in DIST_ASSETS {
        let source = source_dir.join(relative_path);
        println!("cargo:rerun-if-changed={}", source.display());
        if !source.is_file() {
            panic!(
                "missing prebuilt actrailweb asset {}; regenerate the frontend dist before cargo build",
                source.display()
            );
        }
        let target = dist_dir.join(relative_path);
        let parent = target
            .parent()
            .expect("actrailweb asset target has a parent directory");
        fs::create_dir_all(parent).unwrap_or_else(|error| {
            panic!(
                "create actrailweb asset directory {}: {error}",
                parent.display()
            )
        });
        fs::copy(&source, &target).unwrap_or_else(|error| {
            panic!(
                "copy actrailweb asset {} to {}: {error}",
                source.display(),
                target.display()
            )
        });
    }
}

fn build_frontend(frontend_dir: &Path, dist_dir: &Path) {
    for entry_file in FRONTEND_ENTRY_FILES {
        let path = frontend_dir.join(entry_file);
        println!("cargo:rerun-if-changed={}", path.display());
    }
    emit_source_rerun_paths(&frontend_dir.join(FRONTEND_SOURCE_DIR));

    let status = Command::new("npm")
        .current_dir(frontend_dir)
        .args(["run", "build", "--", "--outDir"])
        .arg(dist_dir)
        .status()
        .unwrap_or_else(|error| {
            panic!(
                "run actrailweb frontend build in {}: {error}; run npm ci --prefix crates/apps/web/frontend first",
                frontend_dir.display()
            )
        });

    if !status.success() {
        panic!(
            "actrailweb frontend build failed with status {status}; run npm ci --prefix crates/apps/web/frontend first"
        );
    }
}

fn emit_source_rerun_paths(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
    if path.is_dir() {
        let entries = fs::read_dir(path).unwrap_or_else(|error| {
            panic!(
                "read actrailweb frontend source {}: {error}",
                path.display()
            )
        });
        for entry in entries {
            let entry = entry.unwrap_or_else(|error| {
                panic!(
                    "read actrailweb frontend source entry under {}: {error}",
                    path.display()
                )
            });
            emit_source_rerun_paths(&entry.path());
        }
    }
}
