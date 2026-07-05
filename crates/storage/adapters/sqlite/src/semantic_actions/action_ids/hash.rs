use sha2::{Digest, Sha256};

pub(super) fn sha256_hash_blob(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}
