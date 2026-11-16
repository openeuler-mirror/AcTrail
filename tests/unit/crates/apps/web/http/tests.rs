// Tests for the corresponding production module.
use super::client_disconnected;

#[test]
fn recognizes_expected_client_disconnects() {
    for kind in [
        std::io::ErrorKind::BrokenPipe,
        std::io::ErrorKind::ConnectionReset,
        std::io::ErrorKind::ConnectionAborted,
    ] {
        assert!(client_disconnected(&std::io::Error::from(kind)));
    }
    assert!(!client_disconnected(&std::io::Error::from(
        std::io::ErrorKind::WriteZero,
    )));
}
