//! v9.0 #53: ライブスイッチャー ＆ NLE 同時収録（RTMP / SRT / スティンガー遷移 / `.veproj` 同時生成）
//!
//! Pure Rust・ゼロ `unsafe`・外部追加依存ゼロで放送品質の 2 バス（PGM / PVW）ライブスイッチングと
//! 即座に後編集へ引き継げる `.veproj` タイムライン同時生成を実現する。

use crate::animation::{Interpolation, Keyframe};
use crate::command::{CommandBus, CommandSource, EditCommand};
use crate::project::{
    AssetId, FilterTargetScope, ItemKind, Project, RgbaColor, TimelineItem, TimelineMarker,
    TransitionKind,
};
use crate::time::RationalTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// ライブ配信出力プロトコル
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamProtocol {
    #[default]
    LocalRecordOnly,
    Rtmp,
    Rtmps,
    Srt,
}

impl StreamProtocol {
    pub const ALL: &'static [Self] = &[Self::LocalRecordOnly, Self::Rtmp, Self::Rtmps, Self::Srt];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::LocalRecordOnly => "💾 ローカル ISO 同時収録のみ",
            Self::Rtmp => "📡 RTMP (YouTube / Twitch)",
            Self::Rtmps => "🔒 RTMPS (TLS 暗号化配信)",
            Self::Srt => "⚡ SRT (低遅延・耐パケットロス)",
        }
    }
}

fn default_video_bitrate() -> u32 {
    6_000
}
fn default_audio_bitrate() -> u32 {
    192
}
fn default_stream_w() -> u32 {
    1920
}
fn default_stream_h() -> u32 {
    1080
}
fn default_stream_fps() -> u32 {
    30
}
fn default_srt_latency() -> u32 {
    120
}

/// RTMP / SRT 配信先およびローカル同時収録設定
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamTargetConfig {
    #[serde(default)]
    pub protocol: StreamProtocol,
    /// 例: `rtmp://a.rtmp.youtube.com/live2` または `srt://127.0.0.1:9000`
    #[serde(default)]
    pub endpoint_url: String,
    /// セキュリティ保護のため `.veproj` にはマスク済みラベルのみ保存（生キーはメモリのみ）
    #[serde(default)]
    pub stream_key_hint: String,
    #[serde(default = "default_video_bitrate")]
    pub video_bitrate_kbps: u32,
    #[serde(default = "default_audio_bitrate")]
    pub audio_bitrate_kbps: u32,
    #[serde(default = "default_stream_w")]
    pub width: u32,
    #[serde(default = "default_stream_h")]
    pub height: u32,
    #[serde(default = "default_stream_fps")]
    pub fps: u32,
    /// SRT レイテンシ (ms)
    #[serde(default = "default_srt_latency")]
    pub srt_latency_ms: u32,
    /// ローカル同時収録ファイル出力先（空ならプロジェクト `exports/` フォルダ）
    #[serde(default)]
    pub iso_record_dir: String,
}

impl Default for StreamTargetConfig {
    fn default() -> Self {
        Self {
            protocol: StreamProtocol::LocalRecordOnly,
            endpoint_url: "rtmp://127.0.0.1:1935/live".to_string(),
            stream_key_hint: "****".to_string(),
            video_bitrate_kbps: default_video_bitrate(),
            audio_bitrate_kbps: default_audio_bitrate(),
            width: default_stream_w(),
            height: default_stream_h(),
            fps: default_stream_fps(),
            srt_latency_ms: default_srt_latency(),
            iso_record_dir: String::new(),
        }
    }
}

/// スティンガー／トランジション演出種別
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StingerKind {
    Cut,
    CrossDissolve,
    #[default]
    DiagonalWipe,
    SlashFlash,
    LumaSwipe,
    ZoomBurst,
}

impl StingerKind {
    pub const ALL: &'static [Self] = &[
        Self::Cut,
        Self::CrossDissolve,
        Self::DiagonalWipe,
        Self::SlashFlash,
        Self::LumaSwipe,
        Self::ZoomBurst,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Cut => "✂ ハードカット (0ms)",
            Self::CrossDissolve => "🌫 クロスディゾルブ",
            Self::DiagonalWipe => "⚡ 斜めスティンガーワイプ",
            Self::SlashFlash => "✨ フラッシュスラッシュ遷移",
            Self::LumaSwipe => "🌊 ルマスイープ帯",
            Self::ZoomBurst => "💥 ズームバースト切替",
        }
    }

    pub fn transition_kind(self) -> TransitionKind {
        match self {
            Self::Cut | Self::CrossDissolve => TransitionKind::CrossFade,
            Self::DiagonalWipe => TransitionKind::WipeRight,
            Self::SlashFlash => TransitionKind::Flash,
            Self::LumaSwipe => TransitionKind::SlideLeft,
            Self::ZoomBurst => TransitionKind::RandomLines,
        }
    }
}

fn default_stinger_dur() -> f32 {
    0.55
}
fn default_cut_ratio() -> f32 {
    0.5
}
fn default_stinger_color() -> RgbaColor {
    RgbaColor {
        r: 0.12,
        g: 0.78,
        b: 1.0,
        a: 1.0,
    }
}

/// アルファ付きスティンガートランジション設定
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StingerConfig {
    #[serde(default)]
    pub kind: StingerKind,
    #[serde(default = "default_stinger_dur")]
    pub duration_sec: f32,
    #[serde(default = "default_stinger_color")]
    pub accent_color: RgbaColor,
    /// PGM と PVW を裏側で切り替える進行点 (`0.2..=0.8`, 既定 `0.5`)
    #[serde(default = "default_cut_ratio")]
    pub cut_point_ratio: f32,
}

impl Default for StingerConfig {
    fn default() -> Self {
        Self {
            kind: StingerKind::DiagonalWipe,
            duration_sec: default_stinger_dur(),
            accent_color: default_stinger_color(),
            cut_point_ratio: default_cut_ratio(),
        }
    }
}

/// ライブスイッチャー 1 系統の入力ソース（カメラ／画面共有／VTuber 等）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveSourceChannel {
    pub index: usize,
    pub label: String,
    /// 対応するシーン内レイヤー番号
    pub layer_index: usize,
    /// ISO 個別収録素材アセット ID（収録終了時に自動紐づけ）
    #[serde(default)]
    pub iso_asset_id: Option<AssetId>,
    /// カメラ切替時に音声も追従させるか（AFV: Audio Follow Video）
    #[serde(default)]
    pub audio_follow_video: bool,
    pub tally_color: RgbaColor,
}

/// 1 回のライブスイッチング記録イベント（`.veproj` NLE タイムライン復元用ジャーナル）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveSwitchEvent {
    pub id: Uuid,
    pub timestamp_sec: f64,
    pub from_channel: usize,
    pub to_channel: usize,
    pub stinger: StingerConfig,
    #[serde(default)]
    pub note: String,
}

fn default_live_sources() -> Vec<LiveSourceChannel> {
    vec![
        LiveSourceChannel {
            index: 0,
            label: "CAM 1 (メイン)".into(),
            layer_index: 0,
            iso_asset_id: None,
            audio_follow_video: false,
            tally_color: RgbaColor {
                r: 0.92,
                g: 0.25,
                b: 0.25,
                a: 1.0,
            },
        },
        LiveSourceChannel {
            index: 1,
            label: "CAM 2 (サブ/手元)".into(),
            layer_index: 1,
            iso_asset_id: None,
            audio_follow_video: false,
            tally_color: RgbaColor {
                r: 0.22,
                g: 0.78,
                b: 0.42,
                a: 1.0,
            },
        },
        LiveSourceChannel {
            index: 2,
            label: "CAM 3 (画面共有)".into(),
            layer_index: 2,
            iso_asset_id: None,
            audio_follow_video: false,
            tally_color: RgbaColor {
                r: 0.24,
                g: 0.62,
                b: 0.98,
                a: 1.0,
            },
        },
        LiveSourceChannel {
            index: 3,
            label: "CAM 4 (VTuber)".into(),
            layer_index: 3,
            iso_asset_id: None,
            audio_follow_video: true,
            tally_color: RgbaColor {
                r: 0.88,
                g: 0.36,
                b: 0.92,
                a: 1.0,
            },
        },
    ]
}

fn default_true() -> bool {
    true
}

/// シーン単位のプログラム／プレビュー 2 バス ライブスイッチャー状態
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveSwitcherConfig {
    #[serde(default)]
    pub enabled: bool,
    /// 現在 ON-AIR 中のプログラム系統（赤タリー）
    #[serde(default)]
    pub program_channel: usize,
    /// 次待機中のプレビュー系統（緑タリー）
    #[serde(default = "default_preview_ch")]
    pub preview_channel: usize,
    /// スティンガー進行度（`0.0`=通常、`(0.0..1.0)`=遷移描画中）
    #[serde(default)]
    pub transition_progress: f32,
    #[serde(default)]
    pub stinger: StingerConfig,
    #[serde(default = "default_live_sources")]
    pub sources: Vec<LiveSourceChannel>,
    #[serde(default)]
    pub events: Vec<LiveSwitchEvent>,
    #[serde(default)]
    pub stream_target: StreamTargetConfig,
    /// 同時収録・ジャーナル記録中フラグ
    #[serde(default)]
    pub recording_active: bool,
    /// プレビュー上に PGM / PVW タリー＆ON-AIR HUD を重ねるか
    #[serde(default = "default_true")]
    pub show_tally_hud: bool,
    /// スイッチング時に `.veproj` ジャーナルを自動記録するか
    #[serde(default = "default_true")]
    pub auto_journal_veproj: bool,
}

fn default_preview_ch() -> usize {
    1
}

impl Default for LiveSwitcherConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            program_channel: 0,
            preview_channel: default_preview_ch(),
            transition_progress: 0.0,
            stinger: StingerConfig::default(),
            sources: default_live_sources(),
            events: Vec::new(),
            stream_target: StreamTargetConfig::default(),
            recording_active: false,
            show_tally_hud: true,
            auto_journal_veproj: true,
        }
    }
}

impl LiveSwitcherConfig {
    /// PVW を PGM へ切り替え（CUT または AUTO STINGER）、ジャーナルへイベントを記録する
    pub fn trigger_take(&mut self, timestamp_sec: f64, use_stinger: bool, note: impl Into<String>) {
        if self.sources.is_empty() {
            return;
        }
        let max_idx = self.sources.len() - 1;
        let from_ch = self.program_channel.min(max_idx);
        let to_ch = self.preview_channel.min(max_idx);
        let mut st = self.stinger.clone();
        if !use_stinger {
            st.kind = StingerKind::Cut;
            st.duration_sec = 0.0;
        }
        if self.auto_journal_veproj {
            self.events.push(LiveSwitchEvent {
                id: Uuid::new_v4(),
                timestamp_sec: timestamp_sec.max(0.0),
                from_channel: from_ch,
                to_channel: to_ch,
                stinger: st,
                note: note.into(),
            });
            self.events
                .sort_by(|a, b| a.timestamp_sec.total_cmp(&b.timestamp_sec));
        }
        // スワップ（従来 PGM が次の PVW に待機する放送スイッチャー標準動作）
        self.program_channel = to_ch;
        self.preview_channel = from_ch;
        self.transition_progress = if use_stinger { 0.5 } else { 0.0 };
    }

    /// 指定チャンネルへ直接カット切替する（ハードウェアボタン・MCP 用）
    pub fn cut_to_channel(&mut self, channel: usize, timestamp_sec: f64) {
        if self.sources.is_empty() {
            return;
        }
        let target = channel.min(self.sources.len() - 1);
        self.preview_channel = target;
        self.trigger_take(
            timestamp_sec,
            false,
            format!("Direct Cut -> CH{}", target + 1),
        );
    }
}

/// ライブスイッチング履歴（`events`）から、即座に後編集できる 1-Undo NLE タイムラインコマンド群を生成する
///
/// - 各スイッチ時刻へカラー付き `TimelineMarker` を配置
/// - スティンガー遷移区間へ `ItemKind::Transition` クリップを配置
/// - 対応レイヤー内の先頭クリップへ `opacity` Hold キーフレームを焼き込み、ライブカット割りをそのまま再現
pub fn build_live_switcher_veproj_commands(
    project: &Project,
    cfg: &LiveSwitcherConfig,
    stinger_layer_index: usize,
) -> Option<EditCommand> {
    if cfg.events.is_empty() {
        return None;
    }
    let scene = project.active_scene()?;
    let target_stinger_layer = stinger_layer_index.min(scene.layers.len().saturating_sub(1));
    let mut cmds: Vec<EditCommand> = Vec::new();

    // 1. タイムラインマーカー＆スティンガー遷移クリップ配置
    for (idx, ev) in cfg.events.iter().enumerate() {
        let t = RationalTime::from_seconds(ev.timestamp_sec.max(0.0));
        let to_label = cfg
            .sources
            .get(ev.to_channel)
            .map(|s| s.label.as_str())
            .unwrap_or("CAM");
        let color = cfg
            .sources
            .get(ev.to_channel)
            .map(|s| s.tally_color)
            .unwrap_or(RgbaColor::WHITE);

        cmds.push(EditCommand::AddMarker {
            marker: TimelineMarker {
                id: Uuid::new_v4(),
                time: t,
                label: format!("LIVE #{:02} -> {}", idx + 1, to_label),
                color,
            },
        });

        if ev.stinger.kind != StingerKind::Cut && ev.stinger.duration_sec > 0.05 {
            let half = f64::from(ev.stinger.duration_sec) * 0.5;
            let tr_start = RationalTime::from_seconds((ev.timestamp_sec - half).max(0.0));
            let tr_dur = RationalTime::from_seconds(f64::from(ev.stinger.duration_sec));
            let tr_item = TimelineItem::new(
                format!("Stinger #{} ({})", idx + 1, to_label),
                tr_start,
                tr_dur,
                ItemKind::Transition {
                    transition: ev.stinger.kind.transition_kind(),
                    scope: FilterTargetScope::ImmediateLayer,
                    softness: 0.15,
                },
            );
            cmds.push(EditCommand::AddObject {
                layer_index: target_stinger_layer,
                item: tr_item,
            });
        }
    }

    // 2. 各ソースチャンネル対応レイヤーの先頭クリップへ Opacity Hold キーフレームを適用
    for ch in &cfg.sources {
        let Some(layer) = scene.layers.get(ch.layer_index) else {
            continue;
        };
        let Some(first_item) = layer.items.first() else {
            continue;
        };
        let init_op = if cfg
            .events
            .first()
            .map(|e| e.from_channel == ch.index)
            .unwrap_or(ch.index == cfg.program_channel)
        {
            1.0
        } else {
            0.0
        };
        cmds.push(EditCommand::SetKeyframe {
            item_id: first_item.id,
            property: "opacity".into(),
            keyframe: Keyframe {
                time: RationalTime::ZERO,
                value: init_op,
                interpolation: Interpolation::Hold,
            },
        });
        for ev in &cfg.events {
            let local_t = (RationalTime::from_seconds(ev.timestamp_sec) - first_item.start)
                .max(RationalTime::ZERO);
            let val = if ev.to_channel == ch.index { 1.0 } else { 0.0 };
            cmds.push(EditCommand::SetKeyframe {
                item_id: first_item.id,
                property: "opacity".into(),
                keyframe: Keyframe {
                    time: local_t,
                    value: val,
                    interpolation: Interpolation::Hold,
                },
            });
        }
    }

    Some(EditCommand::Batch {
        label: format!("bake_live_switcher_{}_events", cfg.events.len()),
        commands: cmds,
    })
}

/// ライブ配信終了後に即座に後編集できる自己完結型 `.veproj` JSON を生成する
pub fn export_live_session_veproj_json(
    project: &Project,
    cfg: &LiveSwitcherConfig,
    stinger_layer_index: usize,
) -> Result<String, String> {
    let mut cloned = project.clone();
    let mut bus = CommandBus::new();
    bus.execute(
        &mut cloned,
        EditCommand::SetLiveSwitcher {
            live_switcher: cfg.clone(),
        },
        CommandSource::Gui,
        None,
    )
    .map_err(|e| e.to_string())?;
    if let Some(cmd) = build_live_switcher_veproj_commands(&cloned, cfg, stinger_layer_index) {
        bus.execute(&mut cloned, cmd, CommandSource::Gui, None)
            .map_err(|e| e.to_string())?;
    }
    serde_json::to_string_pretty(&cloned).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{ProjectSettings, ShapeKind};

    #[test]
    fn live_switcher_records_journal_bakes_nle_timeline_and_exports_veproj() {
        let mut proj = Project::new("LiveShow", ProjectSettings::default());
        {
            let scene = proj.active_scene_mut().unwrap();
            for (ch, layer) in scene.layers.iter_mut().enumerate().take(2) {
                layer.items.push(TimelineItem::new(
                    format!("Cam{}", ch + 1),
                    RationalTime::ZERO,
                    RationalTime::from_seconds(20.0),
                    ItemKind::Shape {
                        shape: ShapeKind::Rectangle,
                        width: 640.0,
                        height: 360.0,
                        color: RgbaColor::WHITE,
                    },
                ));
            }
        }

        let mut sw = LiveSwitcherConfig {
            enabled: true,
            ..LiveSwitcherConfig::default()
        };
        sw.trigger_take(2.0, true, "Opening Stinger");
        assert_eq!(sw.program_channel, 1);
        assert_eq!(sw.preview_channel, 0);
        sw.cut_to_channel(0, 6.5);
        assert_eq!(sw.program_channel, 0);
        assert_eq!(sw.events.len(), 2);

        let cmd = build_live_switcher_veproj_commands(&proj, &sw, 2).unwrap();
        let mut bus = CommandBus::new();
        bus.execute(&mut proj, cmd, CommandSource::Gui, None)
            .unwrap();
        let sc = proj.active_scene().unwrap();
        assert_eq!(sc.markers.len(), 2);
        assert_eq!(sc.layers[2].items.len(), 1); // 1 stinger transition + 1 hard cut
        assert!(sc.layers[0].items[0].keyframes.contains_key("opacity"));
        assert!(sc.layers[1].items[0].keyframes.contains_key("opacity"));

        let json_str = export_live_session_veproj_json(&proj, &sw, 2).unwrap();
        assert!(json_str.contains("LIVE #01"));
        assert!(json_str.contains("Opening Stinger"));
    }
}
