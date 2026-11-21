use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

const SUPPORTED_WATCH_EXTS: &[&str] = &[
    "wav", "mp3", "flac", "ogg", "m4a", "aac", "mp4", "mov", "mkv", "webm", "avi", "png", "jpg",
    "jpeg", "webp", "bmp", "psd",
];

fn is_supported_watch_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| SUPPORTED_WATCH_EXTS.contains(&e.to_ascii_lowercase().as_str()))
            .unwrap_or(false)
}

/// かんしくん風 フォルダ監視バックグラウンドハンドル
///
/// 指定フォルダへ新しく生成された音声（`.wav`＋同名`.txt`/`.lab`）・画像・動画・PSD を
/// 書き込み完了（2回連続ポーリングでサイズ＆更新時刻が安定）を確認してから通知する。
pub struct WatchFolderHandle {
    dir: PathBuf,
    stop_flag: Arc<AtomicBool>,
    rx: Receiver<PathBuf>,
    handle: Option<JoinHandle<()>>,
}

impl WatchFolderHandle {
    pub fn start(dir: PathBuf) -> Result<Self, String> {
        if !dir.is_dir() {
            return Err(format!("監視フォルダが存在しません: {}", dir.display()));
        }
        let stop_flag = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let stop_clone = Arc::clone(&stop_flag);
        let watch_dir = dir.clone();

        // 起動時点で既に存在するファイルは既読登録し、起動後の新規追加分だけ取り込む
        let mut seen: HashSet<PathBuf> = HashSet::new();
        if let Ok(entries) = fs::read_dir(&watch_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if is_supported_watch_file(&p) {
                    seen.insert(p);
                }
            }
        }

        let handle = thread::spawn(move || {
            // (len, mtime, stable_ticks)
            let mut pending: HashMap<PathBuf, (u64, Option<SystemTime>, u8)> = HashMap::new();
            while !stop_clone.load(Ordering::Relaxed) {
                if let Ok(entries) = fs::read_dir(&watch_dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if seen.contains(&p) || !is_supported_watch_file(&p) {
                            continue;
                        }
                        let Ok(meta) = entry.metadata() else {
                            continue;
                        };
                        let len = meta.len();
                        let mtime = meta.modified().ok();
                        if len == 0 {
                            continue;
                        }
                        let ready = match pending.get_mut(&p) {
                            Some((prev_len, prev_mt, stable)) => {
                                if *prev_len == len && *prev_mt == mtime {
                                    *stable = stable.saturating_add(1);
                                } else {
                                    *prev_len = len;
                                    *prev_mt = mtime;
                                    *stable = 0;
                                }
                                *stable >= 2
                            }
                            None => {
                                pending.insert(p.clone(), (len, mtime, 0));
                                false
                            }
                        };
                        if ready {
                            pending.remove(&p);
                            seen.insert(p.clone());
                            if tx.send(p).is_err() {
                                return;
                            }
                        }
                    }
                }
                thread::sleep(Duration::from_millis(120));
            }
        });

        Ok(Self {
            dir,
            stop_flag,
            rx,
            handle: Some(handle),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn poll_ready_files(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        while let Ok(p) = self.rx.try_recv() {
            out.push(p);
        }
        out
    }

    pub fn stop(mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for WatchFolderHandle {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use ve_media::write_wav_pcm16;

    #[test]
    fn watch_folder_detects_only_newly_added_stable_files() {
        let tmp = tempdir().unwrap();
        let existing = tmp.path().join("old.wav");
        write_wav_pcm16(&existing, &[0.1; 1600], 16_000, 1).unwrap();

        let watcher = WatchFolderHandle::start(tmp.path().to_path_buf()).unwrap();
        let new_wav = tmp.path().join("voice_001.wav");
        let new_txt = tmp.path().join("voice_001.txt");
        fs::write(&new_txt, "こんにちは、ずんだもんです！").unwrap();
        write_wav_pcm16(&new_wav, &[0.2; 1600], 16_000, 1).unwrap();

        let mut detected = Vec::new();
        for _ in 0..20 {
            detected.extend(watcher.poll_ready_files());
            if !detected.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(80));
        }
        watcher.stop();

        assert_eq!(detected, vec![new_wav]);
    }
}
