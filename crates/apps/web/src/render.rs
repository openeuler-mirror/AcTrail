//! Static web assets.

pub fn html() -> String {
    include_str!("render/index.html").to_string()
}

pub fn css() -> String {
    include_str!("render/app.css").to_string()
}

pub fn javascript() -> String {
    include_str!("render/app.js").to_string()
}
