//! WebSocket negotiation from an already decoded HTTP message head.
use crate::Http1MessageHead;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha1::{Digest, Sha1};

const WEBSOCKET_ACCEPT_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

pub(super) struct WebSocketHandshake;
impl WebSocketHandshake {
    pub(super) fn request(head: &Http1MessageHead) -> Option<(String, String)> {
        if head.method.as_deref() != Some("GET")
            || !Self::has_upgrade_headers(head.headers_text.lines())
        {
            return None;
        }
        let key = Self::header_value(head.headers_text.lines(), "sec-websocket-key")?;
        let digest = Sha1::digest(format!("{key}{WEBSOCKET_ACCEPT_GUID}").as_bytes());
        Some((head.path.clone()?, STANDARD.encode(digest)))
    }
    pub(super) fn accepted(head: &Http1MessageHead) -> Option<(String, NegotiatedExtensions)> {
        if head.status_code.as_deref() != Some("101")
            || !Self::has_upgrade_headers(head.headers_text.lines())
        {
            return None;
        }
        let value =
            Self::header_value(head.headers_text.lines(), "sec-websocket-accept")?.to_string();
        let lower = Self::header_value(head.headers_text.lines(), "sec-websocket-extensions")
            .unwrap_or_default()
            .to_ascii_lowercase();
        Some((
            value,
            NegotiatedExtensions {
                permessage_deflate: lower
                    .split(',')
                    .any(|value| value.trim().starts_with("permessage-deflate")),
                client_no_context_takeover: lower.contains("client_no_context_takeover"),
                server_no_context_takeover: lower.contains("server_no_context_takeover"),
            },
        ))
    }
    fn has_upgrade_headers<'a>(lines: impl Iterator<Item = &'a str>) -> bool {
        let mut upgrade = false;
        let mut connection = false;
        for line in lines {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            upgrade |= name.trim().eq_ignore_ascii_case("upgrade")
                && value.trim().eq_ignore_ascii_case("websocket");
            connection |= name.trim().eq_ignore_ascii_case("connection")
                && value
                    .split(',')
                    .any(|token| token.trim().eq_ignore_ascii_case("upgrade"));
        }
        upgrade && connection
    }

    fn header_value<'a>(lines: impl Iterator<Item = &'a str>, expected: &str) -> Option<&'a str> {
        lines
            .filter_map(|line| line.split_once(':'))
            .find_map(|(name, value)| {
                name.trim()
                    .eq_ignore_ascii_case(expected)
                    .then_some(value.trim())
            })
    }
}
#[derive(Clone)]
pub(super) struct NegotiatedExtensions {
    pub(super) permessage_deflate: bool,
    pub(super) client_no_context_takeover: bool,
    pub(super) server_no_context_takeover: bool,
}
