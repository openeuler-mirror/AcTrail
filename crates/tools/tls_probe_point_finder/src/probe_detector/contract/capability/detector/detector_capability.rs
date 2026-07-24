use crate::probe_detector::contract::capability::ConsumerCapability;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DetectorCapability {
    pub(crate) complete_plaintext_closure: bool,
    pub(crate) consumer: ConsumerCapability,
}

impl DetectorCapability {
    pub(crate) fn executable_by_consumer(&self) -> bool {
        self.complete_plaintext_closure && self.consumer.supported
    }
}
