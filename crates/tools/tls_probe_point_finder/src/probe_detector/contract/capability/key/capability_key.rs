use crate::plan::{ProbeSource, TlsProvider};
use crate::probe_detector::contract::detection::ProbeConsumer;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CapabilityKey {
    pub(crate) architecture: String,
    pub(crate) provider: TlsProvider,
    pub(crate) source: ProbeSource,
    pub(crate) resolver: String,
    pub(crate) consumer: ProbeConsumer,
}
