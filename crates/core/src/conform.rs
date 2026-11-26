//! v7.0 #48: プロキシ ⇔ カメラ原本 SMPTE タイムコード＆リール名自動コンフォームエンジン
//!
//! オフライン軽量プロキシ素材（例: `A001_C012_proxy.mp4`, 開始 TC `01:14:22:00`）で粗編集したタイムラインを、
//! カメラ原本の高解像度マスター素材（例: `A001_C012_4k_master.mov`, 開始 TC `01:14:20:00`）へ
//! SMPTE 開始タイムコード差分を考慮して 1-Undo (`EditCommand::Batch`) で厳密に自動差し替え（逆方向ラウンドトリップ対応）する。

use crate::collab::{format_smpte_timecode, parse_timecode_to_rational};
use crate::command::EditCommand;
use crate::project::{AssetId, AssetRef, ItemId, ItemKind, MediaKind, Project, SceneId};
use crate::time::{FrameRate, RationalTime};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// アセットごとのリール名・SMPTE 開始タイムコード・プロキシ識別メタデータ
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetConformMeta {
    /// カメラリール名 / テープ名（未設定時はファイル名ステムから自動正規化）
    #[serde(default)]
    pub reel_name: Option<String>,
    /// 素材先頭フレームの SMPTE タイムコード（例: `"01:00:00:00"`、未設定時は `"00:00:00:00"` 扱い）
    #[serde(default)]
    pub start_timecode: Option<String>,
    /// プロキシメディアであるかの明示フラグ
    #[serde(default)]
    pub is_proxy_media: bool,
    /// 明示的にペアリングされた対応アセット（プロキシなら原本 ID、原本ならプロキシ ID）
    #[serde(default)]
    pub linked_counterpart_id: Option<AssetId>,
}

/// コンフォーム方向
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConformDirection {
    /// プロキシ素材 → 高解像度カメラ原本（オンライン仕上げ・グレーディング前コンフォーム）
    ProxyToOriginal,
    /// 高解像度カメラ原本 → 軽量プロキシ素材（モバイルオフライン編集への切り戻し）
    OriginalToProxy,
}

impl ConformDirection {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::ProxyToOriginal => "プロキシ → カメラ原本 (Online Master Conform)",
            Self::OriginalToProxy => "カメラ原本 → 軽量プロキシ (Offline Proxy Switch)",
        }
    }
}

const PROXY_SUFFIXES: &[&str] = &[
    "_proxy", "-proxy", ".proxy", "_prx", "-prx", "_lrf", "_low", "_offline", "_540p", "_720p",
    "_360p", "_sub",
];

const ORIGINAL_SUFFIXES: &[&str] = &[
    "_master",
    "-master",
    "_orig",
    "-orig",
    "_original",
    "_raw",
    "-raw",
    "_online",
    "_4k",
    "_8k",
    "_uhd",
    "_prores",
    "_braw",
];

/// ファイル名またはパスからプロキシかどうかを推定する（メタデータ未設定時の自動判定補助）
pub fn infer_is_proxy_name(name_or_path: &str) -> bool {
    let stem = Path::new(name_or_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| name_or_path.to_lowercase());
    PROXY_SUFFIXES.iter().any(|suf| stem.ends_with(suf)) || stem.contains("_proxy_")
}

/// ファイル名またはパスから `_proxy` / `_master` / `_4k` 等のサフィックスを除去して正規化リール識別子を抽出する
pub fn normalize_reel_stem(name_or_path: &str) -> String {
    let mut stem = Path::new(name_or_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| name_or_path.trim().to_lowercase());

    // 複数サフィックスの連鎖（例: `A001_C012_4k_master` や `A001_C012_720p_proxy`）を反復除去
    loop {
        let mut stripped = false;
        for suf in PROXY_SUFFIXES.iter().chain(ORIGINAL_SUFFIXES.iter()) {
            if let Some(prefix) = stem.strip_suffix(suf)
                && !prefix.is_empty()
            {
                stem = prefix.to_string();
                stripped = true;
            }
        }
        if !stripped {
            break;
        }
    }
    stem.trim_matches(|c: char| c == '_' || c == '-' || c == ' ')
        .to_string()
}

fn effective_reel_key(asset: &AssetRef, meta: Option<&AssetConformMeta>) -> String {
    if let Some(r) = meta.and_then(|m| m.reel_name.as_deref()) {
        let trimmed = r.trim();
        if !trimmed.is_empty() {
            return normalize_reel_stem(trimmed);
        }
    }
    normalize_reel_stem(&asset.name)
}

fn effective_is_proxy(asset: &AssetRef, meta: Option<&AssetConformMeta>) -> bool {
    match meta {
        Some(m) => m.is_proxy_media || infer_is_proxy_name(&asset.name),
        None => infer_is_proxy_name(&asset.name),
    }
}

fn effective_start_tc(meta: Option<&AssetConformMeta>, fps: FrameRate) -> RationalTime {
    meta.and_then(|m| m.start_timecode.as_deref())
        .and_then(|tc| parse_timecode_to_rational(tc, fps))
        .unwrap_or(RationalTime::ZERO)
}

/// コンフォーム計画の 1 クリップ変換内容
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConformClipAction {
    pub item_id: ItemId,
    pub item_name: String,
    pub layer_index: usize,
    pub from_asset_id: AssetId,
    pub from_asset_name: String,
    pub to_asset_id: AssetId,
    pub to_asset_name: String,
    pub reel_key: String,
    pub clip_source_tc_in: String,
    pub old_source_offset: RationalTime,
    pub new_source_offset: RationalTime,
    pub handle_clamped: bool,
}

/// コンフォーム時の警告・未解決事項
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConformIssue {
    pub item_id: ItemId,
    pub item_name: String,
    pub message: String,
}

/// コンフォームドライラン／実行レポート
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConformReport {
    pub direction: ConformDirection,
    pub actions: Vec<ConformClipAction>,
    pub issues: Vec<ConformIssue>,
    pub skipped_already_target: usize,
}

/// シーン内の Video / Audio クリップを走査し、リール名＆SMPTE 開始タイムコードに基づくコンフォーム計画を作成する
pub fn plan_timeline_conform(
    project: &Project,
    scene_id: SceneId,
    direction: ConformDirection,
) -> ConformReport {
    let Some(scene) = project.scene(scene_id) else {
        return ConformReport {
            direction,
            actions: Vec::new(),
            issues: Vec::new(),
            skipped_already_target: 0,
        };
    };
    let fps = scene.fps;
    let want_target_proxy = direction == ConformDirection::OriginalToProxy;

    let mut actions = Vec::new();
    let mut issues = Vec::new();
    let mut skipped_already_target = 0usize;

    for (layer_idx, layer) in scene.layers.iter().enumerate() {
        for item in &layer.items {
            let (cur_asset_id, cur_offset) = match &item.kind {
                ItemKind::Video {
                    asset_id,
                    source_offset,
                    ..
                } => (*asset_id, *source_offset),
                ItemKind::Audio {
                    asset_id,
                    source_offset,
                    ..
                } => (*asset_id, *source_offset),
                _ => continue,
            };

            let Some(cur_asset) = project.asset(cur_asset_id) else {
                issues.push(ConformIssue {
                    item_id: item.id,
                    item_name: item.name.clone(),
                    message: format!(
                        "参照アセット {cur_asset_id} がプロジェクト内に見つかりません"
                    ),
                });
                continue;
            };
            if !matches!(cur_asset.kind, MediaKind::Video | MediaKind::Audio) {
                continue;
            }

            let cur_meta = project.conform_meta.get(&cur_asset_id);
            let cur_is_proxy = effective_is_proxy(cur_asset, cur_meta);
            if cur_is_proxy == want_target_proxy {
                skipped_already_target += 1;
                continue;
            }

            let cur_reel = effective_reel_key(cur_asset, cur_meta);
            let cur_start_tc = effective_start_tc(cur_meta, fps);
            let absolute_clip_in_tc = cur_start_tc + cur_offset;

            // 1) 明示リンク (`linked_counterpart_id`) 優先、2) 正規化リール名＆メディア種別＆プロキシ属性一致で検索
            let explicit_cand = cur_meta
                .and_then(|m| m.linked_counterpart_id)
                .and_then(|cid| project.asset(cid));

            let matched_target = explicit_cand.or_else(|| {
                project
                    .assets
                    .iter()
                    .filter(|cand| cand.id != cur_asset_id && cand.kind == cur_asset.kind)
                    .find(|cand| {
                        let cmeta = project.conform_meta.get(&cand.id);
                        effective_is_proxy(cand, cmeta) == want_target_proxy
                            && !cur_reel.is_empty()
                            && effective_reel_key(cand, cmeta) == cur_reel
                    })
            });

            let Some(target_asset) = matched_target else {
                issues.push(ConformIssue {
                    item_id: item.id,
                    item_name: item.name.clone(),
                    message: format!(
                        "リール '{cur_reel}' ({}) に対応する{}アセットが見つかりません",
                        cur_asset.name,
                        if want_target_proxy {
                            "プロキシ"
                        } else {
                            "原本マスター"
                        }
                    ),
                });
                continue;
            };

            let target_meta = project.conform_meta.get(&target_asset.id);
            let target_start_tc = effective_start_tc(target_meta, fps);
            let raw_new_offset = absolute_clip_in_tc - target_start_tc;
            let (new_source_offset, handle_clamped) = if raw_new_offset < RationalTime::ZERO {
                issues.push(ConformIssue {
                    item_id: item.id,
                    item_name: item.name.clone(),
                    message: format!(
                        "クリップ開始 TC ({}) が切替先 '{}' の開始 TC ({}) より {:.3}s 早いため 0s にクランプしました",
                        format_smpte_timecode(absolute_clip_in_tc, fps),
                        target_asset.name,
                        format_smpte_timecode(target_start_tc, fps),
                        (-raw_new_offset.as_seconds())
                    ),
                });
                (RationalTime::ZERO, true)
            } else {
                (raw_new_offset, false)
            };

            actions.push(ConformClipAction {
                item_id: item.id,
                item_name: item.name.clone(),
                layer_index: layer_idx,
                from_asset_id: cur_asset_id,
                from_asset_name: cur_asset.name.clone(),
                to_asset_id: target_asset.id,
                to_asset_name: target_asset.name.clone(),
                reel_key: cur_reel,
                clip_source_tc_in: format_smpte_timecode(absolute_clip_in_tc, fps),
                old_source_offset: cur_offset,
                new_source_offset,
                handle_clamped,
            });
        }
    }

    ConformReport {
        direction,
        actions,
        issues,
        skipped_already_target,
    }
}

/// コンフォーム計画から 1 ステップ Undo 可能な `EditCommand::Batch` を生成する
pub fn build_timeline_conform_command(report: &ConformReport) -> Option<EditCommand> {
    if report.actions.is_empty() {
        return None;
    }
    let cmds: Vec<EditCommand> = report
        .actions
        .iter()
        .map(|act| EditCommand::ConformClipSource {
            item_id: act.item_id,
            new_asset_id: act.to_asset_id,
            new_source_offset: act.new_source_offset,
        })
        .collect();
    Some(EditCommand::Batch {
        label: match report.direction {
            ConformDirection::ProxyToOriginal => "Conform Proxies to Camera Originals".into(),
            ConformDirection::OriginalToProxy => "Conform Camera Originals to Proxies".into(),
        },
        commands: cmds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandBus, CommandSource};
    use crate::project::{ProjectSettings, TimelineItem};
    use uuid::Uuid;

    #[test]
    fn reel_stem_normalization_strips_proxy_and_master_suffixes() {
        assert_eq!(normalize_reel_stem("A001_C012_720p_proxy.mp4"), "a001_c012");
        assert_eq!(
            normalize_reel_stem("/footage/A001_C012_4k_master.mov"),
            "a001_c012"
        );
        assert!(infer_is_proxy_name("CamB_Take03_proxy.mp4"));
        assert!(!infer_is_proxy_name("CamB_Take03_4k_master.mov"));
    }

    #[test]
    fn proxy_to_original_timecode_conform_and_roundtrip_undo_are_exact() {
        let settings = ProjectSettings {
            fps: FrameRate::FPS_24,
            ..Default::default()
        };
        let mut proj = Project::new("ConformFilm", settings);
        let scene_id = proj.active_scene_id;

        let proxy_id = Uuid::new_v4();
        let orig_id = Uuid::new_v4();
        proj.assets.push(AssetRef {
            id: proxy_id,
            name: "A002_C009_proxy.mp4".into(),
            kind: MediaKind::Video,
            relative_path: None,
            fallback_absolute_path: None,
            collected: false,
        });
        proj.assets.push(AssetRef {
            id: orig_id,
            name: "A002_C009_4k_master.mov".into(),
            kind: MediaKind::Video,
            relative_path: None,
            fallback_absolute_path: None,
            collected: false,
        });

        // プロキシは 01:00:10:00 開始、カメラ原本は前後ハンドル込みで 01:00:00:00 開始
        proj.conform_meta.insert(
            proxy_id,
            AssetConformMeta {
                reel_name: Some("A002_C009".into()),
                start_timecode: Some("01:00:10:00".into()),
                is_proxy_media: true,
                linked_counterpart_id: None,
            },
        );
        proj.conform_meta.insert(
            orig_id,
            AssetConformMeta {
                reel_name: Some("A002_C009".into()),
                start_timecode: Some("01:00:00:00".into()),
                is_proxy_media: false,
                linked_counterpart_id: None,
            },
        );

        // プロキシ先頭から +5.0s (= カメラ絶対 TC 01:00:15:00) を使っているクリップ
        let clip = TimelineItem::new(
            "Hero Cut",
            RationalTime::ZERO,
            RationalTime::from_seconds(4.0),
            ItemKind::Video {
                asset_id: proxy_id,
                source_offset: RationalTime::from_seconds(5.0),
                volume: 1.0,
            },
        );
        let clip_id = clip.id;
        proj.active_scene_mut().unwrap().layers[0].items.push(clip);

        // 1) Proxy -> Original: カメラ原本は 01:00:00:00 開始なので new_source_offset = 15.0s になるはず！
        let plan_online = plan_timeline_conform(&proj, scene_id, ConformDirection::ProxyToOriginal);
        assert_eq!(plan_online.actions.len(), 1);
        assert!(plan_online.issues.is_empty());
        assert_eq!(plan_online.actions[0].to_asset_id, orig_id);
        assert_eq!(
            plan_online.actions[0].new_source_offset,
            RationalTime::from_seconds(15.0)
        );
        assert_eq!(plan_online.actions[0].clip_source_tc_in, "01:00:15:00");

        let mut bus = CommandBus::new();
        let cmd = build_timeline_conform_command(&plan_online).unwrap();
        bus.execute(&mut proj, cmd, CommandSource::Gui, None)
            .unwrap();

        if let ItemKind::Video {
            asset_id,
            source_offset,
            ..
        } = &proj.active_scene().unwrap().layers[0].items[0].kind
        {
            assert_eq!(*asset_id, orig_id);
            assert_eq!(*source_offset, RationalTime::from_seconds(15.0));
        } else {
            panic!("expected video item");
        }

        // 2) Original -> Proxy 逆コンフォームで元の +5.0s へ完全一致復帰すること
        let plan_offline =
            plan_timeline_conform(&proj, scene_id, ConformDirection::OriginalToProxy);
        assert_eq!(plan_offline.actions.len(), 1);
        assert_eq!(plan_offline.actions[0].to_asset_id, proxy_id);
        assert_eq!(
            plan_offline.actions[0].new_source_offset,
            RationalTime::from_seconds(5.0)
        );

        // 3) 1-Undo でも元のプロキシ +5.0s に戻ること
        assert!(bus.undo(&mut proj));
        let restored = &proj.active_scene().unwrap().layers[0].items[0];
        assert_eq!(restored.id, clip_id);
        if let ItemKind::Video {
            asset_id,
            source_offset,
            ..
        } = &restored.kind
        {
            assert_eq!(*asset_id, proxy_id);
            assert_eq!(*source_offset, RationalTime::from_seconds(5.0));
        }
    }
}
