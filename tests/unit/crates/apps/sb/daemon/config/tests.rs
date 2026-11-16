// Tests for the corresponding production module.
use super::*;

#[test]
fn checked_in_daemon_configs_match_the_runtime_contract() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    for relative in [
        "deploy/execution-isolation/actrail-sb.toml",
        "deploy/virtual-container/guest/sandbox-observer.toml",
    ] {
        let path = repo.join(relative);
        let config = SbDaemonConfig::load(&path)
            .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));
        config
            .validate()
            .unwrap_or_else(|error| panic!("validate {}: {error}", path.display()));
    }
}
