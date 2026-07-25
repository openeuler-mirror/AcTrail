use crate::probe_detector::contract::candidate::verification::VerifiedTarget;

pub(super) fn verified_targets() -> Vec<VerifiedTarget> {
    vec![VerifiedTarget {
        runtime_version: "legacy Rustls codegen family",
        compiler_shape: "x86_64 release build",
        identity: None,
        evidence_source: "pre-migration finder corpus",
    }]
}
