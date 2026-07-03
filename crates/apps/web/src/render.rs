//! Static web assets.

pub struct StaticAsset {
    pub content_type: &'static str,
    pub body: &'static [u8],
}

struct EmbeddedAsset {
    path: &'static str,
    body: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/actrailweb-assets.rs"));

pub fn asset(request_path: &str) -> Option<StaticAsset> {
    let path = match request_path {
        "/" => "/index.html",
        path if path.starts_with("/assets/") => path,
        _ => return None,
    };
    EMBEDDED_ASSETS
        .iter()
        .find(|asset| asset.path == path)
        .map(|asset| StaticAsset {
            content_type: content_type(path),
            body: asset.body,
        })
}

fn content_type(path: &str) -> &'static str {
    if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".js") {
        "application/javascript; charset=utf-8"
    } else if path.ends_with(".json") || path.ends_with(".map") {
        "application/json; charset=utf-8"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else if path.ends_with(".webp") {
        "image/webp"
    } else if path.ends_with(".ico") {
        "image/x-icon"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else {
        "application/octet-stream"
    }
}
