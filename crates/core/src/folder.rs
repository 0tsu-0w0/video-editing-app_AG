use crate::preset::ItemAlias;
use crate::project::{AssetRef, Project};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FolderError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Asset not found on disk: {0}")]
    AssetOffline(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetResolution {
    Found(PathBuf),
    Offline,
}

/// `history.jsonl` に追記される操作ジャーナルエントリ（Phase 1b Command Bus 連携用）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub seq: u64,
    pub timestamp_ms: u64,
    pub source: String,
    pub command_type: String,
    pub payload: serde_json::Value,
}

/// 仕様書 5章の「1フォルダ管理方式」を管理するハンドル
#[derive(Debug, Clone)]
pub struct ProjectFolder {
    root: PathBuf,
}

impl ProjectFolder {
    pub fn init(root: impl Into<PathBuf>, project: &Project) -> Result<Self, FolderError> {
        let folder = Self { root: root.into() };
        folder.ensure_layout()?;
        folder.save_project(project)?;
        Ok(folder)
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<(Self, Project), FolderError> {
        let folder = Self { root: root.into() };
        folder.ensure_layout()?;
        let project = folder.load_project()?;
        Ok((folder, project))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn project_json_path(&self) -> PathBuf {
        self.root.join("project.json")
    }

    pub fn history_jsonl_path(&self) -> PathBuf {
        self.root.join("history.jsonl")
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.root.join("assets")
    }

    pub fn aliases_dir(&self) -> PathBuf {
        self.root.join("aliases")
    }

    pub fn cache_precomp_dir(&self) -> PathBuf {
        self.root.join("cache").join("precomp")
    }

    pub fn cache_waveforms_dir(&self) -> PathBuf {
        self.root.join("cache").join("waveforms")
    }

    pub fn cache_proxy_dir(&self) -> PathBuf {
        self.root.join("cache").join("proxy")
    }

    pub fn exports_dir(&self) -> PathBuf {
        self.root.join("exports")
    }

    pub fn backups_dir(&self) -> PathBuf {
        self.root.join("backups")
    }

    pub fn ensure_layout(&self) -> Result<(), FolderError> {
        fs::create_dir_all(&self.root)?;
        fs::create_dir_all(self.assets_dir())?;
        fs::create_dir_all(self.aliases_dir())?;
        fs::create_dir_all(self.cache_precomp_dir())?;
        fs::create_dir_all(self.cache_waveforms_dir())?;
        fs::create_dir_all(self.cache_proxy_dir())?;
        fs::create_dir_all(self.exports_dir())?;
        fs::create_dir_all(self.backups_dir())?;
        if !self.history_jsonl_path().exists() {
            fs::File::create(self.history_jsonl_path())?;
        }
        Ok(())
    }

    /// 一時ファイル経由＋`sync_all()` で原子的（atomic）かつ停電安全に `project.json` を保存する
    pub fn save_project(&self, project: &Project) -> Result<(), FolderError> {
        let tmp_path = self.root.join("project.json.tmp");
        let final_path = self.project_json_path();
        let json = serde_json::to_string_pretty(project)?;
        {
            let mut file = fs::File::create(&tmp_path)?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
        }
        fs::rename(&tmp_path, &final_path)?;
        Ok(())
    }

    pub fn load_project(&self) -> Result<Project, FolderError> {
        let data = fs::read_to_string(self.project_json_path())?;
        let project = serde_json::from_str(&data)?;
        Ok(project)
    }

    pub fn append_history(&self, entry: &HistoryEntry) -> Result<(), FolderError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.history_jsonl_path())?;
        let line = serde_json::to_string(entry)?;
        writeln!(file, "{line}")?;
        file.sync_data()?;
        Ok(())
    }

    pub fn read_history(&self) -> Result<Vec<HistoryEntry>, FolderError> {
        let file = fs::File::open(self.history_jsonl_path())?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            entries.push(serde_json::from_str(&line)?);
        }
        Ok(entries)
    }

    /// 正常な `project.json` スナップショット保存後に `history.jsonl` を空にリセットする
    pub fn clear_history(&self) -> Result<(), FolderError> {
        let file = fs::File::create(self.history_jsonl_path())?;
        file.sync_all()?;
        Ok(())
    }

    /// アセット参照を解決する（安全なプロジェクト内相対パス優先 → 絶対パスフォールバック → オフライン判定）
    pub fn resolve_asset(&self, asset: &AssetRef) -> AssetResolution {
        if let Some(rel) = &asset.relative_path
            && is_safe_project_relative_path(rel)
        {
            let candidate = self.root.join(rel);
            if candidate.exists() {
                return AssetResolution::Found(candidate);
            }
        }
        if let Some(abs) = &asset.fallback_absolute_path
            && abs.exists()
        {
            return AssetResolution::Found(abs.clone());
        }
        AssetResolution::Offline
    }

    /// プロジェクト収集: 外部参照アセットを `assets/` へコピーし相対パス参照へ更新する。
    /// 異なるソース同士でファイル名が衝突する場合はアセットIDプレフィックスを付与して上書きを防止する。
    pub fn collect_asset(&self, asset: &mut AssetRef) -> Result<PathBuf, FolderError> {
        let src = match self.resolve_asset(asset) {
            AssetResolution::Found(p) => p,
            AssetResolution::Offline => return Err(FolderError::AssetOffline(asset.name.clone())),
        };
        let file_name = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .filter(|n| !n.is_empty() && n != "." && n != "..")
            .unwrap_or_else(|| format!("{}.bin", asset.id));
        let mut dest_rel = PathBuf::from("assets").join(&file_name);
        let mut dest_abs = self.root.join(&dest_rel);
        if dest_abs.exists() && src != dest_abs {
            let short_id = &asset.id.simple().to_string()[..8];
            dest_rel = PathBuf::from("assets").join(format!("{short_id}_{file_name}"));
            dest_abs = self.root.join(&dest_rel);
        }
        if src != dest_abs {
            fs::copy(&src, &dest_abs)?;
        }
        asset.relative_path = Some(dest_rel);
        asset.collected = true;
        Ok(dest_abs)
    }

    /// AviUtl2 エイリアスを `aliases/<safe_name>.json` へ保存する（パストラバーサル防止付き）
    pub fn save_alias(&self, alias: &ItemAlias) -> Result<PathBuf, FolderError> {
        fs::create_dir_all(self.aliases_dir())?;
        let file_name = format!("{}.json", sanitize_alias_filename(&alias.name));
        let path = self.aliases_dir().join(file_name);
        let json = serde_json::to_string_pretty(alias)?;
        fs::write(&path, json)?;
        Ok(path)
    }

    /// `aliases/` 内の保存済みエイリアスを名前順で全件読み込む
    pub fn load_aliases(&self) -> Vec<ItemAlias> {
        let dir = self.aliases_dir();
        let Ok(entries) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json")
                && let Ok(text) = fs::read_to_string(&path)
                && let Ok(alias) = serde_json::from_str::<ItemAlias>(&text)
            {
                out.push(alias);
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn delete_alias(&self, name: &str) -> Result<(), FolderError> {
        let file_name = format!("{}.json", sanitize_alias_filename(name));
        let path = self.aliases_dir().join(file_name);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// 定期自動保存スナップショットを `backups/autosave_<ms>_<seq>.json` へ原子的に書き出し、
    /// `max_keep` 件を超える古いバックアップを自動ローテーション削除する。
    pub fn save_auto_backup(
        &self,
        project: &Project,
        max_keep: usize,
    ) -> Result<PathBuf, FolderError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};
        static BACKUP_SEQ: AtomicU64 = AtomicU64::new(0);

        fs::create_dir_all(self.backups_dir())?;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let seq = BACKUP_SEQ.fetch_add(1, Ordering::Relaxed);
        let file_name = format!("autosave_{now_ms:016}_{seq:04}.json");
        let tmp_path = self.backups_dir().join(format!("{file_name}.tmp"));
        let final_path = self.backups_dir().join(file_name);
        let json = serde_json::to_string_pretty(project)?;
        {
            let mut file = fs::File::create(&tmp_path)?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
        }
        fs::rename(&tmp_path, &final_path)?;

        // 古いバックアップをローテーション削除
        let keep = max_keep.max(1);
        let all = self.list_backups();
        if all.len() > keep {
            for old in &all[keep..] {
                let _ = fs::remove_file(old);
            }
        }
        Ok(final_path)
    }

    /// `backups/` 内の自動保存スナップショット一覧を新しい順で返す
    pub fn list_backups(&self) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(self.backups_dir()) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().and_then(|e| e.to_str()) == Some("json")
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("autosave_"))
            })
            .collect();
        files.sort();
        files.reverse();
        files
    }

    /// 指定バックアップスナップショットを読み込む（プロジェクト内 `backups/` 配下のみ許可）
    pub fn load_backup(&self, backup_path: &Path) -> Result<Project, FolderError> {
        let canonical_dir = fs::canonicalize(self.backups_dir())?;
        let canonical_file = fs::canonicalize(backup_path)?;
        if !canonical_file.starts_with(&canonical_dir) {
            return Err(FolderError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "backup path outside project backups directory",
            )));
        }
        let data = fs::read_to_string(&canonical_file)?;
        Ok(serde_json::from_str(&data)?)
    }
}

fn sanitize_alias_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' || !c.is_ascii() {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    let trimmed = cleaned.trim_matches(['_', '.']);
    if trimmed.is_empty() {
        "alias".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 信頼できない `project.json` 内の `relative_path` によるディレクトリトラバーサルを防止する
fn is_safe_project_relative_path(path: &Path) -> bool {
    use std::path::Component;
    let mut has_normal = false;
    for comp in path.components() {
        match comp {
            Component::Normal(_) => has_normal = true,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    has_normal
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{MediaKind, ProjectSettings};
    use tempfile::tempdir;
    use uuid::Uuid;

    #[test]
    fn project_folder_roundtrip_and_asset_collect() {
        let tmp = tempdir().unwrap();
        let proj_dir = tmp.path().join("MyVideoProject");
        let external_file = tmp.path().join("sample.wav");
        fs::write(&external_file, b"RIFF....").unwrap();

        let mut project = Project::new("Test Project", ProjectSettings::default());
        project.assets.push(AssetRef {
            id: Uuid::new_v4(),
            name: "sample.wav".to_string(),
            kind: MediaKind::Audio,
            relative_path: None,
            fallback_absolute_path: Some(external_file.clone()),
            collected: false,
        });

        let folder = ProjectFolder::init(&proj_dir, &project).unwrap();
        assert!(folder.cache_precomp_dir().exists());
        assert!(folder.cache_waveforms_dir().exists());
        assert!(folder.cache_proxy_dir().exists());
        assert!(folder.exports_dir().exists());
        assert!(folder.backups_dir().exists());

        // History journal
        folder
            .append_history(&HistoryEntry {
                seq: 1,
                timestamp_ms: 1_700_000_000_000,
                source: "gui".to_string(),
                command_type: "add_object".to_string(),
                payload: serde_json::json!({"name": "Title"}),
            })
            .unwrap();
        assert_eq!(folder.read_history().unwrap().len(), 1);

        // Asset resolution & collect
        assert_eq!(
            folder.resolve_asset(&project.assets[0]),
            AssetResolution::Found(external_file)
        );
        let collected_path = folder.collect_asset(&mut project.assets[0]).unwrap();
        assert!(collected_path.starts_with(folder.assets_dir()));
        assert!(project.assets[0].collected);

        folder.save_project(&project).unwrap();
        let (_, loaded) = ProjectFolder::open(&proj_dir).unwrap();
        assert_eq!(loaded.name, "Test Project");
        assert_eq!(loaded.assets.len(), 1);
        assert!(loaded.assets[0].collected);
    }

    #[test]
    fn rejects_parent_traversal_in_relative_path() {
        let tmp = tempdir().unwrap();
        let secret = tmp.path().join("outside_secret.txt");
        fs::write(&secret, b"top-secret").unwrap();

        let proj_dir = tmp.path().join("Project");
        let project = Project::new("SecTest", ProjectSettings::default());
        let folder = ProjectFolder::init(&proj_dir, &project).unwrap();

        let malicious = AssetRef {
            id: Uuid::new_v4(),
            name: "evil".into(),
            kind: MediaKind::Audio,
            relative_path: Some(PathBuf::from("../outside_secret.txt")),
            fallback_absolute_path: None,
            collected: true,
        };
        assert_eq!(folder.resolve_asset(&malicious), AssetResolution::Offline);
    }

    #[test]
    fn collect_asset_avoids_filename_collision() {
        let tmp = tempdir().unwrap();
        let dir_a = tmp.path().join("dir_a");
        let dir_b = tmp.path().join("dir_b");
        fs::create_dir_all(&dir_a).unwrap();
        fs::create_dir_all(&dir_b).unwrap();
        let file_a = dir_a.join("voice.wav");
        let file_b = dir_b.join("voice.wav");
        fs::write(&file_a, b"CONTENT_A").unwrap();
        fs::write(&file_b, b"CONTENT_B").unwrap();

        let project = Project::new("CollisionTest", ProjectSettings::default());
        let folder = ProjectFolder::init(tmp.path().join("proj"), &project).unwrap();

        let mut asset_a = AssetRef {
            id: Uuid::new_v4(),
            name: "voice.wav".into(),
            kind: MediaKind::Audio,
            relative_path: None,
            fallback_absolute_path: Some(file_a),
            collected: false,
        };
        let mut asset_b = AssetRef {
            id: Uuid::new_v4(),
            name: "voice.wav".into(),
            kind: MediaKind::Audio,
            relative_path: None,
            fallback_absolute_path: Some(file_b),
            collected: false,
        };

        let path_a = folder.collect_asset(&mut asset_a).unwrap();
        let path_b = folder.collect_asset(&mut asset_b).unwrap();
        assert_ne!(path_a, path_b);
        assert_eq!(fs::read(&path_a).unwrap(), b"CONTENT_A");
        assert_eq!(fs::read(&path_b).unwrap(), b"CONTENT_B");
    }

    #[test]
    fn saves_loads_and_sanitizes_aliases_safely() {
        use crate::project::{ItemKind, RgbaColor, TextAnimator, TimelineItem};
        use crate::time::RationalTime;

        let tmp = tempdir().unwrap();
        let project = Project::new("AliasTest", ProjectSettings::default());
        let folder = ProjectFolder::init(tmp.path().join("proj"), &project).unwrap();

        let item = TimelineItem::new(
            "Sample",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Text {
                content: "テスト".into(),
                font_family: "sans-serif".into(),
                font_size: 40.0,
                color: RgbaColor::WHITE,
                animator: TextAnimator::default(),
            },
        );
        let alias = ItemAlias::from_item("../evil/重要テロップ", "字幕", &item);
        let saved_path = folder.save_alias(&alias).unwrap();
        assert!(saved_path.starts_with(folder.aliases_dir()));

        let loaded = folder.load_aliases();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "../evil/重要テロップ");

        folder.delete_alias(&alias.name).unwrap();
        assert!(folder.load_aliases().is_empty());
    }

    #[test]
    fn auto_backup_creates_rotates_and_restores_safely() {
        let tmp = tempdir().unwrap();
        let mut project = Project::new("AutoSaveTest", ProjectSettings::default());
        let folder = ProjectFolder::init(tmp.path().join("proj"), &project).unwrap();

        for i in 1..=5 {
            project.name = format!("Rev {i}");
            folder.save_auto_backup(&project, 3).unwrap();
        }
        let list = folder.list_backups();
        assert_eq!(list.len(), 3, "should rotate to max_keep=3");
        let latest = folder.load_backup(&list[0]).unwrap();
        assert_eq!(latest.name, "Rev 5");
    }
}
