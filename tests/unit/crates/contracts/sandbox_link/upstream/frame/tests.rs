// Tests for the corresponding production module.
use super::*;

#[test]
fn gateway_capability_handshake_is_legacy_compatible() {
    let legacy_hello = Frame::gateway_hello(false);
    assert!(legacy_hello.payload.is_empty());
    assert!(!legacy_hello.decode_gateway_hello().unwrap());
    let legacy_welcome = Frame::gateway_welcome(3, false).unwrap();
    assert_eq!(legacy_welcome.payload.len(), 4);
    assert_eq!(
        legacy_welcome.decode_gateway_welcome().unwrap(),
        GatewayWelcome {
            gateway_id: 3,
            workload_cgroup_observations: false,
        }
    );

    let hello = Frame::gateway_hello(true);
    assert!(hello.decode_gateway_hello().unwrap());
    let welcome = Frame::gateway_welcome(7, true).unwrap();
    assert_eq!(welcome.payload.len(), 8);
    assert_eq!(
        welcome.decode_gateway_welcome().unwrap(),
        GatewayWelcome {
            gateway_id: 7,
            workload_cgroup_observations: true,
        }
    );
}
