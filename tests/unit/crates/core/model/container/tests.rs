// Tests for the corresponding production module.
use super::*;

#[test]
fn normalized_container_id_requires_exact_lowercase_hex_and_round_trips() {
    let raw = "0123456789abcdef".repeat(4);
    let id = NormalizedContainerId::from_lower_hex(&raw).unwrap();
    assert_eq!(id.to_string(), raw);
    assert_eq!(NormalizedContainerId::from_bytes(*id.as_bytes()), id);
    assert!(NormalizedContainerId::from_lower_hex(&raw[..63]).is_err());
    assert!(NormalizedContainerId::from_lower_hex(&raw.to_uppercase()).is_err());
    assert!(NormalizedContainerId::from_lower_hex(&format!("{}g", &raw[..63])).is_err());
}

#[test]
fn persisted_runtime_names_round_trip() {
    for runtime in [
        ContainerRuntime::Docker,
        ContainerRuntime::Containerd,
        ContainerRuntime::K8s,
        ContainerRuntime::Podman,
        ContainerRuntime::Crio,
        ContainerRuntime::Unknown,
    ] {
        assert_eq!(
            ContainerRuntime::from_storage_str(runtime.as_str()),
            Some(runtime)
        );
    }
}
