//! Read-only content routing and bounded HTTP body pages.

pub enum ActionContentSource {
    Llm { action_id: String },
    Http,
}

pub struct HttpContentQuery {
    pub offset: usize,
    pub max_bytes: usize,
    pub read_budget: usize,
}

pub struct HttpContentPage {
    pub kind: &'static str,
    pub text: String,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub total_bytes: usize,
}
