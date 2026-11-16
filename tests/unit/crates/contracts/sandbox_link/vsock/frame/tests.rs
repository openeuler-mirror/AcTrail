// Tests for the corresponding production module.
use super::*;

#[test]
fn welcome_round_trips_id_and_workload_capability_in_v1_payload() {
    let frame = Frame::sb_welcome(17, true).unwrap();
    assert_eq!(frame.payload.len(), 4);
    assert_eq!(
        frame.decode_sb_welcome().unwrap(),
        SbWelcome {
            sb_id: 17 | SB_WELCOME_WORKLOAD_CGROUP,
            workload_cgroup_observations: true,
        }
    );
    assert_ne!(frame.decode_numeric_id().unwrap(), 0);
}

#[test]
fn legacy_welcome_decodes_without_new_capabilities() {
    let frame = Frame::numeric_id(FrameCode::SbWelcome, 23);
    assert_eq!(
        frame.decode_sb_welcome().unwrap(),
        SbWelcome {
            sb_id: 23,
            workload_cgroup_observations: false,
        }
    );
}
