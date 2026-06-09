//! Static web assets.

pub fn html() -> String {
    include_str!("render/dist/index.html").to_string()
}

pub fn css() -> String {
    include_str!("render/dist/assets/app.css").to_string()
}

pub fn javascript() -> String {
    include_str!("render/dist/assets/app.js").to_string()
}
