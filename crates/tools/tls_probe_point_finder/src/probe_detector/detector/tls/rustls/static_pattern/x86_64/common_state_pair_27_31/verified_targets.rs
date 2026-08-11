use crate::probe_detector::contract::candidate::verification::VerifiedTarget;
use crate::{BinaryIdentity, BinaryIdentityTypeCode};

pub(super) fn verified_targets() -> Vec<VerifiedTarget> {
    vec![VerifiedTarget {
        runtime_version: "Rustls 0.23.38 verified xiaoO build",
        compiler_shape: "x86_64 rustc 1.97.1 release ThinLTO build",
        identity: Some(
            BinaryIdentity::try_new(
                BinaryIdentityTypeCode::GnuBuildId,
                "8a58076bb2e2051b48c9dbeae22c9284d66385cd",
            )
            .expect("verified xiaoO identity"),
        ),
        evidence_source: "probe_xiaoo_llm real-agent E2E",
    }]
}
