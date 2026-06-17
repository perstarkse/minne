use sha2::{Digest, Sha256};

#[derive(Debug)]
pub(super) struct BuildParams {
    pub include_impossible: bool,
    pub base_seed: u64,
    pub rng_seed: u64,
}

#[allow(clippy::indexing_slicing)]
pub(super) fn mix_seed(dataset_id: &str, seed: u64) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(dataset_id.as_bytes());
    hasher.update(seed.to_le_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    u64::from_le_bytes(bytes)
}
