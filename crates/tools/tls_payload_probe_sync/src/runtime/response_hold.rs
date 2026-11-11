//! Holds the responses of checkpoint launches until the daemon releases what the process wrote
//! before them on the same TLS connection.

use crate::runtime::config;

/// Called before response bytes read from a TLS connection go back to the Agent.
pub(in crate::runtime) fn before_response(connection: usize) {
    if let Some(config) = config::get() {
        config.wait_for_release(connection);
    }
}
