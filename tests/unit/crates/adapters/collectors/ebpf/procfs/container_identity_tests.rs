// Tests for the corresponding production module.
use super::*;

const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn parses_all_supported_systemd_container_scopes() {
    for (scope, expected) in [
        (format!("docker-{ID}.scope"), ContainerRuntime::Docker),
        (
            format!("cri-containerd-{ID}.scope"),
            ContainerRuntime::Containerd,
        ),
        (format!("crio-{ID}.scope"), ContainerRuntime::Crio),
        (format!("libpod-{ID}.scope"), ContainerRuntime::Podman),
    ] {
        let identity = parse_container_identity(&format!("0::/system.slice/{scope}\n"))
            .expect("supported runtime scope should resolve");
        assert_eq!(identity.runtime, expected);
        assert_eq!(identity.container_id(), ID);
    }
}
