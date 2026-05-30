//! Rule-shape boundaries and matching skeletons.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderRule {
    pub field: String,
    pub equals: String,
    pub provider: String,
    pub confidence_millis: u16,
    pub rationale: Option<String>,
}
