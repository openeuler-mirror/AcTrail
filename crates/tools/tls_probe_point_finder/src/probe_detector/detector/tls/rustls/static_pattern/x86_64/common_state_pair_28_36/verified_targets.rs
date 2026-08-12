use crate::probe_detector::contract::candidate::verification::VerifiedTarget;
use crate::{BinaryIdentity, BinaryIdentityTypeCode};

pub(super) fn verified_targets() -> Vec<VerifiedTarget> {
    vec![VerifiedTarget {
        runtime_version: "Rustls 0.23.38 verified xiaoO 0.1.3 RPM",
        compiler_shape: "x86_64 rustc 1.90.0 unoptimized RPM build",
        identity: Some(
            BinaryIdentity::try_new(
                BinaryIdentityTypeCode::GnuBuildId,
                "098a4668a133bde6b34cfe4f992d5bb28e51571c",
            )
            .expect("verified xiaoO identity"),
        ),
        evidence_source: "probe_xiaoo_llm real-agent E2E",
    }]
}
