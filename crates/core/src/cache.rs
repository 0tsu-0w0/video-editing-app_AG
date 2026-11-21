use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("Cache I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// FNV-1a 64bit の決定論的ハッシュ（キャッシュキー生成用）
pub fn content_hash_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[derive(Debug, Clone)]
struct CacheEntry {
    path: PathBuf,
    size_bytes: u64,
    last_access_tick: u64,
}

/// `cache/precomp/` のコンテンツハッシュ管理＆LRU容量制限マネージャ（仕様書 5.1）
#[derive(Debug)]
pub struct PrecompCacheManager {
    dir: PathBuf,
    max_bytes: u64,
    total_bytes: u64,
    access_counter: u64,
    entries: HashMap<String, CacheEntry>,
}

impl PrecompCacheManager {
    pub fn new(dir: impl Into<PathBuf>, max_bytes: u64) -> Result<Self, CacheError> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        let mut mgr = Self {
            dir,
            max_bytes,
            total_bytes: 0,
            access_counter: 0,
            entries: HashMap::new(),
        };
        mgr.scan_existing()?;
        Ok(mgr)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    fn scan_existing(&mut self) -> Result<(), CacheError> {
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file()
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                let size = entry.metadata()?.len();
                self.access_counter += 1;
                self.total_bytes += size;
                self.entries.insert(
                    stem.to_string(),
                    CacheEntry {
                        path,
                        size_bytes: size,
                        last_access_tick: self.access_counter,
                    },
                );
            }
        }
        self.evict_if_needed()?;
        Ok(())
    }

    /// ハッシュキーでキャッシュを読み出し、LRUアクセス時刻を更新する
    pub fn get(&mut self, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
        if let Some(entry) = self.entries.get_mut(key) {
            if !entry.path.exists() {
                self.total_bytes = self.total_bytes.saturating_sub(entry.size_bytes);
                self.entries.remove(key);
                return Ok(None);
            }
            self.access_counter += 1;
            entry.last_access_tick = self.access_counter;
            let data = fs::read(&entry.path)?;
            Ok(Some(data))
        } else {
            Ok(None)
        }
    }

    /// ハッシュキーでキャッシュを保存し、上限超過時は LRU（最古参照）から自動削除する
    pub fn put(&mut self, key: &str, data: &[u8]) -> Result<PathBuf, CacheError> {
        let path = self.dir.join(format!("{key}.bin"));
        fs::write(&path, data)?;
        let size = data.len() as u64;

        if let Some(prev) = self.entries.remove(key) {
            self.total_bytes = self.total_bytes.saturating_sub(prev.size_bytes);
        }

        self.access_counter += 1;
        self.total_bytes += size;
        self.entries.insert(
            key.to_string(),
            CacheEntry {
                path: path.clone(),
                size_bytes: size,
                last_access_tick: self.access_counter,
            },
        );

        self.evict_if_needed()?;
        Ok(path)
    }

    pub fn invalidate(&mut self, key: &str) -> Result<(), CacheError> {
        if let Some(entry) = self.entries.remove(key) {
            self.total_bytes = self.total_bytes.saturating_sub(entry.size_bytes);
            if entry.path.exists() {
                fs::remove_file(entry.path)?;
            }
        }
        Ok(())
    }

    fn evict_if_needed(&mut self) -> Result<(), CacheError> {
        while self.total_bytes > self.max_bytes && !self.entries.is_empty() {
            let oldest_key = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.last_access_tick)
                .map(|(k, _)| k.clone());
            if let Some(k) = oldest_key {
                self.invalidate(&k)?;
            } else {
                break;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn lru_evicts_least_recently_used_entry_when_over_budget() {
        let tmp = tempdir().unwrap();
        let mut cache = PrecompCacheManager::new(tmp.path().join("precomp"), 20).unwrap();

        let k1 = content_hash_hex(b"frame_1");
        let k2 = content_hash_hex(b"frame_2");
        let k3 = content_hash_hex(b"frame_3");

        cache.put(&k1, &[1u8; 10]).unwrap();
        cache.put(&k2, &[2u8; 10]).unwrap();
        assert_eq!(cache.total_bytes(), 20);

        // k1 にアクセスして最新化 -> k2 が最古になる
        assert_eq!(cache.get(&k1).unwrap(), Some(vec![1u8; 10]));

        // k3 (10B) 追加で上限20B超過 -> k2 が削除され k1 と k3 が残る
        cache.put(&k3, &[3u8; 10]).unwrap();
        assert_eq!(cache.total_bytes(), 20);
        assert!(cache.get(&k2).unwrap().is_none());
        assert!(cache.get(&k1).unwrap().is_some());
        assert!(cache.get(&k3).unwrap().is_some());
    }
}
