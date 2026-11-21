use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use ve_core::content_hash_hex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShaderReloadStatus {
    Loaded {
        revision: u64,
        entry_points: Vec<String>,
    },
    Error {
        last_good_revision: u64,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct HotShaderEntry {
    pub id: String,
    pub path: PathBuf,
    pub last_mtime: Option<SystemTime>,
    pub last_len: u64,
    pub last_hash: String,
    /// 最後に構文・型検証（naga）を通過した安全な WGSL ソース
    pub valid_wgsl: String,
    pub revision: u64,
    pub status: ShaderReloadStatus,
}

/// カスタム WGSL シェーダーのホットリロード管理（`naga` による静的検証＋フォールバック保護付き）
#[derive(Debug, Default)]
pub struct ShaderHotReloader {
    shaders: BTreeMap<String, HotShaderEntry>,
}

impl ShaderHotReloader {
    pub fn new() -> Self {
        Self::default()
    }

    /// `.wgsl` ファイルを監視対象として登録し、初回検証を行う
    pub fn register_file(
        &mut self,
        id: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> Result<&HotShaderEntry, String> {
        let id = id.into();
        let path = path.into();
        let source = fs::read_to_string(&path)
            .map_err(|e| format!("WGSL ファイル読み込み失敗 ({}): {e}", path.display()))?;
        let meta = fs::metadata(&path).ok();
        let mtime = meta.as_ref().and_then(|m| m.modified().ok());
        let len = meta.map(|m| m.len()).unwrap_or(source.len() as u64);
        let hash = content_hash_hex(source.as_bytes());
        let entry_points = validate_wgsl(&source)?;

        let entry = HotShaderEntry {
            id: id.clone(),
            path,
            last_mtime: mtime,
            last_len: len,
            last_hash: hash,
            valid_wgsl: source,
            revision: 1,
            status: ShaderReloadStatus::Loaded {
                revision: 1,
                entry_points,
            },
        };
        self.shaders.insert(id.clone(), entry);
        Ok(self.shaders.get(&id).unwrap())
    }

    /// 登録済み全 WGSL ファイルの更新をポーリングし、変更があれば検証＆ホットリロードする。
    /// まず `fs::metadata`（mtime・サイズ）を確認し、未変更ファイルはディスク読み込みをスキップする。
    /// 構文エラー時は直前の正常シェーダーを保持したままエラー診断を返す（アプリをクラッシュさせない）。
    pub fn poll_reload(&mut self) -> Vec<String> {
        let now = SystemTime::now();
        let mut reloaded_ids = Vec::new();
        for (id, entry) in &mut self.shaders {
            let Ok(meta) = fs::metadata(&entry.path) else {
                continue;
            };
            let mtime = meta.modified().ok();
            let len = meta.len();
            // mtime とサイズが一致し、ファイルシステム更新粒度ウィンドウ(2秒)外ならファイル読込を省略
            if mtime.is_some()
                && mtime == entry.last_mtime
                && len == entry.last_len
                && mtime
                    .and_then(|t| now.duration_since(t).ok())
                    .is_some_and(|age| age >= Duration::from_secs(2))
            {
                continue;
            }

            let Ok(source) = fs::read_to_string(&entry.path) else {
                continue;
            };
            let hash = content_hash_hex(source.as_bytes());
            entry.last_mtime = mtime;
            entry.last_len = len;
            if hash == entry.last_hash {
                continue;
            }
            entry.last_hash = hash;

            match validate_wgsl(&source) {
                Ok(entry_points) => {
                    entry.revision += 1;
                    entry.valid_wgsl = source;
                    entry.status = ShaderReloadStatus::Loaded {
                        revision: entry.revision,
                        entry_points,
                    };
                    reloaded_ids.push(id.clone());
                }
                Err(msg) => {
                    entry.status = ShaderReloadStatus::Error {
                        last_good_revision: entry.revision,
                        message: msg,
                    };
                }
            }
        }
        reloaded_ids
    }

    pub fn get(&self, id: &str) -> Option<&HotShaderEntry> {
        self.shaders.get(id)
    }

    pub fn list(&self) -> Vec<&HotShaderEntry> {
        self.shaders.values().collect()
    }

    /// 検証済みの最新 WGSL を `wgpu::ShaderModule` としてコンパイルする
    pub fn build_wgpu_module(&self, device: &wgpu::Device, id: &str) -> Option<wgpu::ShaderModule> {
        let entry = self.shaders.get(id)?;
        Some(device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&entry.id),
            source: wgpu::ShaderSource::Wgsl(entry.valid_wgsl.as_str().into()),
        }))
    }
}

/// `naga` フロントエンド＆バリデータで WGSL を厳密検証し、エントリーポイント一覧を返す
pub fn validate_wgsl(source: &str) -> Result<Vec<String>, String> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|e| format!("WGSL parse error:\n{}", e.emit_to_string(source)))?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    );
    validator
        .validate(&module)
        .map_err(|e| format!("WGSL validation error: {e}"))?;
    let eps = module
        .entry_points
        .iter()
        .map(|ep| ep.name.clone())
        .collect();
    Ok(eps)
}

/// 指定ディレクトリ内の `.wgsl` ファイルをすべて監視登録する
pub fn register_shader_dir(reloader: &mut ShaderHotReloader, dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("wgsl")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            && reloader.register_file(stem, &path).is_ok()
        {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const VALID_SHADER_V1: &str = r#"
@fragment
fn fs_custom(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(uv.x, uv.y, 0.5, 1.0);
}
"#;

    const VALID_SHADER_V2: &str = r#"
@fragment
fn fs_custom(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(1.0 - uv.x, uv.y, 0.8, 1.0);
}
"#;

    const BROKEN_SHADER: &str = r#"
@fragment
fn fs_custom() -> @location(0) vec4<f32> {
    return vec4<f32>(SYNTAX_ERROR_HERE);
}
"#;

    #[test]
    fn validates_builtin_shaders_and_hot_reloads_with_safe_rollback() {
        // 組み込みシェーダー自体の naga 検証
        assert!(validate_wgsl(include_str!("shaders/composite.wgsl")).is_ok());
        assert!(validate_wgsl(include_str!("shaders/effects.wgsl")).is_ok());

        let tmp = tempdir().unwrap();
        let shader_path = tmp.path().join("my_effect.wgsl");
        fs::write(&shader_path, VALID_SHADER_V1).unwrap();

        let mut reloader = ShaderHotReloader::new();
        let entry = reloader.register_file("my_effect", &shader_path).unwrap();
        assert_eq!(entry.revision, 1);

        // 正常更新 -> revision 2 へホットリロード
        fs::write(&shader_path, VALID_SHADER_V2).unwrap();
        let reloaded = reloader.poll_reload();
        assert_eq!(reloaded, vec!["my_effect"]);
        assert_eq!(reloader.get("my_effect").unwrap().revision, 2);

        // 壊れた WGSL を保存 -> エラー診断を出しつつ revision 2 の正常シェーダーを保持
        fs::write(&shader_path, BROKEN_SHADER).unwrap();
        let reloaded = reloader.poll_reload();
        assert!(reloaded.is_empty());
        let entry = reloader.get("my_effect").unwrap();
        assert_eq!(entry.revision, 2);
        assert_eq!(entry.valid_wgsl, VALID_SHADER_V2);
        assert!(matches!(entry.status, ShaderReloadStatus::Error { .. }));
    }
}
