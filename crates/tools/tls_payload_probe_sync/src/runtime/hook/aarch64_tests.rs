use super::aarch64_relocate::{adr_target, bl_target, relocation_error};
use super::*;

#[test]
fn jump_patch_loads_absolute_target_and_branches_via_x16() {
    let mut bytes = [0_u8; 16];
    let target = 0x1122_3344_5566_7788usize;

    unsafe {
        write_jump(bytes.as_mut_ptr() as usize, target);
    }

    assert_eq!(
        u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
        LDR_X16_LITERAL_8
    );
    assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), BR_X16);
    assert_eq!(
        u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        target as u64
    );
}

#[test]
fn load_immediate_patch_loads_literal_and_skips_over_data() {
    let mut bytes = [0_u8; 16];
    let target = 0x8877_6655_4433_2211usize;

    unsafe {
        write_load_immediate(bytes.as_mut_ptr() as usize, 3, target).unwrap();
    }

    assert_eq!(
        u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
        ldr_x_literal_8(3)
    );
    assert_eq!(
        u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
        B_SKIP_LITERAL
    );
    assert_eq!(
        u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        target as u64
    );
}

#[test]
fn absolute_call_patch_returns_to_code_after_literal() {
    let mut bytes = [0_u8; 20];
    let target = 0x1122_3344_5566_7788usize;

    unsafe {
        write_absolute_call(bytes.as_mut_ptr() as usize, target);
    }

    assert_eq!(
        u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
        LDR_X16_LITERAL_12
    );
    assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), BLR_X16);
    assert_eq!(
        u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        B_SKIP_LITERAL
    );
    assert_eq!(
        u64::from_le_bytes(bytes[12..20].try_into().unwrap()),
        target as u64
    );
}

#[test]
fn relocation_decodes_openssl_adrp_and_bl_targets() {
    assert_eq!(adr_target(0x364e4, 0xb000_0343), Some(0x9f000));
    assert_eq!(bl_target(0x36578, 0x97ff_ff66), Some(0x36310));
}

#[test]
fn relocation_filter_rejects_unsupported_register_branches_and_literal_loads() {
    for instruction in [
        0xd61f_0000, // br x0
        0x5800_0000, // ldr literal
    ] {
        assert!(
            relocation_error(instruction).is_some(),
            "instruction 0x{instruction:08x} should be rejected"
        );
    }
}

#[test]
fn relocation_filter_allows_common_frame_setup_instructions() {
    for instruction in [
        0xa9bf_7bfd, // stp x29, x30, [sp, #-16]!
        0x9100_03fd, // mov x29, sp
        0x1400_0000, // b
        0x5400_0000, // b.cond
        0x3400_0000, // cbz
        0x3700_0000, // tbnz
        0x9400_0000, // bl
        0x9000_0000, // adrp
        0xf81f_0ff3, // str x19, [sp, #-16]!
        0xaa00_03f3, // mov x19, x0
    ] {
        assert!(
            relocation_error(instruction).is_none(),
            "instruction 0x{instruction:08x} should be allowed"
        );
    }
}
