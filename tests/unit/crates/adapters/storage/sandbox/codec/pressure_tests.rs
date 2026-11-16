// Tests for the corresponding production module.
use super::*;

fn pressure() -> GuestPressureSnapshot {
    GuestPressureSnapshot {
        guest_boot_id: GuestBootId::new([3; 16]),
        sampled_at_ms: 9_876,
        memory_some: PsiAverages {
            avg10_millipercent: 30_000,
            avg60_millipercent: 15_500,
            avg300_millipercent: 1_000,
        },
        memory_full: PsiAverages {
            avg10_millipercent: 0,
            avg60_millipercent: 123_456,
            avg300_millipercent: 7,
        },
    }
}

#[test]
fn pressure_observation_round_trips() {
    let observation = Observation::GuestPressure(pressure());
    let (kind, bytes) = ObservationCodec::encode(&observation);
    assert_eq!(kind, GUEST_PRESSURE_KIND);
    assert_eq!(bytes.len(), 48);
    let decoded = ObservationCodec::decode(kind, &bytes).expect("decode pressure");
    assert_eq!(decoded, observation);
}

#[test]
fn pressure_decode_rejects_trailing_bytes() {
    let observation = Observation::GuestPressure(pressure());
    let (kind, mut bytes) = ObservationCodec::encode(&observation);
    bytes.push(0);
    assert!(ObservationCodec::decode(kind, &bytes).is_err());
}
