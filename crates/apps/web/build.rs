use std::path::PathBuf;

const DIST_INDEX: &str = "src/render/dist/index.html";
const DIST_CSS: &str = "src/render/dist/assets/app.css";
const DIST_JS: &str = "src/render/dist/assets/app.js";

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for path in [DIST_INDEX, DIST_CSS, DIST_JS] {
        let asset = manifest_dir.join(path);
        println!("cargo:rerun-if-changed={}", asset.display());
        if !asset.is_file() {
            panic!(
                "missing checked-in actrailweb asset {}; run npm build in crates/apps/web/frontend and commit the generated dist",
                asset.display()
            );
        }
    }
}
