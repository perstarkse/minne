use std::{
    fs::{self, File},
    io::Read,
    path::Path,
};

#[cfg(test)]
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SIDECAR_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChecksumSidecar {
    pub version: u32,
    pub sha256: String,
    pub size_bytes: u64,
    #[serde(default)]
    pub modified_unix_secs: u64,
}

impl ChecksumSidecar {
    #[cfg(test)]
    pub fn sidecar_path(content_path: &Path) -> PathBuf {
        content_path.with_extension("sha256")
    }

    #[cfg(test)]
    pub fn is_valid_for(&self, content_path: &Path) -> bool {
        if self.version != SIDECAR_VERSION {
            return false;
        }
        let Ok(metadata) = fs::metadata(content_path) else {
            return false;
        };
        if metadata.len() != self.size_bytes {
            return false;
        }
        if self.modified_unix_secs != 0 {
            let Ok(modified) = metadata.modified() else {
                return true;
            };
            let Ok(secs) = modified.duration_since(std::time::UNIX_EPOCH) else {
                return true;
            };
            if secs.as_secs() != self.modified_unix_secs {
                return false;
            }
        }
        true
    }
}

#[allow(clippy::indexing_slicing)]
pub fn hash_file(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("opening file {} for checksum", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 65_536];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("reading {} for checksum", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn read_sidecar(path: &Path) -> Result<Option<ChecksumSidecar>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading checksum sidecar {}", path.display()))?;
    let sidecar: ChecksumSidecar = serde_json::from_str(&raw)
        .with_context(|| format!("parsing checksum sidecar {}", path.display()))?;
    Ok(Some(sidecar))
}

#[cfg(test)]
pub fn write_sidecar(content_path: &Path, sha256: &str) -> Result<()> {
    let metadata = fs::metadata(content_path)
        .with_context(|| format!("reading metadata for {}", content_path.display()))?;
    let modified_unix_secs = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_secs());
    let sidecar = ChecksumSidecar {
        version: SIDECAR_VERSION,
        sha256: sha256.to_string(),
        size_bytes: metadata.len(),
        modified_unix_secs,
    };
    let path = ChecksumSidecar::sidecar_path(content_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating checksum sidecar directory {}", parent.display()))?;
    }
    let blob = serde_json::to_vec_pretty(&sidecar).context("serialising checksum sidecar")?;
    fs::write(&path, blob)
        .with_context(|| format!("writing checksum sidecar {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
pub fn content_checksum(content_path: &Path) -> Result<String> {
    let sidecar_path = ChecksumSidecar::sidecar_path(content_path);
    if let Some(sidecar) = read_sidecar(&sidecar_path)? {
        if sidecar.is_valid_for(content_path) {
            return Ok(sidecar.sha256);
        }
    }
    let sha256 = hash_file(content_path)?;
    write_sidecar(content_path, &sha256)?;
    Ok(sha256)
}

pub fn store_aggregate_checksum(store_dir: &Path) -> Result<String> {
    let marker = store_dir.join("checksum.sha256");
    let meta = store_dir.join("meta.json");
    if marker.is_file() && meta.is_file() {
        if let (Ok(marker_meta), Ok(meta_meta)) = (marker.metadata(), meta.metadata()) {
            if marker_meta
                .modified()
                .ok()
                .zip(meta_meta.modified().ok())
                .is_some_and(|(marker_modified, meta_modified)| marker_modified >= meta_modified)
            {
                if let Some(sidecar) = read_sidecar(&marker)? {
                    return Ok(sidecar.sha256);
                }
            }
        }
    }

    let mut entries = Vec::new();
    collect_store_files(store_dir, store_dir, &mut entries)?;
    entries.sort();

    let mut hasher = Sha256::new();
    for relative in &entries {
        let path = store_dir.join(relative);
        if path == marker {
            continue;
        }
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        let file_hash = hash_file(&path)?;
        hasher.update(file_hash.as_bytes());
    }
    let digest = format!("{:x}", hasher.finalize());

    let sidecar = ChecksumSidecar {
        version: SIDECAR_VERSION,
        sha256: digest.clone(),
        size_bytes: entries.len() as u64,
        modified_unix_secs: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs()),
    };
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&marker, serde_json::to_vec_pretty(&sidecar)?)?;
    Ok(digest)
}

fn collect_store_files(base: &Path, current: &Path, entries: &mut Vec<String>) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == "checksum.sha256") {
            continue;
        }
        if path.is_dir() {
            collect_store_files(base, &path, entries)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            entries.push(relative);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn sidecar_round_trip() -> Result<()> {
        let dir = tempdir()?;
        let file = dir.path().join("sample.json");
        fs::write(&file, br#"{"hello":"world"}"#)?;

        let first = content_checksum(&file)?;
        let second = content_checksum(&file)?;
        assert_eq!(first, second);

        fs::write(&file, br#"{"hello":"world!"}"#)?;
        let third = content_checksum(&file)?;
        assert_ne!(first, third);
        Ok(())
    }
}
