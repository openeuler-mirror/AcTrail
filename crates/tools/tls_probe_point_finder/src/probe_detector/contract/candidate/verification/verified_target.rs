use crate::BinaryIdentity;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedTarget {
    pub(crate) runtime_version: &'static str,
    pub(crate) compiler_shape: &'static str,
    pub(crate) identity: Option<BinaryIdentity>,
    pub(crate) evidence_source: &'static str,
}
