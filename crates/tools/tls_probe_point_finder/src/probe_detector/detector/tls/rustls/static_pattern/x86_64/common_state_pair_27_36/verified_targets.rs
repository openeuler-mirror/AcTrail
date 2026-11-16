use crate::probe_detector::contract::candidate::verification::VerifiedTarget;
use crate::{BinaryIdentity, BinaryIdentityTypeCode};

pub(super) fn verified_targets() -> Vec<VerifiedTarget> {
    vec![VerifiedTarget {
        runtime_version: "Rustls 0.23.43",
        compiler_shape: "x86_64 rustc 1.96.0 LLD 22.1.2 build",
        identity: Some(
            BinaryIdentity::try_new(
                BinaryIdentityTypeCode::GnuBuildId,
                "03ae690d3df583c289dd9c0aabb665b06d8a09d3",
            )
            .expect("verified xiaoO identity"),
        ),
        evidence_source: "static analysis of the local x86_64 xiaoO binary",
    }]
}
