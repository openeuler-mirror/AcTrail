use crate::probe_detector::contract::candidate::verification::VerifiedTarget;

pub(super) fn verified_targets() -> Vec<VerifiedTarget> {
    vec![VerifiedTarget {
        runtime_version: "Grok 1.0.70 embedded Rustls 0.23.37 build",
        compiler_shape: "aarch64 static rustc 1.94.0 release build, 160-byte outbound frame",
        identity: None,
        evidence_source: "Grok 1.0.70 plaintext entry disassembly",
    }]
}
