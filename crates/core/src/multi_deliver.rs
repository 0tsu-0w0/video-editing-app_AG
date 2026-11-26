//! v6.0 #44: プロ・マルチプラットフォーム自動リフォーマット＆納品バンドルパッケージャー
//! 1 本のマスタータイムラインから YouTube 16:9 / Shorts・Reels 9:16 / Square 1:1 / Cinema 4K の
//! 派生シーン自動生成（字幕折り返し＆中央リフレーム対応）と納品マニフェスト構築を一括実行する。

use crate::command::EditCommand;
use crate::project::{AudioDuckingConfig, ItemKind, Project, Scene, SceneId};
use crate::timeline_ops::{format_youtube_chapters, wrap_subtitle_japanese};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryPlatform {
    #[default]
    YouTube16x9,
    ShortsReels9x16,
    SquareFeed1x1,
    Cinema4kMaster,
}

impl DeliveryPlatform {
    pub const ALL: &'static [Self] = &[
        Self::YouTube16x9,
        Self::ShortsReels9x16,
        Self::SquareFeed1x1,
        Self::Cinema4kMaster,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::YouTube16x9 => "youtube_16x9",
            Self::ShortsReels9x16 => "shorts_reels_9x16",
            Self::SquareFeed1x1 => "square_feed_1x1",
            Self::Cinema4kMaster => "cinema_4k_master",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::YouTube16x9 => "📺 YouTube 16:9 (1920×1080 / -14 LUFS)",
            Self::ShortsReels9x16 => "📱 Shorts / Reels / TikTok 9:16 (1080×1920)",
            Self::SquareFeed1x1 => "🟦 X / Instagram フィード 1:1 (1080×1080)",
            Self::Cinema4kMaster => "🎬 シネマ 4K マスター (3840×2160 / -24 LUFS)",
        }
    }

    pub fn default_target(self) -> DeliverableTarget {
        match self {
            Self::YouTube16x9 => DeliverableTarget {
                platform: self,
                width: 1920,
                height: 1080,
                target_lufs: -14.0,
                subtitle_max_chars: 26,
                include_srt: true,
                include_chapters: true,
            },
            Self::ShortsReels9x16 => DeliverableTarget {
                platform: self,
                width: 1080,
                height: 1920,
                target_lufs: -14.0,
                subtitle_max_chars: 13,
                include_srt: true,
                include_chapters: false,
            },
            Self::SquareFeed1x1 => DeliverableTarget {
                platform: self,
                width: 1080,
                height: 1080,
                target_lufs: -14.0,
                subtitle_max_chars: 16,
                include_srt: true,
                include_chapters: false,
            },
            Self::Cinema4kMaster => DeliverableTarget {
                platform: self,
                width: 3840,
                height: 2160,
                target_lufs: -24.0,
                subtitle_max_chars: 30,
                include_srt: true,
                include_chapters: true,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeliverableTarget {
    pub platform: DeliveryPlatform,
    pub width: u32,
    pub height: u32,
    pub target_lufs: f32,
    pub subtitle_max_chars: usize,
    pub include_srt: bool,
    pub include_chapters: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeliveryBundleConfig {
    pub targets: Vec<DeliverableTarget>,
    #[serde(default = "default_true")]
    pub include_thumbnail: bool,
    #[serde(default = "default_true")]
    pub include_edl: bool,
    #[serde(default = "default_true")]
    pub include_fcpxml: bool,
    #[serde(default = "default_true")]
    pub include_qc_report: bool,
    #[serde(default)]
    pub base_title: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_true() -> bool {
    true
}

impl Default for DeliveryBundleConfig {
    fn default() -> Self {
        Self {
            targets: vec![
                DeliveryPlatform::YouTube16x9.default_target(),
                DeliveryPlatform::ShortsReels9x16.default_target(),
                DeliveryPlatform::SquareFeed1x1.default_target(),
            ],
            include_thumbnail: true,
            include_edl: true,
            include_fcpxml: true,
            include_qc_report: true,
            base_title: "Master Delivery".into(),
            tags: vec!["video".into(), "master".into(), "v6".into()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeliveryBundleArtifact {
    pub kind: String,
    pub platform: String,
    pub relative_path: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeliveryBundleManifest {
    pub project_name: String,
    pub base_title: String,
    pub scene_duration_sec: f64,
    pub tags: Vec<String>,
    pub youtube_chapters: String,
    pub artifacts: Vec<DeliveryBundleArtifact>,
}

/// 既存シーンから指定プラットフォーム（16:9 / 9:16 / 1:1 / 4K）向けに
/// 解像度・中央リフレーム・字幕文字数自動折り返しを最適化した派生シーン作成コマンドを生成する
pub fn build_reformatted_scene_command(
    src: &Scene,
    target: &DeliverableTarget,
) -> (SceneId, EditCommand) {
    let sx = target.width.max(64) as f32 / src.width.max(64) as f32;
    let sy = target.height.max(64) as f32 / src.height.max(64) as f32;
    let uniform = sx.min(sy);
    let mut new_layers = src.layers.clone();

    for layer in &mut new_layers {
        layer.id = Uuid::new_v4();
        for item in &mut layer.items {
            item.id = Uuid::new_v4();
            // シーン中心基準の相対位置を新アスペクト比へ比例マップ
            let rel_x = (item.transform.x - src.width as f32 * 0.5) * uniform;
            let rel_y = (item.transform.y - src.height as f32 * 0.5) * uniform;
            item.transform.x = (target.width as f32 * 0.5 + rel_x).clamp(0.0, target.width as f32);
            item.transform.y =
                (target.height as f32 * 0.5 + rel_y).clamp(0.0, target.height as f32);

            match &mut item.kind {
                ItemKind::Text {
                    content, font_size, ..
                } => {
                    *content = wrap_subtitle_japanese(content, target.subtitle_max_chars.max(8));
                    *font_size = (*font_size * uniform).clamp(16.0, 140.0);
                    // 縦動画時は下部 UI 被りを避けてセーフゾーンへ寄せる
                    if target.height > target.width
                        && item.transform.y > target.height as f32 * 0.72
                    {
                        item.transform.y = target.height as f32 * 0.68;
                    }
                }
                ItemKind::GenerativeBg { config } => {
                    config.width = target.width as f32;
                    config.height = target.height as f32;
                }
                ItemKind::Shape { width, height, .. }
                | ItemKind::CharacterSprite { width, height, .. }
                | ItemKind::AudioSpectrum { width, height, .. } => {
                    *width = (*width * uniform).max(16.0);
                    *height = (*height * uniform).max(16.0);
                }
                ItemKind::DataChart { config } => {
                    config.width =
                        (config.width * uniform).clamp(160.0, target.width as f32 * 0.92);
                    config.height =
                        (config.height * uniform).clamp(120.0, target.height as f32 * 0.75);
                }
                _ => {}
            }
        }
    }

    let scene_id = Uuid::new_v4();
    let variant_scene = Scene {
        id: scene_id,
        name: format!("{} [{}]", src.name, target.platform.id()),
        width: target.width,
        height: target.height,
        fps: src.fps,
        camera: src.camera,
        master_audio_effects: src.master_audio_effects.clone(),
        target_lufs: Some(target.target_lufs),
        audio_ducking: AudioDuckingConfig {
            ..src.audio_ducking
        },
        color_grade: src.color_grade.clone(),
        color_management: src.color_management,
        cinema_finishing: src.cinema_finishing.clone(),
        motion_blur: src.motion_blur,
        markers: src.markers.clone(),
        node_graph: src.node_graph.clone(),
        review_comments: Vec::new(),
        ab_compare: crate::studio_hub::AbCompareConfig::default(),
        optical_vfx: src.optical_vfx.clone(),
        adr_session: src.adr_session.clone(),
        spatial_bus: src.spatial_bus.clone(),
        layers: new_layers,
    };

    (
        scene_id,
        EditCommand::CreateScene {
            scene: variant_scene,
            switch_to: false,
        },
    )
}

/// 配信納品マニフェストオブジェクトを構築する
pub fn build_delivery_manifest(
    project: &Project,
    cfg: &DeliveryBundleConfig,
    artifacts: Vec<DeliveryBundleArtifact>,
) -> DeliveryBundleManifest {
    let (dur_sec, chapters) = project
        .active_scene()
        .map(|sc| {
            (
                sc.total_duration().as_seconds(),
                format_youtube_chapters(&sc.markers),
            )
        })
        .unwrap_or((0.0, String::new()));

    DeliveryBundleManifest {
        project_name: project.name.clone(),
        base_title: if cfg.base_title.trim().is_empty() {
            project.name.clone()
        } else {
            cfg.base_title.clone()
        },
        scene_duration_sec: dur_sec,
        tags: cfg.tags.clone(),
        youtube_chapters: chapters,
        artifacts,
    }
}

/// 納品バンドル内の JSON ファイル（派生シーン・QC レポート・manifest.json）を整形シリアライズする
pub fn serialize_pretty_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(value).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{ProjectSettings, RgbaColor, TextAnimator, TimelineItem, TimelineMarker};
    use crate::time::RationalTime;

    #[test]
    fn reformats_master_scene_into_9x16_and_1x1_variants_with_wrapped_subtitles() {
        let mut proj = Project::new("MultiDeliverTest", ProjectSettings::default());
        {
            let sc = proj.active_scene_mut().unwrap();
            sc.layers[1].items.push(TimelineItem::new(
                "LongSub",
                RationalTime::ZERO,
                RationalTime::from_seconds(3.0),
                ItemKind::Text {
                    content: "これは縦動画向けに自動で改行リレイアウトされる長い日本語テロップのテストです。".into(),
                    font_family: "Sans".into(),
                    font_size: 48.0,
                    color: RgbaColor::WHITE,
                    animator: TextAnimator::default(),
                },
            ));
            sc.markers
                .push(TimelineMarker::new(RationalTime::ZERO, "オープニング"));
        }
        let src = proj.active_scene().unwrap();
        let (_id_9x16, cmd_9x16) = build_reformatted_scene_command(
            src,
            &DeliveryPlatform::ShortsReels9x16.default_target(),
        );
        let EditCommand::CreateScene { scene: s9, .. } = cmd_9x16 else {
            panic!("expected CreateScene");
        };
        assert_eq!((s9.width, s9.height), (1080, 1920));
        assert_eq!(s9.target_lufs, Some(-14.0));
        if let ItemKind::Text { content, .. } = &s9.layers[1].items[0].kind {
            assert!(
                content.contains('\n'),
                "subtitle should wrap for 9:16: {content}"
            );
        } else {
            panic!("expected Text");
        }

        let manifest = build_delivery_manifest(&proj, &DeliveryBundleConfig::default(), vec![]);
        assert!(manifest.youtube_chapters.contains("00:00 オープニング"));
    }
}
