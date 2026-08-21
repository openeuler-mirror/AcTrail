//! Incremental request-line boundary recovery before HTTP/1 decoder activation.

const LINE_END: &[u8] = b"\r\n";
const REQUEST_METHODS: [&[u8]; 9] = [
    b"GET", b"POST", b"PUT", b"PATCH", b"DELETE", b"HEAD", b"OPTIONS", b"CONNECT", b"TRACE",
];

pub(in crate::llm_pipeline) enum RequestBoundary {
    NeedMore,
    Start,
    Skip(usize),
}

#[derive(Default)]
pub(in crate::llm_pipeline) struct RequestResynchronizer {
    line_start: usize,
    line_scan: usize,
}

impl RequestResynchronizer {
    pub(in crate::llm_pipeline) fn classify(&mut self, bytes: &[u8]) -> RequestBoundary {
        loop {
            let search_start = self.line_scan.saturating_sub(1).max(self.line_start);
            let Some(relative_end) = find_bytes(&bytes[search_start..], LINE_END) else {
                self.line_scan = bytes.len();
                return RequestBoundary::NeedMore;
            };
            let line_end = search_start + relative_end;
            if let Some(relative_start) = request_start_in_line(&bytes[self.line_start..line_end]) {
                let request_start = self.line_start + relative_start;
                return if request_start == 0 {
                    RequestBoundary::Start
                } else {
                    RequestBoundary::Skip(request_start)
                };
            }
            self.line_start = line_end + LINE_END.len();
            self.line_scan = self.line_start;
        }
    }

    pub(in crate::llm_pipeline) fn reset(&mut self) {
        self.line_start = 0;
        self.line_scan = 0;
    }
}

fn request_start_in_line(line: &[u8]) -> Option<usize> {
    (0..line.len()).find(|offset| request_line_is_valid(&line[*offset..]))
}

fn request_line_is_valid(line: &[u8]) -> bool {
    let mut parts = line
        .split(u8::is_ascii_whitespace)
        .filter(|part| !part.is_empty());
    parts
        .next()
        .is_some_and(|method| REQUEST_METHODS.contains(&method))
        && parts.next().is_some()
        && parts
            .next()
            .is_some_and(|version| version.starts_with(b"HTTP/"))
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
