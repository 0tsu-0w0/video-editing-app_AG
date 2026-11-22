use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("Cache I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid cache key (path traversal or unsupported characters): {0}")]
    InvalidKey(String),
}

/// キャッシュキーにパストラバーサル文字やパス区切りが含まれないことを検証する
fn validate_cache_key(key: &str) -> Result<(), CacheError> {
    if !key.is_empty()
        && key.len() <= 128
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        Ok(())
    } else {
        Err(CacheError::InvalidKey(key.to_string()))
    }
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
                && validate_cache_key(stem).is_ok()
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
        validate_cache_key(key)?;
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
        validate_cache_key(key)?;
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
        validate_cache_key(key)?;
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

/// v1.0 #18: アプリ再起動後も再利用できる SSD 永続フレームキャッシュ（`cache/frames/`）
/// コンテンツハッシュキー＋LRU 容量上限管理で RGBA8 または Linear f32 フレームを安全に永続化する。
#[derive(Debug)]
pub struct PersistentFrameCache {
    inner: PrecompCacheManager,
    hits: u64,
    misses: u64,
}

impl PersistentFrameCache {
    const MAGIC: &'static [u8; 4] = b"VEFC";

    pub fn open(dir: impl Into<PathBuf>, max_bytes: u64) -> Result<Self, CacheError> {
        Ok(Self {
            inner: PrecompCacheManager::new(dir, max_bytes)?,
            hits: 0,
            misses: 0,
        })
    }

    pub fn total_bytes(&self) -> u64 {
        self.inner.total_bytes()
    }

    pub fn entry_count(&self) -> usize {
        self.inner.entry_count()
    }

    pub fn hits(&self) -> u64 {
        self.hits
    }

    pub fn misses(&self) -> u64 {
        self.misses
    }

    /// 決定論的フレームハッシュキーを生成する
    pub fn frame_key(scene_signature: &str, width: u32, height: u32, frame_idx: i64) -> String {
        let payload = format!("{scene_signature}|{width}x{height}|f{frame_idx}");
        format!("f_{}", content_hash_hex(payload.as_bytes()))
    }

    /// RGBA8 フレーム（`width * height * 4` バイト）をディスクへ永続保存する
    pub fn put_rgba8(
        &mut self,
        key: &str,
        width: u32,
        height: u32,
        rgba8: &[u8],
    ) -> Result<PathBuf, CacheError> {
        let mut buf = Vec::with_capacity(12 + rgba8.len());
        buf.extend_from_slice(Self::MAGIC);
        buf.extend_from_slice(&width.to_le_bytes());
        buf.extend_from_slice(&height.to_le_bytes());
        buf.extend_from_slice(rgba8);
        self.inner.put(key, &buf)
    }

    /// ディスクキャッシュから RGBA8 フレームを読み出す（サイズ整合検証付き）
    pub fn get_rgba8(&mut self, key: &str) -> Result<Option<(u32, u32, Vec<u8>)>, CacheError> {
        let Some(raw) = self.inner.get(key)? else {
            self.misses += 1;
            return Ok(None);
        };
        if raw.len() < 12 || &raw[0..4] != Self::MAGIC {
            self.misses += 1;
            return Ok(None);
        }
        let w = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]);
        let h = u32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]);
        let expected = (w as usize).saturating_mul(h as usize).saturating_mul(4);
        if raw.len() != 12 + expected {
            self.misses += 1;
            return Ok(None);
        }
        self.hits += 1;
        Ok(Some((w, h, raw[12..].to_vec())))
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

    #[test]
    fn rejects_path_traversal_cache_keys() {
        let tmp = tempdir().unwrap();
        let mut cache = PrecompCacheManager::new(tmp.path().join("precomp"), 1024).unwrap();

        for bad in ["../escape", "a/b", r"a\b", "", "..", "foo.bar"] {
            assert!(matches!(
                cache.put(bad, b"data"),
                Err(CacheError::InvalidKey(_))
            ));
            assert!(matches!(cache.get(bad), Err(CacheError::InvalidKey(_))));
            assert!(matches!(
                cache.invalidate(bad),
                Err(CacheError::InvalidKey(_))
            ));
        }
    }

    #[test]
    fn persistent_frame_cache_survives_reopen_and_enforces_lru() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join("frames");
        let key = PersistentFrameCache::frame_key("scene_sig_v1", 4, 2, 10);
        let rgba = vec![128u8; 4 * 2 * 4];

        {
            let mut fc = PersistentFrameCache::open(&dir, 1024).unwrap();
            fc.put_rgba8(&key, 4, 2, &rgba).unwrap();
            assert_eq!(fc.entry_count(), 1);
        }

        // アプリ再起動をシミュレートして再オープンしても即ヒットする
        let mut reopened = PersistentFrameCache::open(&dir, 1024).unwrap();
        let loaded = reopened.get_rgba8(&key).unwrap().unwrap();
        assert_eq!(loaded.0, 4);
        assert_eq!(loaded.1, 2);
        assert_eq!(loaded.2, rgba);
        assert_eq!(reopened.hits(), 1);
    }
}
