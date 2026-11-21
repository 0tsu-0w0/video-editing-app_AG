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

    pub fn ensure_layout(&self) -> Result<(), FolderError> {
        fs::create_dir_all(&self.root)?;
        fs::create_dir_all(self.assets_dir())?;
        fs::create_dir_all(self.cache_precomp_dir())?;
        fs::create_dir_all(self.cache_waveforms_dir())?;
        fs::create_dir_all(self.cache_proxy_dir())?;
        fs::create_dir_all(self.exports_dir())?;
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
}
