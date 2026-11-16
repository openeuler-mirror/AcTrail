// Tests for the corresponding production module.
use super::*;

fn pressure() -> GuestPressureSnapshot {
    GuestPressureSnapshot {
        guest_boot_id: GuestBootId::new([9; 16]),
        sampled_at_ms: 1_234,
        memory_some: PsiAverages {
            avg10_millipercent: 2_350,
            avg60_millipercent: 1_100,
            avg300_millipercent: 500,
        },
        memory_full: PsiAverages {
            avg10_millipercent: 100_000,
            avg60_millipercent: 0,
            avg300_millipercent: 42_000,
        },
    }
}

#[test]
fn pressure_observation_round_trips() {
    let codec = ObservationBatchCodec;
    let batch = ObservationBatch::new(7, vec![Observation::GuestPressure(pressure())]);
    let encoded = codec.encode(&batch).expect("encode pressure");
    assert_eq!(encoded.len(), 10 + 3 + PRESSURE_BYTES);
    assert_eq!(encoded[10], PRESSURE_CODE);
    assert_eq!(codec.decode(&encoded).expect("decode pressure"), batch);
}

#[test]
fn pressure_rejects_wrong_body_length() {
    let codec = ObservationBatchCodec;
    let mut encoded = codec
        .encode(&ObservationBatch::new(
            1,
            vec![Observation::GuestPressure(pressure())],
        ))
        .expect("encode pressure");
    // Corrupt the body length so it no longer matches PRESSURE_BYTES.
    encoded[11] = 0;
    encoded[12] = (PRESSURE_BYTES + 1) as u8;
    assert!(codec.decode(&encoded).is_err());
}
