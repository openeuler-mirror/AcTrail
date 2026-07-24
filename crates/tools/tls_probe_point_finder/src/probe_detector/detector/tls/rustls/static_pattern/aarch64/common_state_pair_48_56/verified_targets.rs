use crate::probe_detector::contract::candidate::verification::VerifiedTarget;
use crate::{BinaryIdentity, BinaryIdentityTypeCode};

pub(super) fn verified_targets() -> Vec<VerifiedTarget> {
    vec![VerifiedTarget {
        runtime_version: "Rustls 0.23.42 verified xiaoO build",
        compiler_shape: "aarch64 release build",
        identity: Some(
            BinaryIdentity::try_new(
                BinaryIdentityTypeCode::GnuBuildId,
                "70a8bf883ae1546d61b49a79ac4d1eb00a9a095b",
            )
            .expect("verified xiaoO identity"),
        ),
        evidence_source: "xiaoo-rustls real-agent E2E",
    }]
}
