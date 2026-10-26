use crate::probe_detector::contract::candidate::verification::VerifiedTarget;
use crate::{BinaryIdentity, BinaryIdentityTypeCode};

pub(super) fn verified_targets() -> Vec<VerifiedTarget> {
    vec![VerifiedTarget {
        runtime_version: "Codex 0.155.1 embedded Rustls build",
        compiler_shape: "aarch64 musl release build, x22 tag temporary",
        identity: Some(
            BinaryIdentity::try_new(
                BinaryIdentityTypeCode::ElfExecutableSampleSha256V1,
                "f57e62af5e754861d923a6a7cda31fb407a135b7d4bfd94eeb89137bfdb7c83c",
            )
            .expect("verified Codex identity"),
        ),
        evidence_source: "Codex 0.155.1 real-agent LLM capture",
    }]
}
