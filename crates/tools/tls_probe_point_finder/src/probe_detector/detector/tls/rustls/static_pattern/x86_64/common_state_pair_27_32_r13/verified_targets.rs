use crate::probe_detector::contract::candidate::verification::VerifiedTarget;
use crate::{BinaryIdentity, BinaryIdentityTypeCode};

pub(super) fn verified_targets() -> Vec<VerifiedTarget> {
    vec![
        VerifiedTarget {
            runtime_version: "Codex 0.145.0 embedded Rustls build",
            compiler_shape: "x86_64 musl release build",
            identity: Some(
                BinaryIdentity::try_new(
                    BinaryIdentityTypeCode::ElfExecutableSampleSha256V1,
                    "6428ad0cfeb017568967979084ef405987352d7c52e02d25189b5bdd337fa11e",
                )
                .expect("verified Codex 0.145.0 identity"),
            ),
            evidence_source: "local codex-websocket real-agent E2E",
        },
        VerifiedTarget {
            runtime_version: "Codex 0.146.0 embedded Rustls build",
            compiler_shape: "x86_64 musl release build",
            identity: Some(
                BinaryIdentity::try_new(
                    BinaryIdentityTypeCode::ElfExecutableSampleSha256V1,
                    "6948d0811ec18dab404ee6949296b85dc192126a6033ab17918b9b61d8bdc168",
                )
                .expect("verified Codex 0.146.0 identity"),
            ),
            evidence_source: "local codex-websocket real-agent E2E",
        },
    ]
}
