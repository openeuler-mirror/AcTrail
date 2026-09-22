use crate::RecordingError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryFailureKind {
    Storage,
    Consumer,
    Runtime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryFailure {
    pub kind: DeliveryFailureKind,
    pub error: RecordingError,
}

/// Independent output failures; an empty report means every attempted delivery succeeded.
#[derive(Default, Debug)]
#[must_use]
pub struct DeliveryReport {
    failures: Vec<DeliveryFailure>,
}

impl DeliveryReport {
    pub fn storage_succeeded(&self) -> bool {
        !self
            .failures
            .iter()
            .any(|failure| failure.kind == DeliveryFailureKind::Storage)
    }

    pub fn into_failures(self) -> Vec<DeliveryFailure> {
        self.failures
    }

    pub(crate) fn record(&mut self, kind: DeliveryFailureKind, result: Result<(), RecordingError>) {
        if let Err(error) = result {
            self.failures.push(DeliveryFailure { kind, error });
        }
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.failures.extend(other.failures);
    }
}
