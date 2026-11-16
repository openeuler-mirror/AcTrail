use crate::probe_detector::contract::candidate::verification::VerifiedTarget;
use crate::{BinaryIdentity, BinaryIdentityTypeCode};

pub(super) fn verified_targets() -> Vec<VerifiedTarget> {
    vec![VerifiedTarget {
        runtime_version: "Rustls 0.23.43 verified xiaoO build",
        compiler_shape: "x86_64 unoptimized ELF, 72-byte outbound stack frame",
        identity: Some(
            BinaryIdentity::try_new(
                BinaryIdentityTypeCode::GnuBuildId,
                "b4d3610f17f2e51ade209618330d2bbd4eab068d",
            )
            .expect("verified xiaoO identity"),
        ),
        evidence_source: "Rustls 0.23.43 source and verified ELF function bodies",
    }]
}
