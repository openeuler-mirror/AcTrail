use std::collections::BTreeMap;

use crate::providers::boringssl::{DetectedOffset, StaticPatternDetection};

use super::{boringssl_probe_symbols, boringssl_static_probe_offsets};

#[test]
fn boringssl_static_plan_excludes_validation_anchors() {
    let detection = StaticPatternDetection {
        arch_label: "aarch64",
        matches: Vec::new(),
        offsets: vec![
            DetectedOffset {
                symbol: "SSL_read",
                file_offset: 0x100,
                virtual_address: 0x1000,
            },
            DetectedOffset {
                symbol: "SSL_read_internal",
                file_offset: 0x200,
                virtual_address: 0x2000,
            },
            DetectedOffset {
                symbol: "SSL_do_handshake",
                file_offset: 0x300,
                virtual_address: 0x3000,
            },
            DetectedOffset {
                symbol: "SSL_write",
                file_offset: 0x400,
                virtual_address: 0x4000,
            },
        ],
        map_symbols: BTreeMap::new(),
    };

    let symbols = boringssl_static_probe_offsets(&detection)
        .into_iter()
        .map(|(symbol, _, _)| symbol)
        .collect::<Vec<_>>();

    assert_eq!(symbols, vec!["SSL_read", "SSL_write"]);
}

#[test]
fn boringssl_symbol_map_plan_excludes_validation_anchors() {
    let symbols = BTreeMap::from([
        ("SSL_do_handshake".to_string(), 0x1000),
        ("SSL_read".to_string(), 0x2000),
        ("SSL_write".to_string(), 0x3000),
    ]);

    let probe_symbols = boringssl_probe_symbols(&symbols)
        .into_keys()
        .collect::<Vec<_>>();

    assert_eq!(probe_symbols, vec!["SSL_read", "SSL_write"]);
}
