// Tests for the corresponding production module.
use super::*;

#[test]
fn legacy_generated_schema_setting_normalizes_and_round_trips_as_current() {
    for version in [2, 3] {
        let document: SandboxEvidenceDocument =
            toml::from_str(&format!("schema_version = {version}\n")).unwrap();
        let config = document.to_config().unwrap();
        assert_eq!(config.schema_version, 3);
        assert_eq!(
            SandboxEvidenceDocument::from_config(&config).schema_version,
            3
        );
    }
    for version in [0, 1, 4] {
        let document = SandboxEvidenceDocument {
            schema_version: version,
            ..Default::default()
        };
        assert!(document.to_config().is_err());
    }
}

#[test]
fn full_generated_operator_config_accepts_previous_explicit_schema() {
    use super::super::OperatorDocument;
    let generated = OperatorDocument::default_toml().unwrap();
    assert!(generated.contains("schema_version = 3"));
    let legacy = generated.replace("schema_version = 3", "schema_version = 2");
    let config = OperatorDocument::parse(&legacy)
        .unwrap()
        .to_config()
        .unwrap();
    assert_eq!(config.sandbox_evidence.schema_version, 3);
}
