//! Identity verification contracts for raw events and membership matching.

use model_core::process::ProcessIdentity;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VerificationOutcome {
    Match,
    Mismatch {
        expected: ProcessIdentity,
        observed: ProcessIdentity,
    },
}

pub trait IdentityVerifier {
    fn verify(&self, expected: &ProcessIdentity, observed: &ProcessIdentity)
    -> VerificationOutcome;
}
