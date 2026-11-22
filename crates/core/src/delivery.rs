//! v1.0 #19: 仕上げ・受け渡し（プロジェクト検証チェックリスト ＆ Pure-Rust ZIP アーカイブパッケージ化）
//!
//! 外部クレート依存ゼロ・ゼロ `unsafe` で標準 PKZIP（Store + IEEE 802.3 CRC-32）を生成し、
//! オフライン素材・音割れリスク・クリップ重複・式構文エラー等を一括検証する。

use crate::expression::compile_expression;
use crate::folder::{AssetResolution, ProjectFolder};
use crate::project::{ItemId, ItemKind, Project};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// 検証チェック項目の重要度
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    Error,
    Warning,
    Info,
}

/// プロジェクト書き出し前検証で見つかった 1 件の診断項目
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryIssue {
    pub severity: IssueSeverity,
    pub category: String,
    pub message: String,
    pub item_id: Option<ItemId>,
}

/// プロジェクト検証レポート
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryReport {
    pub passed: bool,
    pub error_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
    pub issues: Vec<DeliveryIssue>,
}

/// 書き出し前のプロジェクト検証チェックリストを実行する:
/// - オフライン（行方不明）アセット検出
/// - 空タイムライン・ゼロ尺クリップ検出
/// - 音声ゲイン過大（クリッピング危険）および目標 LUFS 過大チェック
/// - 同一レイヤー上のクリップ重複検出
/// - 未解決エクスプレッション構文エラー検出
/// - シーン解像度・FPS 妥当性（奇数解像度による yuv420p エンコードエラー警告）
pub fn validate_project_delivery(
    project: &Project,
    folder: Option<&ProjectFolder>,
) -> DeliveryReport {
    let mut issues = Vec::new();

    // 1. オフライン素材チェック
    for asset in &project.assets {
        let found = match folder {
            Some(f) => matches!(f.resolve_asset(asset), AssetResolution::Found(_)),
            None => asset
                .fallback_absolute_path
                .as_ref()
                .is_some_and(|p| p.exists()),
        };
        if !found {
            issues.push(DeliveryIssue {
                severity: IssueSeverity::Error,
                category: "offline_asset".into(),
                message: format!("オフライン素材が検出されました: {}", asset.name),
                item_id: None,
            });
        }
    }

    // 2. アクティブシーン詳細検証
    let Some(scene) = project.active_scene() else {
        issues.push(DeliveryIssue {
            severity: IssueSeverity::Error,
            category: "scene".into(),
            message: "アクティブなシーンが存在しません".into(),
            item_id: None,
        });
        return finalize_report(issues);
    };

    if scene.width % 2 != 0 || scene.height % 2 != 0 {
        issues.push(DeliveryIssue {
            severity: IssueSeverity::Warning,
            category: "resolution".into(),
            message: format!(
                "シーン解像度 {}x{} に奇数が含まれています（H.264/H.265 書き出し時は偶数を推奨）",
                scene.width, scene.height
            ),
            item_id: None,
        });
    }

    if let Some(lufs) = scene.target_lufs
        && lufs > -9.0
    {
        issues.push(DeliveryIssue {
            severity: IssueSeverity::Warning,
            category: "audio_clipping".into(),
            message: format!(
                "目標ラウドネス {lufs:.1} LUFS は過大でクリッピングの恐れがあります（推奨 -14.0 LUFS）"
            ),
            item_id: None,
        });
    }

    let total_items: usize = scene.layers.iter().map(|l| l.items.len()).sum();
    if total_items == 0 {
        issues.push(DeliveryIssue {
            severity: IssueSeverity::Warning,
            category: "empty_timeline".into(),
            message: "タイムラインにクリップが 1 件も配置されていません".into(),
            item_id: None,
        });
    }

    for (l_idx, layer) in scene.layers.iter().enumerate() {
        if layer.volume > 2.0 && !layer.muted {
            issues.push(DeliveryIssue {
                severity: IssueSeverity::Warning,
                category: "audio_clipping".into(),
                message: format!(
                    "L{l_idx} ({}) のトラック音量が {:.0}% と高すぎます（音割れ注意）",
                    layer.name,
                    layer.volume * 100.0
                ),
                item_id: None,
            });
        }

        // 同一レイヤー内クリップ重複チェック
        let mut sorted: Vec<_> = layer.items.iter().collect();
        sorted.sort_by_key(|i| i.start);
        for win in sorted.windows(2) {
            if win[0].end() > win[1].start {
                issues.push(DeliveryIssue {
                    severity: IssueSeverity::Warning,
                    category: "clip_overlap".into(),
                    message: format!(
                        "L{l_idx} でクリップ「{}」と「{}」が時間重複しています",
                        win[0].name, win[1].name
                    ),
                    item_id: Some(win[1].id),
                });
            }
        }

        for item in &layer.items {
            if item.duration.as_seconds() <= 0.001 {
                issues.push(DeliveryIssue {
                    severity: IssueSeverity::Error,
                    category: "zero_duration".into(),
                    message: format!("クリップ「{}」の尺が 0 秒です", item.name),
                    item_id: Some(item.id),
                });
            }

            // クリップ音量過大チェック
            if let ItemKind::Audio { volume, .. } | ItemKind::Video { volume, .. } = item.kind
                && volume * layer.volume > 2.2
                && !layer.muted
            {
                issues.push(DeliveryIssue {
                    severity: IssueSeverity::Warning,
                    category: "audio_clipping".into(),
                    message: format!(
                        "クリップ「{}」の合成ゲインが {:.0}% です（クリッピング注意）",
                        item.name,
                        volume * layer.volume * 100.0
                    ),
                    item_id: Some(item.id),
                });
            }

            // エクスプレッション構文チェック
            for (prop, expr) in &item.expressions {
                if let Err(err) = compile_expression(expr) {
                    issues.push(DeliveryIssue {
                        severity: IssueSeverity::Error,
                        category: "expression_error".into(),
                        message: format!(
                            "クリップ「{}」のプロパティ '{prop}' の式エラー: {err}",
                            item.name
                        ),
                        item_id: Some(item.id),
                    });
                }
            }
        }
    }

    if issues.is_empty() {
        issues.push(DeliveryIssue {
            severity: IssueSeverity::Info,
            category: "ok".into(),
            message: format!(
                "全チェック合格（{}x{} @ {:.2}fps / {} クリップ / {:.2}秒）",
                scene.width,
                scene.height,
                scene.fps.as_f64(),
                total_items,
                scene.total_duration().as_seconds()
            ),
            item_id: None,
        });
    }

    finalize_report(issues)
}

fn finalize_report(issues: Vec<DeliveryIssue>) -> DeliveryReport {
    let error_count = issues
        .iter()
        .filter(|i| i.severity == IssueSeverity::Error)
        .count();
    let warning_count = issues
        .iter()
        .filter(|i| i.severity == IssueSeverity::Warning)
        .count();
    let info_count = issues
        .iter()
        .filter(|i| i.severity == IssueSeverity::Info)
        .count();
    DeliveryReport {
        passed: error_count == 0,
        error_count,
        warning_count,
        info_count,
        issues,
    }
}

/// 標準 IEEE 802.3 CRC-32（ZIP アーカイブ用・Pure Rust）
pub fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// アーカイブ ZIP パッケージ化の結果サマリー
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchivePackageSummary {
    pub zip_path: PathBuf,
    pub file_count: usize,
    pub total_bytes: u64,
}

/// プロジェクトフォルダ（`project.json`, `assets/`, `luts/`, `presets/`, `templates/`, `aliases/`）を
/// Pure-Rust・ゼロ外部依存で標準 ZIP ファイルへパッケージ化する（`cache/`・`exports/` は肥大化防止のため除外）。
pub fn archive_project_to_zip(
    folder: &ProjectFolder,
    output_zip: &Path,
) -> Result<ArchivePackageSummary, std::io::Error> {
    if let Some(parent) = output_zip.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    let pj = folder.project_json_path();
    if pj.is_file() {
        entries.push(("project.json".to_string(), pj));
    }
    let include_dirs = ["assets", "luts", "presets", "templates", "aliases"];
    for dir_name in include_dirs {
        let dir = folder.root().join(dir_name);
        if dir.is_dir() {
            collect_files_recursive(folder.root(), &dir, &mut entries)?;
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut zip_bytes = Vec::new();
    let mut central_dir = Vec::new();
    let mut total_uncompressed = 0u64;

    for (rel_name, abs_path) in &entries {
        let data = fs::read(abs_path)?;
        let crc = crc32_ieee(&data);
        let size = data.len() as u32;
        total_uncompressed += data.len() as u64;
        let name_bytes = rel_name.as_bytes();
        let name_len = name_bytes.len() as u16;
        let local_offset = zip_bytes.len() as u32;

        // Local File Header (PK\x03\x04)
        zip_bytes.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        zip_bytes.extend_from_slice(&20u16.to_le_bytes()); // version needed 2.0
        zip_bytes.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 filename flag
        zip_bytes.extend_from_slice(&0u16.to_le_bytes()); // method 0 = Store
        zip_bytes.extend_from_slice(&0u16.to_le_bytes()); // mod time
        zip_bytes.extend_from_slice(&0x5421u16.to_le_bytes()); // mod date
        zip_bytes.extend_from_slice(&crc.to_le_bytes());
        zip_bytes.extend_from_slice(&size.to_le_bytes());
        zip_bytes.extend_from_slice(&size.to_le_bytes());
        zip_bytes.extend_from_slice(&name_len.to_le_bytes());
        zip_bytes.extend_from_slice(&0u16.to_le_bytes()); // extra len
        zip_bytes.extend_from_slice(name_bytes);
        zip_bytes.extend_from_slice(&data);

        // Central Directory Header (PK\x01\x02)
        central_dir.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central_dir.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central_dir.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central_dir.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 flag
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // Store
        central_dir.extend_from_slice(&0u16.to_le_bytes());
        central_dir.extend_from_slice(&0x5421u16.to_le_bytes());
        central_dir.extend_from_slice(&crc.to_le_bytes());
        central_dir.extend_from_slice(&size.to_le_bytes());
        central_dir.extend_from_slice(&size.to_le_bytes());
        central_dir.extend_from_slice(&name_len.to_le_bytes());
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // extra len
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // comment len
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // disk start
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central_dir.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central_dir.extend_from_slice(&local_offset.to_le_bytes());
        central_dir.extend_from_slice(name_bytes);
    }

    let cd_offset = zip_bytes.len() as u32;
    let cd_size = central_dir.len() as u32;
    let count = entries.len() as u16;
    zip_bytes.extend_from_slice(&central_dir);

    // End of Central Directory Record (PK\x05\x06)
    zip_bytes.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    zip_bytes.extend_from_slice(&0u16.to_le_bytes());
    zip_bytes.extend_from_slice(&0u16.to_le_bytes());
    zip_bytes.extend_from_slice(&count.to_le_bytes());
    zip_bytes.extend_from_slice(&count.to_le_bytes());
    zip_bytes.extend_from_slice(&cd_size.to_le_bytes());
    zip_bytes.extend_from_slice(&cd_offset.to_le_bytes());
    zip_bytes.extend_from_slice(&0u16.to_le_bytes());

    let mut file = fs::File::create(output_zip)?;
    file.write_all(&zip_bytes)?;
    file.sync_all()?;

    Ok(ArchivePackageSummary {
        zip_path: output_zip.to_path_buf(),
        file_count: entries.len(),
        total_bytes: total_uncompressed,
    })
}

fn collect_files_recursive(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), std::io::Error> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(root, &path, out)?;
        } else if path.is_file()
            && let Ok(rel) = path.strip_prefix(root)
        {
            let rel_str = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push((rel_str, path));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{
        AssetRef, MediaKind, ProjectSettings, RgbaColor, ShapeKind, TimelineItem,
    };
    use crate::time::RationalTime;
    use tempfile::tempdir;
    use uuid::Uuid;

    #[test]
    fn validates_offline_assets_clipping_overlaps_and_bad_expressions() {
        let mut proj = Project::new("DeliveryCheck", ProjectSettings::default());
        proj.assets.push(AssetRef {
            id: Uuid::new_v4(),
            name: "missing.mp4".into(),
            kind: MediaKind::Video,
            relative_path: Some(PathBuf::from("assets/missing.mp4")),
            fallback_absolute_path: None,
            collected: false,
        });
        let s = proj.active_scene_mut().unwrap();
        s.target_lufs = Some(-6.0);
        let mut a = TimelineItem::new(
            "A",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 100.0,
                height: 100.0,
                color: RgbaColor::WHITE,
            },
        );
        a.expressions.insert("x".into(), "1 + * bad".into());
        let b = TimelineItem::new(
            "B",
            RationalTime::from_seconds(1.5),
            RationalTime::from_seconds(2.0),
            ItemKind::Shape {
                shape: ShapeKind::Ellipse,
                width: 100.0,
                height: 100.0,
                color: RgbaColor::WHITE,
            },
        );
        s.layers[0].items.extend([a, b]);

        let report = validate_project_delivery(&proj, None);
        assert!(!report.passed);
        assert!(report.error_count >= 2);
        assert!(report.warning_count >= 2);
    }

    #[test]
    fn archives_project_folder_to_valid_pkzip_with_crc32() {
        assert_eq!(crc32_ieee(b"123456789"), 0xCBF4_3926);

        let tmp = tempdir().unwrap();
        let proj = Project::new("ZipTest", ProjectSettings::default());
        let folder = ProjectFolder::init(tmp.path().join("proj"), &proj).unwrap();
        fs::write(folder.assets_dir().join("sample.txt"), b"hello video").unwrap();
        // cache は除外されることを確認
        fs::write(folder.cache_precomp_dir().join("temp.bin"), b"ignored").unwrap();

        let out_zip = tmp.path().join("archive.zip");
        let summary = archive_project_to_zip(&folder, &out_zip).unwrap();
        assert_eq!(summary.file_count, 2);
        let bytes = fs::read(&out_zip).unwrap();
        assert_eq!(&bytes[0..4], b"PK\x03\x04");
        assert!(bytes.windows(12).any(|w| w == b"project.json"));
        assert!(bytes.windows(17).any(|w| w == b"assets/sample.txt"));
        assert!(!bytes.windows(8).any(|w| w == b"temp.bin"));
    }
}
