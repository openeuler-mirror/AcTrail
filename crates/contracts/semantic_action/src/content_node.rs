//! Request-content navigation with bounded block decoding and paged string values.

#[derive(Clone, Debug)]
pub struct ContentNodeQuery {
    pub pointer: String,
    /// Child index for containers; decoded UTF-8 byte offset for strings.
    pub offset: usize,
    pub limit: usize,
    pub preview_bytes: usize,
    /// Unique canonical block bytes read within this request, including type descriptions.
    pub block_read_budget: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentNodeKind {
    Null,
    Boolean,
    Number,
    String,
    Array,
    Object,
}

impl ContentNodeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean => "boolean",
            Self::Number => "number",
            Self::String => "string",
            Self::Array => "array",
            Self::Object => "object",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ContentNodeChild {
    pub token: String,
    pub pointer: String,
    pub kind: ContentNodeKind,
    /// Known for inline containers; block-backed containers are counted when opened.
    pub child_count: Option<usize>,
}

#[derive(Clone, Debug)]
pub enum ContentNode {
    Branch {
        kind: ContentNodeKind,
        total_children: usize,
        offset: usize,
        limit: usize,
        next_offset: Option<usize>,
        children: Vec<ContentNodeChild>,
    },
    Leaf {
        value: serde_json::Value,
        /// Strings count decoded UTF-8 bytes; other scalars count canonical JSON bytes.
        value_bytes: u64,
        returned_bytes: u64,
        /// Start and continuation positions in decoded UTF-8 bytes for strings.
        offset: usize,
        next_offset: Option<usize>,
        /// This page contains only part of the value, including noninitial pages.
        truncated: bool,
    },
}

#[derive(Clone, Debug)]
pub struct ContentNodePage {
    pub action_id: String,
    pub format_version: u32,
    pub canonical_body_bytes: u64,
    pub pointer: String,
    pub node: ContentNode,
}
