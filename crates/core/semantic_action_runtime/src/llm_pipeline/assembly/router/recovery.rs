//! Incremental trusted-boundary recovery after a localized stream reset.

use crate::llm_pipeline::assembly::router::LiveStreamDirection;

const LINE_END: &[u8] = b"\r\n";
const REQUEST_METHODS: [&[u8]; 9] = [
    b"GET", b"POST", b"PUT", b"PATCH", b"DELETE", b"HEAD", b"OPTIONS", b"CONNECT", b"TRACE",
];

pub(super) enum RecoveryBoundary {
    NeedMore,
    Found(usize),
}

/// Scans every byte at most a constant number of times inside the configured
/// assembly window. The caller releases the damaged prefix after a boundary
/// is confirmed, so split request/response lines remain recoverable.
#[derive(Default)]
pub(super) struct RecoveryScanner {
    line_start: usize,
    line_scan: usize,
}

impl RecoveryScanner {
    pub(super) fn inspect(
        &mut self,
        direction: LiveStreamDirection,
        bytes: &[u8],
    ) -> RecoveryBoundary {
        loop {
            let search_start = self.line_scan.saturating_sub(1).max(self.line_start);
            let Some(relative_end) = find_bytes(&bytes[search_start..], LINE_END) else {
                self.line_scan = bytes.len();
                return RecoveryBoundary::NeedMore;
            };
            let line_end = search_start + relative_end;
            let boundary = match direction {
                LiveStreamDirection::Outbound => {
                    request_start_in_line(&bytes[self.line_start..line_end])
                }
                LiveStreamDirection::Inbound => {
                    response_start_in_line(&bytes[self.line_start..line_end])
                }
            };
            if let Some(relative_start) = boundary {
                return RecoveryBoundary::Found(self.line_start + relative_start);
            }
            self.line_start = line_end + LINE_END.len();
            self.line_scan = self.line_start;
        }
    }

    pub(super) fn reset(&mut self) {
        self.line_start = 0;
        self.line_scan = 0;
    }
}

fn request_start_in_line(line: &[u8]) -> Option<usize> {
    let mut tokens = TokenSpans::new(line);
    let mut first = tokens.next()?;
    let mut second = tokens.next()?;
    while let Some(third) = tokens.next() {
        if let Some(method_start) = request_method_suffix(first.bytes)
            && third.bytes.starts_with(b"HTTP/")
        {
            return Some(first.start + method_start);
        }
        first = second;
        second = third;
    }
    None
}

fn response_start_in_line(line: &[u8]) -> Option<usize> {
    let mut tokens = TokenSpans::new(line);
    let mut version = tokens.next()?;
    for status in tokens {
        if status.bytes.len() == 3
            && status.bytes.iter().all(u8::is_ascii_digit)
            && let Some(version_start) = find_bytes(version.bytes, b"HTTP/")
        {
            return Some(version.start + version_start);
        }
        version = status;
    }
    None
}

fn request_method_suffix(token: &[u8]) -> Option<usize> {
    REQUEST_METHODS.iter().find_map(|method| {
        token
            .ends_with(method)
            .then_some(token.len().saturating_sub(method.len()))
    })
}

struct Token<'a> {
    start: usize,
    bytes: &'a [u8],
}

struct TokenSpans<'a> {
    line: &'a [u8],
    cursor: usize,
}

impl<'a> TokenSpans<'a> {
    fn new(line: &'a [u8]) -> Self {
        Self { line, cursor: 0 }
    }
}

impl<'a> Iterator for TokenSpans<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        while self
            .line
            .get(self.cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.cursor += 1;
        }
        let start = self.cursor;
        while self
            .line
            .get(self.cursor)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
        {
            self.cursor += 1;
        }
        (start < self.cursor).then(|| Token {
            start,
            bytes: &self.line[start..self.cursor],
        })
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
