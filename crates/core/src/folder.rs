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

    /// 一時ファイル経由で原子的（atomic）に `project.json` を保存する
    pub fn save_project(&self, project: &Project) -> Result<(), FolderError> {
        let tmp_path = self.root.join("project.json.tmp");
        let final_path = self.project_json_path();
        let json = serde_json::to_string_pretty(project)?;
        fs::write(&tmp_path, json)?;
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

    /// アセット参照を解決する（相対パス優先 → 絶対パスフォールバック → オフライン判定）
    pub fn resolve_asset(&self, asset: &AssetRef) -> AssetResolution {
        if let Some(rel) = &asset.relative_path {
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

    /// プロジェクト収集: 外部参照アセットを `assets/` へコピーし相対パス参照へ更新する
    pub fn collect_asset(&self, asset: &mut AssetRef) -> Result<PathBuf, FolderError> {
        let src = match self.resolve_asset(asset) {
            AssetResolution::Found(p) => p,
            AssetResolution::Offline => return Err(FolderError::AssetOffline(asset.name.clone())),
        };
        let file_name = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("{}.bin", asset.id));
        let dest_rel = PathBuf::from("assets").join(&file_name);
        let dest_abs = self.root.join(&dest_rel);
        if src != dest_abs {
            fs::copy(&src, &dest_abs)?;
        }
        asset.relative_path = Some(dest_rel);
        asset.collected = true;
        Ok(dest_abs)
    }
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
}
