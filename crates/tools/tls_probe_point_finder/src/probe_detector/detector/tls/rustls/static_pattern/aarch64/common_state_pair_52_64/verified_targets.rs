use crate::probe_detector::contract::candidate::verification::VerifiedTarget;
use crate::{BinaryIdentity, BinaryIdentityTypeCode};

pub(super) fn verified_targets() -> Vec<VerifiedTarget> {
    vec![
        VerifiedTarget {
            runtime_version: "Rustls 0.23.40 verified build",
            compiler_shape: "aarch64 release build",
            identity: None,
            evidence_source: "pre-migration finder corpus",
        },
        VerifiedTarget {
            runtime_version: "Codex 0.145.0 embedded Rustls build",
            compiler_shape: "aarch64 musl release build",
            identity: Some(
                BinaryIdentity::try_new(
                    BinaryIdentityTypeCode::ElfExecutableSampleSha256V1,
                    "a69190690f24de5ba74c7ecac61c18f4737b3e3ac8e4e34d610f528833b1fe54",
                )
                .expect("verified Codex identity"),
            ),
            evidence_source: "codex-websocket real-agent E2E",
        },
    ]
}
