//! Static web assets.

pub fn html() -> String {
    include_str!(concat!(env!("OUT_DIR"), "/actrailweb-dist/index.html")).to_string()
}

pub fn css() -> String {
    include_str!(concat!(env!("OUT_DIR"), "/actrailweb-dist/assets/app.css")).to_string()
}

pub fn javascript() -> String {
    include_str!(concat!(env!("OUT_DIR"), "/actrailweb-dist/assets/app.js")).to_string()
}
