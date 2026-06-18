const FNV_OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;
const FNV_PRIME: u64 = 1_099_511_628_211;

pub(super) fn stable_hash_text(value: &str) -> String {
    stable_hash_bytes(value.as_bytes())
}

pub(super) fn stable_hash_bytes(bytes: &[u8]) -> String {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

pub(super) fn encode_path_ids(path_ids: &[u64]) -> String {
    path_ids
        .iter()
        .map(|path_id| path_id.to_string())
        .collect::<Vec<_>>()
        .join(",")
}
