pub(crate) struct ProxyDiagnostics;

impl ProxyDiagnostics {
    pub(crate) fn startup_failed(error: &dyn std::fmt::Display) {
        eprintln!("actraild-alert-proxy startup failed: {error}");
    }

    pub(crate) fn ready(daemon_socket: &std::path::Path, subscriber: std::net::SocketAddr) {
        println!(
            "actraild-alert-proxy ready daemon_socket={} subscriber={subscriber}",
            daemon_socket.display()
        );
    }

    pub(crate) fn connection_failed(scope: &'static str, error: &dyn std::fmt::Display) {
        eprintln!("actraild-alert-proxy {scope} connection failed: {error}");
    }

    pub(crate) fn runtime_failed(scope: &'static str, error: &dyn std::fmt::Display) {
        eprintln!("actraild-alert-proxy {scope} runtime failed: {error}");
    }
}
