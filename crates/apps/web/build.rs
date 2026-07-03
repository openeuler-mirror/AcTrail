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
const REQUIRED_DIST_ASSETS: &[&str] = &["index.html", "assets/app.css", "assets/app.js"];
const ASSET_TABLE_FILE: &str = "actrailweb-assets.rs";

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let dist_dir = out_dir.join(DIST_DIR_NAME);

    println!("cargo:rerun-if-env-changed={PREBUILT_ASSETS_ENV}");

    match env::var_os(PREBUILT_ASSETS_ENV) {
        Some(path) => copy_prebuilt_assets(PathBuf::from(path), &dist_dir),
        None => build_frontend(&manifest_dir.join(FRONTEND_DIR), &dist_dir),
    }

    for relative_path in REQUIRED_DIST_ASSETS {
        let asset = dist_dir.join(relative_path);
        if !asset.is_file() {
            panic!(
                "missing actrailweb build asset {}; expected npm build or {PREBUILT_ASSETS_ENV} to provide it",
                asset.display()
            );
        }
    }
    write_asset_table(&dist_dir, &out_dir.join(ASSET_TABLE_FILE));
}

fn copy_prebuilt_assets(source_dir: PathBuf, dist_dir: &Path) {
    if !source_dir.is_absolute() {
        panic!("{PREBUILT_ASSETS_ENV} must be an absolute path");
    }
    if !source_dir.is_dir() {
        panic!(
            "{PREBUILT_ASSETS_ENV} must point to a directory, got {}",
            source_dir.display()
        );
    }
    println!(
        "cargo:warning=using prebuilt actrailweb assets from {}",
        source_dir.display()
    );
    emit_source_rerun_paths(&source_dir);
    if dist_dir.exists() {
        fs::remove_dir_all(dist_dir).unwrap_or_else(|error| {
            panic!(
                "remove existing actrailweb asset directory {}: {error}",
                dist_dir.display()
            )
        });
    }
    copy_dir_all(&source_dir, dist_dir);
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

fn copy_dir_all(source: &Path, target: &Path) {
    fs::create_dir_all(target)
        .unwrap_or_else(|error| panic!("create directory {}: {error}", target.display()));
    let entries = fs::read_dir(source)
        .unwrap_or_else(|error| panic!("read directory {}: {error}", source.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!("read directory entry under {}: {error}", source.display())
        });
        let entry_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry_path.is_dir() {
            copy_dir_all(&entry_path, &target_path);
        } else if entry_path.is_file() {
            fs::copy(&entry_path, &target_path).unwrap_or_else(|error| {
                panic!(
                    "copy actrailweb asset {} to {}: {error}",
                    entry_path.display(),
                    target_path.display()
                )
            });
        }
    }
}

fn write_asset_table(dist_dir: &Path, target: &Path) {
    let mut assets = collect_dist_assets(dist_dir);
    assets.sort();

    let mut output = String::from("static EMBEDDED_ASSETS: &[EmbeddedAsset] = &[\n");
    for relative_path in assets {
        let request_path = format!("/{}", relative_path.to_string_lossy().replace('\\', "/"));
        let source_path = dist_dir.join(&relative_path);
        output.push_str(&format!(
            "    EmbeddedAsset {{ path: {request_path:?}, body: include_bytes!({:?}) }},\n",
            source_path.display().to_string()
        ));
    }
    output.push_str("];\n");
    fs::write(target, output).unwrap_or_else(|error| {
        panic!("write actrailweb asset table {}: {error}", target.display())
    });
}

fn collect_dist_assets(dist_dir: &Path) -> Vec<PathBuf> {
    let mut assets = Vec::new();
    collect_dist_assets_inner(dist_dir, dist_dir, &mut assets);
    assets
}

fn collect_dist_assets_inner(root: &Path, path: &Path, assets: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(path)
        .unwrap_or_else(|error| panic!("read dist directory {}: {error}", path.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!(
                "read dist directory entry under {}: {error}",
                path.display()
            )
        });
        let entry_path = entry.path();
        if entry_path.is_dir() {
            collect_dist_assets_inner(root, &entry_path, assets);
        } else if entry_path.is_file() {
            let relative_path = entry_path
                .strip_prefix(root)
                .unwrap_or_else(|error| {
                    panic!(
                        "strip dist root {} from asset {}: {error}",
                        root.display(),
                        entry_path.display()
                    )
                })
                .to_path_buf();
            assets.push(relative_path);
        }
    }
}
