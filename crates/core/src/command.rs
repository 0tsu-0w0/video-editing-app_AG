use crate::animation::Keyframe;
use crate::camera::{Camera3D, Transform3D};
use crate::collab::ReviewComment;
use crate::expression::compile_expression;
use crate::folder::{FolderError, HistoryEntry, ProjectFolder};
use crate::lipsync::CharacterSpriteConfig;
use crate::node_graph::NodeCompositeGraph;
use crate::preset::ColorPalette;
use crate::project::{
    AudioDuckingConfig, AudioEffectNode, AudioReactiveConfig, BezierMask, BlendMode, EffectId,
    EffectNode, FilterTargetScope, ItemId, ItemKind, Layer, MotionBlurConfig, ParticleConfig,
    Project, RgbaColor, Scene, SceneColorGrade, SceneId, TextAnimator, TextStyle, TimelineItem,
    TimelineMarker, TrackMatte, Transform2D, TransitionKind, VectorShapeConfig,
};
use crate::template::{
    TemplateParamValue, apply_responsive_time_stretch, apply_template_values_to_scene,
};
use crate::time::RationalTime;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("Active scene not found")]
    NoActiveScene,
    #[error("Scene not found: {0}")]
    SceneNotFound(SceneId),
    #[error("Layer index out of bounds: {0}")]
    InvalidLayer(usize),
    #[error("Timeline item not found: {0}")]
    ItemNotFound(ItemId),
    #[error("Effect not found: {0}")]
    EffectNotFound(EffectId),
    #[error("Split position {0:?} is outside item bounds")]
    InvalidSplitPosition(RationalTime),
    #[error("No items selected for precompose")]
    EmptyPrecomposeSelection,
    #[error("Invalid expression: {0}")]
    InvalidExpression(String),
    #[error("GUI drag operation in progress; retry after release")]
    GuiDragLocked,
    #[error("Journal I/O error: {0}")]
    Journal(#[from] FolderError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandSource {
    Gui,
    Mcp,
}

impl CommandSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gui => "gui",
            Self::Mcp => "mcp",
        }
    }
}

/// GUI操作と内蔵MCP操作のすべてが経由する統一編集コマンド
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum EditCommand {
    AddObject {
        layer_index: usize,
        item: TimelineItem,
    },
    DeleteObject {
        item_id: ItemId,
    },
    MoveObject {
        item_id: ItemId,
        target_layer_index: usize,
        new_start: RationalTime,
    },
    TrimObject {
        item_id: ItemId,
        new_start: RationalTime,
        new_duration: RationalTime,
    },
    SplitObject {
        item_id: ItemId,
        split_at: RationalTime,
        new_right_id: ItemId,
    },
    SetTransform {
        item_id: ItemId,
        transform: Transform2D,
    },
    SetBlendMode {
        item_id: ItemId,
        blend_mode: BlendMode,
    },
    SetTextContent {
        item_id: ItemId,
        content: String,
    },
    SetTextAnimator {
        item_id: ItemId,
        animator: TextAnimator,
    },
    SetKeyframe {
        item_id: ItemId,
        property: String,
        keyframe: Keyframe,
    },
    RemoveKeyframe {
        item_id: ItemId,
        property: String,
        time: RationalTime,
    },
    SetTrackMatte {
        layer_index: usize,
        matte: Option<TrackMatte>,
    },
    /// ワンクリック結合（Smart Precompose）: 選択アイテムを新シーンへ移送し、Precomp クリップへ置換
    PrecomposeSelection {
        item_ids: Vec<ItemId>,
        new_scene_id: SceneId,
        precomp_name: String,
        precomp_item_id: ItemId,
        target_layer_index: usize,
    },
    SwitchScene {
        scene_id: SceneId,
    },
    SetCamera3D {
        camera: Camera3D,
    },
    SetTransform3D {
        item_id: ItemId,
        transform_3d: Transform3D,
    },
    SetCharacterSprite {
        item_id: ItemId,
        config: CharacterSpriteConfig,
    },
    ImportSubtitles {
        layer_index: usize,
        items: Vec<TimelineItem>,
    },
    AddEffect {
        item_id: ItemId,
        effect: EffectNode,
    },
    UpdateEffectParams {
        item_id: ItemId,
        effect_id: EffectId,
        params: BTreeMap<String, f32>,
    },
    RemoveEffect {
        item_id: ItemId,
        effect_id: EffectId,
    },
    ToggleEffect {
        item_id: ItemId,
        effect_id: EffectId,
        enabled: bool,
    },
    ReorderEffect {
        item_id: ItemId,
        from_index: usize,
        to_index: usize,
    },
    PasteEffects {
        item_id: ItemId,
        effects: Vec<EffectNode>,
    },
    SetColorPalette {
        palette: ColorPalette,
    },
    SetItemColor {
        item_id: ItemId,
        color: RgbaColor,
    },
    SetTextStyle {
        item_id: ItemId,
        style: TextStyle,
    },
    SetTextFont {
        item_id: ItemId,
        font_family: String,
        font_size: f32,
    },
    SetFavoriteFonts {
        fonts: Vec<String>,
    },
    #[allow(clippy::too_many_arguments)]
    SetLayerSettings {
        layer_index: usize,
        name: String,
        visible: bool,
        muted: bool,
        offset_x: f32,
        offset_y: f32,
        volume: f32,
        color_tag: Option<RgbaColor>,
    },
    SetPlaybackSpeed {
        item_id: ItemId,
        speed: f32,
        loop_playback: bool,
    },
    SetTransition {
        item_id: ItemId,
        transition: TransitionKind,
        softness: f32,
    },
    AddAudioEffect {
        /// `None` の場合はアクティブシーンのマスター音声エフェクトチェーンへ追加
        item_id: Option<ItemId>,
        effect: AudioEffectNode,
    },
    RemoveAudioEffect {
        item_id: Option<ItemId>,
        effect_id: EffectId,
    },
    UpdateAudioEffect {
        item_id: Option<ItemId>,
        effect_id: EffectId,
        enabled: bool,
        params: BTreeMap<String, f32>,
    },
    SetTargetLufs {
        target_lufs: Option<f32>,
    },
    AddMarker {
        marker: TimelineMarker,
    },
    RemoveMarker {
        marker_id: uuid::Uuid,
    },
    /// アクティブシーンの解像度・FPSを変更（`rescale_items` でアイテム座標・サイズを自動リフレーム）
    SetSceneSettings {
        width: u32,
        height: u32,
        fps: crate::time::FrameRate,
        rescale_items: bool,
    },
    SetAudioDucking {
        ducking: AudioDuckingConfig,
    },
    SetAudioReactive {
        item_id: ItemId,
        reactive: AudioReactiveConfig,
    },
    /// v0.7 #8: アイテムの自由ベジェマスク列を更新する
    SetMasks {
        item_id: ItemId,
        masks: Vec<BezierMask>,
    },
    /// v0.7 #10: 新規シーンを追加し、任意でアクティブ切替する（ショート動画自動切り抜き等）
    CreateScene {
        scene: Scene,
        switch_to: bool,
    },
    /// v0.7.1: フィルタオブジェクトの種別・適用範囲・パラメータを更新する
    SetFilterObject {
        item_id: ItemId,
        effect_type: String,
        scope: FilterTargetScope,
        params: BTreeMap<String, f32>,
    },
    /// v0.7.1: グループ制御の対象レイヤー数を更新する
    SetGroupControl {
        item_id: ItemId,
        target_layer_count: usize,
    },
    /// v0.7.1: オーディオスペクトラムの設定を更新する
    #[allow(clippy::too_many_arguments)]
    SetAudioSpectrum {
        item_id: ItemId,
        linked_audio_layer: usize,
        width: f32,
        height: f32,
        bar_count: usize,
        color: RgbaColor,
        peak_color: RgbaColor,
        sensitivity: f32,
    },
    /// v0.7.1: 既存タイムラインマーカーのラベル・色を更新する
    UpdateMarker {
        marker_id: uuid::Uuid,
        label: String,
        color: RgbaColor,
    },
    /// v0.8 #13: シーン全体のカラーグレーディング＆.cube 3D LUT 設定を更新する
    SetSceneColorGrade {
        grade: SceneColorGrade,
    },
    /// v0.9 #14: アイテムのプロパティにエクスプレッションを設定（`None` または空文字で解除）
    SetExpression {
        item_id: ItemId,
        property: String,
        expression: Option<String>,
    },
    /// v0.9 #15: モーショングラフィックス テンプレートインスタンスの公開パラメータ＆レスポンシブ尺を更新
    UpdateTemplateInstance {
        item_id: ItemId,
        values: BTreeMap<String, TemplateParamValue>,
        duration_sec: Option<f64>,
    },
    /// v0.9 #16: パーティクルエミッター設定を更新する
    SetParticleConfig {
        item_id: ItemId,
        config: ParticleConfig,
    },
    /// v0.9 #16: 自由ベジェベクターシェイプ設定を更新する
    SetVectorShape {
        item_id: ItemId,
        config: VectorShapeConfig,
    },
    /// v0.9 #16: シーンのシャッター角度ベースモーションブラー設定を更新する
    SetMotionBlur {
        motion_blur: MotionBlurConfig,
    },
    /// v1.0 #19: 選択クリップ群のグルーピング設定／映像・音声リンク解除 (`group_id = None`)
    SetGroupId {
        item_ids: Vec<ItemId>,
        group_id: Option<uuid::Uuid>,
    },
    /// v1.0 #19: スリップトリム（タイムライン位置・尺は変えずに source_offset のみずらす）
    SlipTrim {
        item_id: ItemId,
        delta: RationalTime,
    },
    /// v1.0 #19: ロールトリム（隣接 2 クリップの境界カット点だけを左右へ滑らかに移動）
    RollTrim {
        left_item_id: ItemId,
        right_item_id: ItemId,
        delta: RationalTime,
    },
    /// v1.0 #19: スライドトリム（中央クリップを移動し、前後隣接クリップのアウト／インを自動追従調整）
    SlideTrim {
        item_id: ItemId,
        left_neighbor_id: Option<ItemId>,
        right_neighbor_id: Option<ItemId>,
        delta: RationalTime,
    },
    /// v1.0 #19: オーディオトラックミキサー設定（音量・パン・ミュート・ソロ・出力バス・リバーブセンド）
    SetLayerMixer {
        layer_index: usize,
        volume: f32,
        pan: f32,
        muted: bool,
        solo: bool,
        bus: String,
        send_reverb: f32,
    },
    /// v2.0 #21: アクティブシーンのノードベース合成グラフを更新する
    SetNodeGraph {
        graph: NodeCompositeGraph,
    },
    /// v2.0 #22: 動画クリップのオプティカルフロー中間フレーム補間フラグを更新する
    SetOpticalFlowInterp {
        item_id: ItemId,
        enabled: bool,
    },
    /// v2.0 #24: アクティブシーンのタイムコード付きレビューコメント一覧を更新する
    SetReviewComments {
        comments: Vec<ReviewComment>,
    },
    /// v2.0 #24: 3-Way ブランチマージ結果を 1 ステップ Undo 可能に適用する
    ReplaceProjectSnapshot {
        project: Box<Project>,
    },
    /// v2.1 #26: アイテムの 4 点コーナーピン合成（ホモグラフィ射影変換）を更新する
    SetCornerPin {
        item_id: ItemId,
        corner_pin: crate::project::CornerPinConfig,
    },
    /// v2.1 #28: テキストアイテムのワード単位キネティック字幕設定を更新する
    SetKineticSubtitle {
        item_id: ItemId,
        kinetic: crate::kinetic_sub::KineticSubtitleConfig,
    },
    /// v3.0 #30: 3D メッシュ / glTF 2.0 設定を更新する
    SetMesh3D {
        item_id: ItemId,
        config: crate::scene3d::Mesh3DConfig,
    },
    /// v3.0 #31: オフライン吹替／リップシンク用の音素タイミング列を更新する
    SetItemPhonemes {
        item_id: ItemId,
        phonemes: Vec<crate::lipsync::PhonemeSegment>,
    },
    /// v4.0 #33: シーンの ACEScg / OpenColorIO プロカラー管理設定を更新する
    SetColorManagement {
        config: crate::color_mgmt::ColorManagementConfig,
    },
    /// v4.0 #35: プロシージャル背景ジェネレーター設定を更新する
    SetGenerativeBg {
        item_id: ItemId,
        config: crate::generative::GenerativeBgConfig,
    },
    /// v5.0 #37: 2D スケルトンボーン＆2-Bone IK パペットリグ設定を更新する
    SetRig2D {
        item_id: ItemId,
        config: Box<crate::rigging::Rig2DConfig>,
    },
    /// v5.0 #39: クリップのスピードランプ（Time-Remapping 曲線）設定を更新する
    SetSpeedRamp {
        item_id: ItemId,
        speed_ramp: crate::speed_ramp::SpeedRampConfig,
    },
    /// v5.0 #40: シーンのシネマフィニッシング（ハレーション・フレア・グレイン・レターボックス・バーンイン）を更新する
    SetCinemaFinishing {
        config: crate::finishing::CinemaFinishingConfig,
    },
    /// v6.0 #41: データ駆動インフォグラフィック・チャート設定を更新する
    SetDataChart {
        item_id: ItemId,
        config: Box<crate::data_viz::DataChartConfig>,
    },
    /// v6.0 #42: クリップの非破壊 VFX ペイント／クローンスタンプ／ワイヤー消し設定を更新する
    SetVfxPaint {
        item_id: ItemId,
        vfx_paint: crate::vfx_paint::VfxPaintConfig,
    },
    /// v7.0 #48: アセットのリール名・SMPTE 開始タイムコード・プロキシ連携メタデータを更新する
    SetAssetConformMeta {
        asset_id: crate::project::AssetId,
        meta: crate::conform::AssetConformMeta,
    },
    /// v7.0 #48: プロキシ⇔原本自動コンフォームでクリップの参照アセットとソース開始オフセットを差し替える
    ConformClipSource {
        item_id: ItemId,
        new_asset_id: crate::project::AssetId,
        new_source_offset: RationalTime,
    },
    /// v7.0 #48: アクティブシーンの A/B スプリット比較ビュー設定を更新する
    SetAbCompare {
        ab_compare: crate::studio_hub::AbCompareConfig,
    },
    /// v8.0 #49: クリップのエッジ吸着キーフレームロトブラシ設定を更新する
    SetRotoBrush {
        item_id: ItemId,
        roto_brush: crate::v80_vfx::RotoBrushConfig,
    },
    /// v8.0 #50: SVG `<path>` インポート＆モーフィング設定を更新する
    SetSvgMorph {
        item_id: ItemId,
        config: Box<crate::motion_foundry::SvgMorphConfig>,
    },
    /// v8.0 #50: テキストクリップの Text-on-Path 設定を更新する
    SetTextOnPath {
        item_id: ItemId,
        text_on_path: crate::motion_foundry::TextOnPathConfig,
    },
    /// v8.0 #50: クリップのディスプレイスメント／グリッドワープ設定を更新する
    SetWarpDisplace {
        item_id: ItemId,
        warp_displace: crate::motion_foundry::WarpDisplaceConfig,
    },
    /// v8.0 #51: アクティブシーンのシネマ光学 VFX 設定を更新する
    SetOpticalVfx {
        optical_vfx: crate::v80_vfx::OpticalVfxConfig,
    },
    /// v8.0 #52: アクティブシーンの ADR カウント収録セッション設定を更新する
    SetAdrSession {
        adr_session: crate::v80_vfx::AdrSessionConfig,
    },
    /// v8.0 #52: アクティブシーンの 5.1ch / FOA Ambisonics 空間音声バス設定を更新する
    SetSpatialBus {
        spatial_bus: crate::v80_vfx::SpatialBusConfig,
    },
    /// v9.0 #53: アクティブシーンの 2 バスライブスイッチャー＆NLE 同時収録設定を更新する
    SetLiveSwitcher {
        live_switcher: crate::live_switcher::LiveSwitcherConfig,
    },
    /// v9.0 #54: VTuber 2D 三角メッシュ＆Verlet 物理設定を更新する
    SetVtuberAvatar {
        item_id: ItemId,
        config: Box<crate::vtuber_rig::VtuberMeshConfig>,
    },
    /// v9.0 #55: プロジェクトの OSC/MIDI/Stream Deck ハードウェアコントローラー設定を更新する
    SetHardwareController {
        hardware_ctrl: crate::hardware_ctrl::HardwareControllerConfig,
    },
    /// v9.0 #56: プロジェクトのローカル スマート DAM インデックスを更新する
    SetSmartDamIndex {
        smart_dam: crate::smart_dam::SmartDamIndex,
    },
    /// v9.0 #56: 行方不明アセットのファイルパスを 1-Undo で一括自動再リンクする
    RelinkAssets {
        relinks: Vec<(crate::project::AssetId, String)>,
    },
    /// 複数コマンドを1つの Undo 単位として原子的に実行する
    Batch {
        label: String,
        commands: Vec<EditCommand>,
    },
}

impl EditCommand {
    pub fn command_name(&self) -> &'static str {
        match self {
            Self::AddObject { .. } => "add_object",
            Self::DeleteObject { .. } => "delete_object",
            Self::MoveObject { .. } => "move_object",
            Self::TrimObject { .. } => "trim_object",
            Self::SplitObject { .. } => "split_object",
            Self::SetTransform { .. } => "set_transform",
            Self::SetBlendMode { .. } => "set_blend_mode",
            Self::SetTextContent { .. } => "set_text_content",
            Self::SetTextAnimator { .. } => "set_text_animator",
            Self::SetKeyframe { .. } => "set_keyframe",
            Self::RemoveKeyframe { .. } => "remove_keyframe",
            Self::SetTrackMatte { .. } => "set_track_matte",
            Self::PrecomposeSelection { .. } => "precompose_selection",
            Self::SwitchScene { .. } => "switch_scene",
            Self::SetCamera3D { .. } => "set_camera_3d",
            Self::SetTransform3D { .. } => "set_transform_3d",
            Self::SetCharacterSprite { .. } => "set_character_sprite",
            Self::ImportSubtitles { .. } => "import_subtitles",
            Self::AddEffect { .. } => "add_effect",
            Self::UpdateEffectParams { .. } => "update_effect_params",
            Self::RemoveEffect { .. } => "remove_effect",
            Self::ToggleEffect { .. } => "toggle_effect",
            Self::ReorderEffect { .. } => "reorder_effect",
            Self::PasteEffects { .. } => "paste_effects",
            Self::SetColorPalette { .. } => "set_color_palette",
            Self::SetItemColor { .. } => "set_item_color",
            Self::SetTextStyle { .. } => "set_text_style",
            Self::SetTextFont { .. } => "set_text_font",
            Self::SetFavoriteFonts { .. } => "set_favorite_fonts",
            Self::SetLayerSettings { .. } => "set_layer_settings",
            Self::SetPlaybackSpeed { .. } => "set_playback_speed",
            Self::SetTransition { .. } => "set_transition",
            Self::AddAudioEffect { .. } => "add_audio_effect",
            Self::RemoveAudioEffect { .. } => "remove_audio_effect",
            Self::UpdateAudioEffect { .. } => "update_audio_effect",
            Self::SetTargetLufs { .. } => "set_target_lufs",
            Self::AddMarker { .. } => "add_marker",
            Self::RemoveMarker { .. } => "remove_marker",
            Self::SetSceneSettings { .. } => "set_scene_settings",
            Self::SetAudioDucking { .. } => "set_audio_ducking",
            Self::SetAudioReactive { .. } => "set_audio_reactive",
            Self::SetMasks { .. } => "set_masks",
            Self::CreateScene { .. } => "create_scene",
            Self::SetFilterObject { .. } => "set_filter_object",
            Self::SetGroupControl { .. } => "set_group_control",
            Self::SetAudioSpectrum { .. } => "set_audio_spectrum",
            Self::UpdateMarker { .. } => "update_marker",
            Self::SetSceneColorGrade { .. } => "set_scene_color_grade",
            Self::SetExpression { .. } => "set_expression",
            Self::UpdateTemplateInstance { .. } => "update_template_instance",
            Self::SetParticleConfig { .. } => "set_particle_config",
            Self::SetVectorShape { .. } => "set_vector_shape",
            Self::SetMotionBlur { .. } => "set_motion_blur",
            Self::SetGroupId { .. } => "set_group_id",
            Self::SlipTrim { .. } => "slip_trim",
            Self::RollTrim { .. } => "roll_trim",
            Self::SlideTrim { .. } => "slide_trim",
            Self::SetLayerMixer { .. } => "set_layer_mixer",
            Self::SetNodeGraph { .. } => "set_node_graph",
            Self::SetOpticalFlowInterp { .. } => "set_optical_flow_interp",
            Self::SetReviewComments { .. } => "set_review_comments",
            Self::ReplaceProjectSnapshot { .. } => "replace_project_snapshot",
            Self::SetCornerPin { .. } => "set_corner_pin",
            Self::SetKineticSubtitle { .. } => "set_kinetic_subtitle",
            Self::SetMesh3D { .. } => "set_mesh_3d",
            Self::SetItemPhonemes { .. } => "set_item_phonemes",
            Self::SetColorManagement { .. } => "set_color_management",
            Self::SetGenerativeBg { .. } => "set_generative_bg",
            Self::SetRig2D { .. } => "set_rig_2d",
            Self::SetSpeedRamp { .. } => "set_speed_ramp",
            Self::SetCinemaFinishing { .. } => "set_cinema_finishing",
            Self::SetDataChart { .. } => "set_data_chart",
            Self::SetVfxPaint { .. } => "set_vfx_paint",
            Self::SetAssetConformMeta { .. } => "set_asset_conform_meta",
            Self::ConformClipSource { .. } => "conform_clip_source",
            Self::SetAbCompare { .. } => "set_ab_compare",
            Self::SetRotoBrush { .. } => "set_roto_brush",
            Self::SetSvgMorph { .. } => "set_svg_morph",
            Self::SetTextOnPath { .. } => "set_text_on_path",
            Self::SetWarpDisplace { .. } => "set_warp_displace",
            Self::SetOpticalVfx { .. } => "set_optical_vfx",
            Self::SetAdrSession { .. } => "set_adr_session",
            Self::SetSpatialBus { .. } => "set_spatial_bus",
            Self::SetLiveSwitcher { .. } => "set_live_switcher",
            Self::SetVtuberAvatar { .. } => "set_vtuber_avatar",
            Self::SetHardwareController { .. } => "set_hardware_controller",
            Self::SetSmartDamIndex { .. } => "set_smart_dam_index",
            Self::RelinkAssets { .. } => "relink_assets",
            Self::Batch { .. } => "batch_execute",
        }
    }
}

#[derive(Debug, Clone)]
struct UndoRecord {
    coalesce_key: Option<String>,
    before: Project,
    after: Project,
    command: EditCommand,
    source: CommandSource,
}

/// Undo スタックの既定最大保持段数（長時間編集時のメモリ肥大化を防止）
pub const DEFAULT_MAX_UNDO_DEPTH: usize = 200;

/// GUI・AI（MCP）・Undo/Redo・Command Coalescing・`history.jsonl` 復旧を一元管理する Command Bus
#[derive(Debug)]
pub struct CommandBus {
    undo_stack: std::collections::VecDeque<UndoRecord>,
    redo_stack: Vec<UndoRecord>,
    max_undo_depth: usize,
    revision: u64,
    next_seq: u64,
    gui_drag_locked: bool,
    journal_folder: Option<ProjectFolder>,
}

impl Default for CommandBus {
    fn default() -> Self {
        Self {
            undo_stack: std::collections::VecDeque::new(),
            redo_stack: Vec::new(),
            max_undo_depth: DEFAULT_MAX_UNDO_DEPTH,
            revision: 0,
            next_seq: 0,
            gui_drag_locked: false,
            journal_folder: None,
        }
    }
}

impl CommandBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_folder(folder: ProjectFolder) -> Self {
        let next_seq = folder
            .read_history()
            .map(|h| h.last().map(|e| e.seq + 1).unwrap_or(1))
            .unwrap_or(1);
        Self {
            next_seq,
            journal_folder: Some(folder),
            ..Self::default()
        }
    }

    pub fn with_max_undo_depth(mut self, max_depth: usize) -> Self {
        self.max_undo_depth = max_depth.max(1);
        while self.undo_stack.len() > self.max_undo_depth {
            self.undo_stack.pop_front();
        }
        self
    }

    /// プロジェクト状態が変更（execute / undo / redo）されるたびに増加するモノトニック世代番号（GUI ダーティ判定用）
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn set_gui_drag_locked(&mut self, locked: bool) {
        self.gui_drag_locked = locked;
    }

    pub fn is_gui_drag_locked(&self) -> bool {
        self.gui_drag_locked
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo_len(&self) -> usize {
        self.undo_stack.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo_stack.len()
    }

    /// v2.0 #23: Undo スタック上の直近実行コマンド列を古い順で返す（編集レシピの自動記録用）
    pub fn recent_commands(&self, limit: usize) -> Vec<EditCommand> {
        let len = self.undo_stack.len();
        let skip = len.saturating_sub(limit);
        self.undo_stack
            .iter()
            .skip(skip)
            .map(|r| r.command.clone())
            .collect()
    }

    /// コマンドを実行する。`coalesce_key` が直前の Undo レコードと一致する場合は1つの Undo 単位に統合する。
    pub fn execute(
        &mut self,
        project: &mut Project,
        command: EditCommand,
        source: CommandSource,
        coalesce_key: Option<&str>,
    ) -> Result<(), CommandError> {
        if source == CommandSource::Mcp && self.gui_drag_locked {
            return Err(CommandError::GuiDragLocked);
        }

        // 作業コピー上で先に適用し、失敗時は元プロジェクトを一切汚さない
        let mut working = project.clone();
        apply_command(&mut working, &command)?;

        // コミット確定 & Command Coalescing（Project クローン回数を最小化）
        self.redo_stack.clear();
        let can_coalesce = coalesce_key.is_some_and(|key| {
            self.undo_stack.back().is_some_and(|last| {
                last.source == source && last.coalesce_key.as_deref() == Some(key)
            })
        });

        if can_coalesce {
            if let Some(last) = self.undo_stack.back_mut() {
                last.after = working.clone();
                last.command = command.clone();
            }
            *project = working;
        } else {
            let before = std::mem::replace(project, working.clone());
            if self.undo_stack.len() >= self.max_undo_depth.max(1) {
                self.undo_stack.pop_front();
            }
            self.undo_stack.push_back(UndoRecord {
                coalesce_key: coalesce_key.map(str::to_owned),
                before,
                after: working,
                command: command.clone(),
                source,
            });
        }

        self.revision = self.revision.wrapping_add(1);

        if let Some(folder) = &self.journal_folder {
            let seq = self.next_seq.max(1);
            self.next_seq = seq + 1;
            let timestamp_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let entry = HistoryEntry {
                seq,
                timestamp_ms,
                source: source.as_str().to_string(),
                command_type: command.command_name().to_string(),
                payload: serde_json::to_value(&command)?,
            };
            folder.append_history(&entry)?;
        }

        Ok(())
    }

    pub fn undo(&mut self, project: &mut Project) -> bool {
        if let Some(record) = self.undo_stack.pop_back() {
            *project = record.before.clone();
            self.redo_stack.push(record);
            self.revision = self.revision.wrapping_add(1);
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self, project: &mut Project) -> bool {
        if let Some(record) = self.redo_stack.pop() {
            *project = record.after.clone();
            if self.undo_stack.len() >= self.max_undo_depth.max(1) {
                self.undo_stack.pop_front();
            }
            self.undo_stack.push_back(record);
            self.revision = self.revision.wrapping_add(1);
            true
        } else {
            false
        }
    }

    /// クラッシュ復旧: `history.jsonl` のエントリ列をスナップショット上のプロジェクトへ再適用する
    pub fn replay_history(
        project: &mut Project,
        entries: &[HistoryEntry],
    ) -> Result<usize, CommandError> {
        let mut applied = 0;
        for entry in entries {
            let cmd: EditCommand = serde_json::from_value(entry.payload.clone())?;
            apply_command(project, &cmd)?;
            applied += 1;
        }
        Ok(applied)
    }
}

fn find_item_ref(project: &Project, item_id: ItemId) -> Result<&TimelineItem, CommandError> {
    let scene = project.active_scene().ok_or(CommandError::NoActiveScene)?;
    for layer in &scene.layers {
        if let Some(item) = layer.items.iter().find(|i| i.id == item_id) {
            return Ok(item);
        }
    }
    Err(CommandError::ItemNotFound(item_id))
}

fn find_item_mut(
    project: &mut Project,
    item_id: ItemId,
) -> Result<&mut TimelineItem, CommandError> {
    let scene = project
        .active_scene_mut()
        .ok_or(CommandError::NoActiveScene)?;
    for layer in &mut scene.layers {
        if let Some(item) = layer.items.iter_mut().find(|i| i.id == item_id) {
            return Ok(item);
        }
    }
    Err(CommandError::ItemNotFound(item_id))
}

fn apply_command(project: &mut Project, command: &EditCommand) -> Result<(), CommandError> {
    match command {
        EditCommand::AddObject { layer_index, item } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let layer = scene
                .layers
                .get_mut(*layer_index)
                .ok_or(CommandError::InvalidLayer(*layer_index))?;
            layer.items.push(item.clone());
            Ok(())
        }
        EditCommand::DeleteObject { item_id } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            for layer in &mut scene.layers {
                if let Some(pos) = layer.items.iter().position(|i| i.id == *item_id) {
                    layer.items.remove(pos);
                    return Ok(());
                }
            }
            Err(CommandError::ItemNotFound(*item_id))
        }
        EditCommand::MoveObject {
            item_id,
            target_layer_index,
            new_start,
        } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            if *target_layer_index >= scene.layers.len() {
                return Err(CommandError::InvalidLayer(*target_layer_index));
            }
            let mut extracted = None;
            for layer in &mut scene.layers {
                if let Some(pos) = layer.items.iter().position(|i| i.id == *item_id) {
                    extracted = Some(layer.items.remove(pos));
                    break;
                }
            }
            let mut item = extracted.ok_or(CommandError::ItemNotFound(*item_id))?;
            item.start = (*new_start).max(RationalTime::ZERO);
            scene.layers[*target_layer_index].items.push(item);
            Ok(())
        }
        EditCommand::TrimObject {
            item_id,
            new_start,
            new_duration,
        } => {
            let item = find_item_mut(project, *item_id)?;
            item.start = (*new_start).max(RationalTime::ZERO);
            item.duration = (*new_duration).max(RationalTime::from_ticks(1));
            Ok(())
        }
        EditCommand::SplitObject {
            item_id,
            split_at,
            new_right_id,
        } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            for layer in &mut scene.layers {
                if let Some(pos) = layer.items.iter().position(|i| i.id == *item_id) {
                    let left = &mut layer.items[pos];
                    if *split_at <= left.start || *split_at >= left.end() {
                        return Err(CommandError::InvalidSplitPosition(*split_at));
                    }
                    let left_dur = *split_at - left.start;
                    let right_dur = left.end() - *split_at;
                    let mut right = left.clone();
                    right.id = *new_right_id;
                    right.start = *split_at;
                    right.duration = right_dur;
                    match &mut right.kind {
                        ItemKind::Video { source_offset, .. }
                        | ItemKind::Audio { source_offset, .. } => {
                            *source_offset = *source_offset + left_dur;
                        }
                        _ => {}
                    }
                    left.duration = left_dur;
                    layer.items.insert(pos + 1, right);
                    return Ok(());
                }
            }
            Err(CommandError::ItemNotFound(*item_id))
        }
        EditCommand::SetTransform { item_id, transform } => {
            let item = find_item_mut(project, *item_id)?;
            item.transform = *transform;
            Ok(())
        }
        EditCommand::SetBlendMode {
            item_id,
            blend_mode,
        } => {
            let item = find_item_mut(project, *item_id)?;
            item.blend_mode = *blend_mode;
            Ok(())
        }
        EditCommand::SetTextContent { item_id, content } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::Text {
                content: existing, ..
            } = &mut item.kind
            {
                *existing = content.clone();
            }
            Ok(())
        }
        EditCommand::SetTextAnimator { item_id, animator } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::Text {
                animator: existing, ..
            } = &mut item.kind
            {
                *existing = *animator;
            }
            Ok(())
        }
        EditCommand::SetKeyframe {
            item_id,
            property,
            keyframe,
        } => {
            let item = find_item_mut(project, *item_id)?;
            let track = item.keyframes.entry(property.clone()).or_default();
            if let Some(existing) = track.iter_mut().find(|k| k.time == keyframe.time) {
                *existing = *keyframe;
            } else {
                track.push(*keyframe);
                track.sort_by_key(|k| k.time);
            }
            Ok(())
        }
        EditCommand::RemoveKeyframe {
            item_id,
            property,
            time,
        } => {
            let item = find_item_mut(project, *item_id)?;
            if let Some(track) = item.keyframes.get_mut(property) {
                track.retain(|k| k.time != *time);
            }
            Ok(())
        }
        EditCommand::SetTrackMatte { layer_index, matte } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let layer = scene
                .layers
                .get_mut(*layer_index)
                .ok_or(CommandError::InvalidLayer(*layer_index))?;
            layer.track_matte = *matte;
            Ok(())
        }
        EditCommand::PrecomposeSelection {
            item_ids,
            new_scene_id,
            precomp_name,
            precomp_item_id,
            target_layer_index,
        } => {
            if item_ids.is_empty() {
                return Err(CommandError::EmptyPrecomposeSelection);
            }
            let (width, height, fps) = {
                let active = project.active_scene().ok_or(CommandError::NoActiveScene)?;
                if *target_layer_index >= active.layers.len() {
                    return Err(CommandError::InvalidLayer(*target_layer_index));
                }
                (active.width, active.height, active.fps)
            };

            // 選択アイテムを元シーンからレイヤー順を保って抽出
            let mut extracted: Vec<(usize, TimelineItem)> = Vec::new();
            {
                let active = project
                    .active_scene_mut()
                    .ok_or(CommandError::NoActiveScene)?;
                for (layer_idx, layer) in active.layers.iter_mut().enumerate() {
                    let mut remaining = Vec::with_capacity(layer.items.len());
                    for item in layer.items.drain(..) {
                        if item_ids.contains(&item.id) {
                            extracted.push((layer_idx, item));
                        } else {
                            remaining.push(item);
                        }
                    }
                    layer.items = remaining;
                }
            }
            if extracted.is_empty() {
                return Err(CommandError::EmptyPrecomposeSelection);
            }

            let min_start = extracted
                .iter()
                .map(|(_, it)| it.start)
                .min()
                .unwrap_or(RationalTime::ZERO);
            let max_end = extracted
                .iter()
                .map(|(_, it)| it.end())
                .max()
                .unwrap_or(min_start + RationalTime::from_seconds(1.0));
            let precomp_duration = (max_end - min_start).max(RationalTime::from_ticks(1));

            let max_layer_idx = extracted.iter().map(|(l, _)| *l).max().unwrap_or(0);
            let mut new_layers: Vec<Layer> = (0..=max_layer_idx.max(2))
                .map(|i| {
                    let mut l = Layer::new(format!("Layer {}", i + 1));
                    // クラッシュ復旧リプレイ時の完全一致＆Precompハッシュキャッシュ整合性のため決定論的IDを割当
                    l.id =
                        uuid::Uuid::from_u128(new_scene_id.as_u128().wrapping_add((i + 1) as u128));
                    l
                })
                .collect();
            for (layer_idx, mut item) in extracted {
                item.start = item.start - min_start;
                new_layers[layer_idx].items.push(item);
            }

            let new_scene = Scene {
                id: *new_scene_id,
                name: precomp_name.clone(),
                width,
                height,
                fps,
                camera: Camera3D::default(),
                master_audio_effects: Vec::new(),
                target_lufs: None,
                audio_ducking: AudioDuckingConfig::default(),
                color_grade: SceneColorGrade::default(),
                color_management: crate::color_mgmt::ColorManagementConfig::default(),
                cinema_finishing: crate::finishing::CinemaFinishingConfig::default(),
                motion_blur: MotionBlurConfig::default(),
                markers: Vec::new(),
                node_graph: NodeCompositeGraph::default(),
                review_comments: Vec::new(),
                ab_compare: crate::studio_hub::AbCompareConfig::default(),
                optical_vfx: crate::v80_vfx::OpticalVfxConfig::default(),
                adr_session: crate::v80_vfx::AdrSessionConfig::default(),
                spatial_bus: crate::v80_vfx::SpatialBusConfig::default(),
                live_switcher: crate::live_switcher::LiveSwitcherConfig::default(),
                layers: new_layers,
            };
            project.scenes.push(new_scene);

            let mut precomp_clip = TimelineItem::new(
                precomp_name.clone(),
                min_start,
                precomp_duration,
                ItemKind::Precomp {
                    scene_id: *new_scene_id,
                },
            );
            precomp_clip.id = *precomp_item_id;

            let active = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            active.layers[*target_layer_index].items.push(precomp_clip);
            Ok(())
        }
        EditCommand::SwitchScene { scene_id } => {
            if project.scene(*scene_id).is_none() {
                return Err(CommandError::SceneNotFound(*scene_id));
            }
            project.active_scene_id = *scene_id;
            Ok(())
        }
        EditCommand::SetCamera3D { camera } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            scene.camera = *camera;
            Ok(())
        }
        EditCommand::SetTransform3D {
            item_id,
            transform_3d,
        } => {
            let item = find_item_mut(project, *item_id)?;
            item.transform_3d = *transform_3d;
            Ok(())
        }
        EditCommand::SetCharacterSprite { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::CharacterSprite {
                config: existing, ..
            } = &mut item.kind
            {
                *existing = config.clone();
            }
            Ok(())
        }
        EditCommand::ImportSubtitles { layer_index, items } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let layer = scene
                .layers
                .get_mut(*layer_index)
                .ok_or(CommandError::InvalidLayer(*layer_index))?;
            layer.items.extend(items.iter().cloned());
            Ok(())
        }
        EditCommand::AddEffect { item_id, effect } => {
            let item = find_item_mut(project, *item_id)?;
            item.effect_chain.push(effect.clone());
            Ok(())
        }
        EditCommand::UpdateEffectParams {
            item_id,
            effect_id,
            params,
        } => {
            let item = find_item_mut(project, *item_id)?;
            let eff = item
                .effect_chain
                .iter_mut()
                .find(|e| e.id == *effect_id)
                .ok_or(CommandError::EffectNotFound(*effect_id))?;
            for (k, v) in params {
                eff.params.insert(k.clone(), *v);
            }
            Ok(())
        }
        EditCommand::RemoveEffect { item_id, effect_id } => {
            let item = find_item_mut(project, *item_id)?;
            let prev = item.effect_chain.len();
            item.effect_chain.retain(|e| e.id != *effect_id);
            if item.effect_chain.len() == prev {
                return Err(CommandError::EffectNotFound(*effect_id));
            }
            Ok(())
        }
        EditCommand::ToggleEffect {
            item_id,
            effect_id,
            enabled,
        } => {
            let item = find_item_mut(project, *item_id)?;
            let eff = item
                .effect_chain
                .iter_mut()
                .find(|e| e.id == *effect_id)
                .ok_or(CommandError::EffectNotFound(*effect_id))?;
            eff.enabled = *enabled;
            Ok(())
        }
        EditCommand::ReorderEffect {
            item_id,
            from_index,
            to_index,
        } => {
            let item = find_item_mut(project, *item_id)?;
            let len = item.effect_chain.len();
            if *from_index < len && *to_index < len && from_index != to_index {
                let eff = item.effect_chain.remove(*from_index);
                item.effect_chain.insert(*to_index, eff);
            }
            Ok(())
        }
        EditCommand::PasteEffects { item_id, effects } => {
            let item = find_item_mut(project, *item_id)?;
            item.effect_chain.extend(effects.iter().cloned());
            Ok(())
        }
        EditCommand::SetColorPalette { palette } => {
            project.color_palette = palette.clone();
            Ok(())
        }
        EditCommand::SetItemColor { item_id, color } => {
            let item = find_item_mut(project, *item_id)?;
            match &mut item.kind {
                ItemKind::Text {
                    color: existing, ..
                }
                | ItemKind::Shape {
                    color: existing, ..
                } => {
                    *existing = *color;
                }
                _ => {}
            }
            Ok(())
        }
        EditCommand::SetTextStyle { item_id, style } => {
            let item = find_item_mut(project, *item_id)?;
            item.text_style = *style;
            Ok(())
        }
        EditCommand::SetTextFont {
            item_id,
            font_family,
            font_size,
        } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::Text {
                font_family: ff,
                font_size: fs,
                ..
            } = &mut item.kind
            {
                *ff = font_family.clone();
                *fs = font_size.clamp(6.0, 400.0);
            }
            Ok(())
        }
        EditCommand::SetFavoriteFonts { fonts } => {
            project.favorite_fonts = fonts.clone();
            Ok(())
        }
        EditCommand::SetLayerSettings {
            layer_index,
            name,
            visible,
            muted,
            offset_x,
            offset_y,
            volume,
            color_tag,
        } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let layer = scene
                .layers
                .get_mut(*layer_index)
                .ok_or(CommandError::InvalidLayer(*layer_index))?;
            layer.name = name.clone();
            layer.visible = *visible;
            layer.muted = *muted;
            layer.offset_x = *offset_x;
            layer.offset_y = *offset_y;
            layer.volume = volume.clamp(0.0, 4.0);
            layer.color_tag = *color_tag;
            Ok(())
        }
        EditCommand::SetPlaybackSpeed {
            item_id,
            speed,
            loop_playback,
        } => {
            let item = find_item_mut(project, *item_id)?;
            item.playback.speed = speed.clamp(-8.0, 8.0);
            item.playback.loop_playback = *loop_playback;
            Ok(())
        }
        EditCommand::SetTransition {
            item_id,
            transition,
            softness,
        } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::Transition {
                transition: t,
                softness: s,
                ..
            } = &mut item.kind
            {
                *t = *transition;
                *s = softness.clamp(0.0, 0.5);
            }
            Ok(())
        }
        EditCommand::AddAudioEffect { item_id, effect } => {
            if let Some(id) = item_id {
                let item = find_item_mut(project, *id)?;
                item.audio_effects.push(effect.clone());
            } else {
                let scene = project
                    .active_scene_mut()
                    .ok_or(CommandError::NoActiveScene)?;
                scene.master_audio_effects.push(effect.clone());
            }
            Ok(())
        }
        EditCommand::RemoveAudioEffect { item_id, effect_id } => {
            let chain = if let Some(id) = item_id {
                &mut find_item_mut(project, *id)?.audio_effects
            } else {
                &mut project
                    .active_scene_mut()
                    .ok_or(CommandError::NoActiveScene)?
                    .master_audio_effects
            };
            let prev = chain.len();
            chain.retain(|e| e.id != *effect_id);
            if chain.len() == prev {
                return Err(CommandError::EffectNotFound(*effect_id));
            }
            Ok(())
        }
        EditCommand::UpdateAudioEffect {
            item_id,
            effect_id,
            enabled,
            params,
        } => {
            let chain = if let Some(id) = item_id {
                &mut find_item_mut(project, *id)?.audio_effects
            } else {
                &mut project
                    .active_scene_mut()
                    .ok_or(CommandError::NoActiveScene)?
                    .master_audio_effects
            };
            let eff = chain
                .iter_mut()
                .find(|e| e.id == *effect_id)
                .ok_or(CommandError::EffectNotFound(*effect_id))?;
            eff.enabled = *enabled;
            for (k, v) in params {
                eff.params.insert(k.clone(), *v);
            }
            Ok(())
        }
        EditCommand::SetTargetLufs { target_lufs } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            scene.target_lufs = target_lufs.map(|v| v.clamp(-36.0, -3.0));
            Ok(())
        }
        EditCommand::AddMarker { marker } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            scene.markers.push(marker.clone());
            scene.markers.sort_by_key(|m| m.time);
            Ok(())
        }
        EditCommand::RemoveMarker { marker_id } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            scene.markers.retain(|m| m.id != *marker_id);
            Ok(())
        }
        EditCommand::SetSceneSettings {
            width,
            height,
            fps,
            rescale_items,
        } => {
            let new_w = (*width).clamp(16, 8192);
            let new_h = (*height).clamp(16, 8192);
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let old_w = scene.width.max(1) as f32;
            let old_h = scene.height.max(1) as f32;
            scene.width = new_w;
            scene.height = new_h;
            scene.fps = *fps;

            if *rescale_items {
                let sx = new_w as f32 / old_w;
                let sy = new_h as f32 / old_h;
                let uniform = sx.min(sy);
                for layer in &mut scene.layers {
                    layer.offset_x *= sx;
                    layer.offset_y *= sy;
                    for item in &mut layer.items {
                        item.transform.x *= sx;
                        item.transform.y *= sy;
                        if let Some(kfs) = item.keyframes.get_mut("x") {
                            for kf in kfs {
                                kf.value *= sx;
                            }
                        }
                        if let Some(kfs) = item.keyframes.get_mut("y") {
                            for kf in kfs {
                                kf.value *= sy;
                            }
                        }
                        match &mut item.kind {
                            ItemKind::Shape { width, height, .. }
                            | ItemKind::CharacterSprite { width, height, .. }
                            | ItemKind::AudioSpectrum { width, height, .. } => {
                                *width = (*width * uniform).max(4.0);
                                *height = (*height * uniform).max(4.0);
                            }
                            ItemKind::Text { font_size, .. } => {
                                *font_size = (*font_size * uniform).clamp(8.0, 400.0);
                            }
                            _ => {}
                        }
                    }
                }
            }
            project.settings.width = new_w;
            project.settings.height = new_h;
            project.settings.fps = *fps;
            Ok(())
        }
        EditCommand::SetAudioDucking { ducking } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            scene.audio_ducking = *ducking;
            Ok(())
        }
        EditCommand::SetAudioReactive { item_id, reactive } => {
            let item = find_item_mut(project, *item_id)?;
            item.audio_reactive = *reactive;
            Ok(())
        }
        EditCommand::SetMasks { item_id, masks } => {
            let item = find_item_mut(project, *item_id)?;
            item.masks = masks.clone();
            Ok(())
        }
        EditCommand::CreateScene { scene, switch_to } => {
            let new_id = scene.id;
            if let Some(existing) = project.scenes.iter_mut().find(|s| s.id == new_id) {
                *existing = scene.clone();
            } else {
                project.scenes.push(scene.clone());
            }
            if *switch_to {
                project.active_scene_id = new_id;
            }
            Ok(())
        }
        EditCommand::SetFilterObject {
            item_id,
            effect_type,
            scope,
            params,
        } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::FilterObject {
                effect_type: et,
                scope: sc,
                params: pm,
            } = &mut item.kind
            {
                *et = effect_type.clone();
                *sc = *scope;
                *pm = params.clone();
            }
            Ok(())
        }
        EditCommand::SetGroupControl {
            item_id,
            target_layer_count,
        } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::GroupControl {
                target_layer_count: cnt,
            } = &mut item.kind
            {
                *cnt = (*target_layer_count).clamp(1, 64);
            }
            Ok(())
        }
        EditCommand::SetAudioSpectrum {
            item_id,
            linked_audio_layer,
            width,
            height,
            bar_count,
            color,
            peak_color,
            sensitivity,
        } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::AudioSpectrum {
                linked_audio_layer: l,
                width: w,
                height: h,
                bar_count: bc,
                color: c,
                peak_color: pc,
                sensitivity: sens,
            } = &mut item.kind
            {
                *l = *linked_audio_layer;
                *w = width.clamp(16.0, 8192.0);
                *h = height.clamp(16.0, 8192.0);
                *bc = (*bar_count).clamp(4, 64);
                *c = *color;
                *pc = *peak_color;
                *sens = sensitivity.clamp(0.1, 10.0);
            }
            Ok(())
        }
        EditCommand::UpdateMarker {
            marker_id,
            label,
            color,
        } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            if let Some(m) = scene.markers.iter_mut().find(|m| m.id == *marker_id) {
                m.label = label.clone();
                m.color = *color;
            }
            Ok(())
        }
        EditCommand::SetSceneColorGrade { grade } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut g = grade.clone();
            g.exposure = g.exposure.clamp(-3.0, 3.0);
            g.contrast = g.contrast.clamp(0.2, 2.5);
            g.saturation = g.saturation.clamp(0.0, 2.5);
            g.temperature = g.temperature.clamp(-1.0, 1.0);
            g.tint = g.tint.clamp(-1.0, 1.0);
            g.lut_intensity = g.lut_intensity.clamp(0.0, 1.0);
            for ch in &mut g.lift {
                *ch = ch.clamp(-0.5, 0.5);
            }
            for ch in &mut g.gamma {
                *ch = ch.clamp(-0.5, 0.5);
            }
            for ch in &mut g.gain {
                *ch = ch.clamp(0.2, 2.5);
            }
            scene.color_grade = g;
            Ok(())
        }
        EditCommand::SetExpression {
            item_id,
            property,
            expression,
        } => {
            let item = find_item_mut(project, *item_id)?;
            match expression.as_deref().map(str::trim) {
                None | Some("") => {
                    item.expressions.remove(property);
                }
                Some(src) => {
                    compile_expression(src)
                        .map_err(|e| CommandError::InvalidExpression(e.to_string()))?;
                    item.expressions.insert(property.clone(), src.to_string());
                }
            }
            Ok(())
        }
        EditCommand::UpdateTemplateInstance {
            item_id,
            values,
            duration_sec,
        } => {
            let (precomp_scene_id, binding_snap, old_dur, new_dur) = {
                let item = find_item_mut(project, *item_id)?;
                let ItemKind::Precomp { scene_id } = item.kind else {
                    return Ok(());
                };
                let Some(binding) = item.template_binding.as_mut() else {
                    return Ok(());
                };
                for (k, v) in values {
                    binding.values.insert(k.clone(), v.clone());
                }
                let prev_dur = binding.current_duration_sec.max(0.1);
                let target_dur = duration_sec
                    .map(|d| d.clamp(0.3, 600.0))
                    .unwrap_or(prev_dur);
                if (target_dur - prev_dur).abs() > 1e-4 {
                    binding.current_duration_sec = target_dur;
                    item.duration = RationalTime::from_seconds(target_dur);
                }
                (scene_id, binding.clone(), prev_dur, target_dur)
            };

            if let Some(nested) = project.scenes.iter_mut().find(|s| s.id == precomp_scene_id) {
                apply_template_values_to_scene(
                    nested,
                    &binding_snap.exposed_params,
                    &binding_snap.values,
                );
                if (new_dur - old_dur).abs() > 1e-4 {
                    apply_responsive_time_stretch(
                        nested,
                        old_dur,
                        new_dur,
                        binding_snap.responsive_time.intro_sec,
                        binding_snap.responsive_time.outro_sec,
                    );
                }
            }
            Ok(())
        }
        EditCommand::SetParticleConfig { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::ParticleEmitter { config: dst } = &mut item.kind {
                let mut c = *config;
                c.count = c.count.clamp(1, 600);
                c.speed = c.speed.clamp(0.0, 4000.0);
                c.size_px = c.size_px.clamp(1.0, 200.0);
                c.spread_deg = c.spread_deg.clamp(0.0, 360.0);
                c.lifetime_sec = c.lifetime_sec.clamp(0.1, 30.0);
                c.depth_3d = c.depth_3d.clamp(0.0, 1.0);
                *dst = c;
            }
            Ok(())
        }
        EditCommand::SetVectorShape { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::VectorShape { config: dst } = &mut item.kind {
                let mut c = config.clone();
                c.width = c.width.clamp(8.0, 8192.0);
                c.height = c.height.clamp(8.0, 8192.0);
                c.stroke_width = c.stroke_width.clamp(0.0, 128.0);
                c.dash_length = c.dash_length.clamp(0.0, 500.0);
                c.dash_gap = c.dash_gap.clamp(0.0, 500.0);
                c.trim_start = c.trim_start.clamp(0.0, 1.0);
                c.trim_end = c.trim_end.clamp(0.0, 1.0);
                *dst = c;
            }
            Ok(())
        }
        EditCommand::SetMotionBlur { motion_blur } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut mb = *motion_blur;
            mb.shutter_angle_deg = mb.shutter_angle_deg.clamp(0.0, 360.0);
            mb.shutter_phase_deg = mb.shutter_phase_deg.clamp(-180.0, 180.0);
            mb.preview_samples = mb.preview_samples.clamp(1, 8);
            mb.export_samples = mb.export_samples.clamp(1, 16);
            scene.motion_blur = mb;
            Ok(())
        }
        EditCommand::SetGroupId { item_ids, group_id } => {
            for &id in item_ids {
                if let Ok(item) = find_item_mut(project, id) {
                    item.group_id = *group_id;
                }
            }
            Ok(())
        }
        EditCommand::SlipTrim { item_id, delta } => {
            let item = find_item_mut(project, *item_id)?;
            match &mut item.kind {
                ItemKind::Video { source_offset, .. } | ItemKind::Audio { source_offset, .. } => {
                    *source_offset = (*source_offset + *delta).max(RationalTime::ZERO);
                }
                _ => {}
            }
            Ok(())
        }
        EditCommand::RollTrim {
            left_item_id,
            right_item_id,
            delta,
        } => {
            let min_dur = RationalTime::from_seconds(0.05);
            let (left_dur, right_start, right_dur) = {
                let left = find_item_ref(project, *left_item_id)?;
                let right = find_item_ref(project, *right_item_id)?;
                (left.duration, right.start, right.duration)
            };
            let max_pos = (right_dur - min_dur).max(RationalTime::ZERO);
            let max_neg = (min_dur - left_dur).min(RationalTime::ZERO);
            let clamped = (*delta).clamp(max_neg, max_pos);

            let left = find_item_mut(project, *left_item_id)?;
            left.duration = (left.duration + clamped).max(min_dur);

            let right = find_item_mut(project, *right_item_id)?;
            right.start = (right_start + clamped).max(RationalTime::ZERO);
            right.duration = (right_dur - clamped).max(min_dur);
            match &mut right.kind {
                ItemKind::Video { source_offset, .. } | ItemKind::Audio { source_offset, .. } => {
                    *source_offset = (*source_offset + clamped).max(RationalTime::ZERO);
                }
                _ => {}
            }
            Ok(())
        }
        EditCommand::SlideTrim {
            item_id,
            left_neighbor_id,
            right_neighbor_id,
            delta,
        } => {
            let min_dur = RationalTime::from_seconds(0.05);
            let item_start = find_item_ref(project, *item_id)?.start;
            let mut clamped = *delta;
            if item_start + clamped < RationalTime::ZERO {
                clamped = RationalTime::ZERO - item_start;
            }
            if let Some(lid) = left_neighbor_id
                && let Ok(l) = find_item_ref(project, *lid)
            {
                let min_delta = min_dur - l.duration;
                if clamped < min_delta {
                    clamped = min_delta;
                }
            }
            if let Some(rid) = right_neighbor_id
                && let Ok(r) = find_item_ref(project, *rid)
            {
                let max_delta = r.duration - min_dur;
                if clamped > max_delta {
                    clamped = max_delta;
                }
            }

            let mid = find_item_mut(project, *item_id)?;
            mid.start = (mid.start + clamped).max(RationalTime::ZERO);

            if let Some(lid) = left_neighbor_id
                && let Ok(l) = find_item_mut(project, *lid)
            {
                l.duration = (l.duration + clamped).max(min_dur);
            }
            if let Some(rid) = right_neighbor_id
                && let Ok(r) = find_item_mut(project, *rid)
            {
                r.start = (r.start + clamped).max(RationalTime::ZERO);
                r.duration = (r.duration - clamped).max(min_dur);
                match &mut r.kind {
                    ItemKind::Video { source_offset, .. }
                    | ItemKind::Audio { source_offset, .. } => {
                        *source_offset = (*source_offset + clamped).max(RationalTime::ZERO);
                    }
                    _ => {}
                }
            }
            Ok(())
        }
        EditCommand::SetLayerMixer {
            layer_index,
            volume,
            pan,
            muted,
            solo,
            bus,
            send_reverb,
        } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let layer = scene
                .layers
                .get_mut(*layer_index)
                .ok_or(CommandError::InvalidLayer(*layer_index))?;
            layer.volume = volume.clamp(0.0, 4.0);
            layer.pan = pan.clamp(-1.0, 1.0);
            layer.muted = *muted;
            layer.solo = *solo;
            layer.bus = if bus.trim().is_empty() {
                "Master".to_string()
            } else {
                bus.trim().to_string()
            };
            layer.send_reverb = send_reverb.clamp(0.0, 1.0);
            Ok(())
        }
        EditCommand::SetNodeGraph { graph } => {
            if let Err(err) = graph.topological_order() {
                return Err(CommandError::InvalidExpression(err.to_string()));
            }
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            scene.node_graph = graph.clone();
            Ok(())
        }
        EditCommand::SetOpticalFlowInterp { item_id, enabled } => {
            let item = find_item_mut(project, *item_id)?;
            item.playback.optical_flow_interp = *enabled;
            Ok(())
        }
        EditCommand::SetReviewComments { comments } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut sorted = comments.clone();
            sorted.sort_by_key(|c| c.time);
            scene.review_comments = sorted;
            Ok(())
        }
        EditCommand::ReplaceProjectSnapshot {
            project: next_project,
        } => {
            *project = (**next_project).clone();
            Ok(())
        }
        EditCommand::SetCornerPin {
            item_id,
            corner_pin,
        } => {
            let item = find_item_mut(project, *item_id)?;
            item.corner_pin = *corner_pin;
            Ok(())
        }
        EditCommand::SetKineticSubtitle { item_id, kinetic } => {
            let item = find_item_mut(project, *item_id)?;
            item.kinetic_subtitle = kinetic.clone();
            Ok(())
        }
        EditCommand::SetMesh3D { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::Mesh3D { config: ref mut c } = item.kind {
                *c = config.clone();
                Ok(())
            } else {
                Err(CommandError::ItemNotFound(*item_id))
            }
        }
        EditCommand::SetItemPhonemes { item_id, phonemes } => {
            let item = find_item_mut(project, *item_id)?;
            item.phoneme_segments = phonemes.clone();
            Ok(())
        }
        EditCommand::SetColorManagement { config } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut c = *config;
            c.exposure_stops = c.exposure_stops.clamp(-4.0, 4.0);
            scene.color_management = c;
            Ok(())
        }
        EditCommand::SetGenerativeBg { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::GenerativeBg { config: ref mut c } = item.kind {
                let mut next = *config;
                next.speed = next.speed.clamp(0.0, 10.0);
                next.scale = next.scale.clamp(0.1, 10.0);
                next.complexity = next.complexity.clamp(1, 8);
                next.width = next.width.clamp(32.0, 8192.0);
                next.height = next.height.clamp(32.0, 8192.0);
                *c = next;
                Ok(())
            } else {
                Err(CommandError::ItemNotFound(*item_id))
            }
        }
        EditCommand::SetRig2D { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::RiggedPuppet { config: ref mut c } = item.kind {
                let mut next = (**config).clone();
                next.width = next.width.clamp(64.0, 4096.0);
                next.height = next.height.clamp(64.0, 4096.0);
                next.motion_speed = next.motion_speed.clamp(0.0, 6.0);
                next.motion_amp = next.motion_amp.clamp(0.0, 3.0);
                **c = next;
                Ok(())
            } else {
                Err(CommandError::ItemNotFound(*item_id))
            }
        }
        EditCommand::SetSpeedRamp {
            item_id,
            speed_ramp,
        } => {
            let item = find_item_mut(project, *item_id)?;
            let mut sr = speed_ramp.clone();
            sr.knots = sr.normalized();
            item.speed_ramp = sr;
            Ok(())
        }
        EditCommand::SetCinemaFinishing { config } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut next = config.clone();
            next.halation_strength = next.halation_strength.clamp(0.0, 1.5);
            next.halation_threshold = next.halation_threshold.clamp(0.3, 1.0);
            next.anamorphic_flare = next.anamorphic_flare.clamp(0.0, 1.5);
            next.film_grain = next.film_grain.clamp(0.0, 0.5);
            next.vignette = next.vignette.clamp(0.0, 1.0);
            scene.cinema_finishing = next;
            Ok(())
        }
        EditCommand::SetDataChart { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::DataChart { config: ref mut c } = item.kind {
                let mut next = (**config).clone();
                next.width = next.width.clamp(120.0, 4096.0);
                next.height = next.height.clamp(80.0, 4096.0);
                next.animate_in_sec = next.animate_in_sec.clamp(0.05, 30.0);
                **c = next;
                Ok(())
            } else {
                Err(CommandError::ItemNotFound(*item_id))
            }
        }
        EditCommand::SetVfxPaint { item_id, vfx_paint } => {
            let item = find_item_mut(project, *item_id)?;
            let mut next = vfx_paint.clone();
            for stroke in &mut next.strokes {
                stroke.radius_px = stroke.radius_px.clamp(1.0, 256.0);
                stroke.hardness = stroke.hardness.clamp(0.05, 1.0);
                stroke.opacity = stroke.opacity.clamp(0.0, 1.0);
                stroke.write_on_progress = stroke.write_on_progress.clamp(0.0, 1.0);
            }
            item.vfx_paint = next;
            Ok(())
        }
        EditCommand::SetAssetConformMeta { asset_id, meta } => {
            project.conform_meta.insert(*asset_id, meta.clone());
            Ok(())
        }
        EditCommand::ConformClipSource {
            item_id,
            new_asset_id,
            new_source_offset,
        } => {
            let item = find_item_mut(project, *item_id)?;
            let clamped_off = (*new_source_offset).max(RationalTime::ZERO);
            match &mut item.kind {
                ItemKind::Video {
                    asset_id,
                    source_offset,
                    ..
                }
                | ItemKind::Audio {
                    asset_id,
                    source_offset,
                    ..
                } => {
                    *asset_id = *new_asset_id;
                    *source_offset = clamped_off;
                    Ok(())
                }
                ItemKind::Image { asset_id } => {
                    *asset_id = *new_asset_id;
                    Ok(())
                }
                _ => Err(CommandError::ItemNotFound(*item_id)),
            }
        }
        EditCommand::SetAbCompare { ab_compare } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut next = ab_compare.clone();
            next.split_ratio = next.split_ratio.clamp(0.05, 0.95);
            next.diff_gain = next.diff_gain.clamp(1.0, 16.0);
            next.reference_sec = next.reference_sec.max(0.0);
            scene.ab_compare = next;
            Ok(())
        }
        EditCommand::SetRotoBrush {
            item_id,
            roto_brush,
        } => {
            let item = find_item_mut(project, *item_id)?;
            let mut next = roto_brush.clone();
            next.edge_snap_radius_px = next.edge_snap_radius_px.clamp(1, 48);
            next.edge_weight = next.edge_weight.clamp(0.05, 1.0);
            next.keyframes.sort_by_key(|k| k.time);
            item.roto_brush = next;
            Ok(())
        }
        EditCommand::SetSvgMorph { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::SvgMorph { config: ref mut c } = item.kind {
                let mut next = (**config).clone();
                next.width = next.width.clamp(32.0, 4096.0);
                next.height = next.height.clamp(32.0, 4096.0);
                next.morph_progress = next.morph_progress.clamp(0.0, 1.0);
                next.auto_morph_hz = next.auto_morph_hz.clamp(0.0, 8.0);
                next.stroke_width = next.stroke_width.clamp(0.0, 64.0);
                next.trim_start = next.trim_start.clamp(0.0, 1.0);
                next.trim_end = next.trim_end.clamp(0.0, 1.0);
                next.sample_count = next.sample_count.clamp(12, 256);
                **c = next;
                Ok(())
            } else {
                Err(CommandError::ItemNotFound(*item_id))
            }
        }
        EditCommand::SetTextOnPath {
            item_id,
            text_on_path,
        } => {
            let item = find_item_mut(project, *item_id)?;
            let mut next = text_on_path.clone();
            next.path_offset = next.path_offset.clamp(-1.0, 2.0);
            next.animate_speed = next.animate_speed.clamp(-2.0, 2.0);
            next.letter_spacing_px = next.letter_spacing_px.clamp(-24.0, 96.0);
            next.vertical_offset_px = next.vertical_offset_px.clamp(-200.0, 200.0);
            item.text_on_path = next;
            Ok(())
        }
        EditCommand::SetWarpDisplace {
            item_id,
            warp_displace,
        } => {
            let item = find_item_mut(project, *item_id)?;
            let mut next = warp_displace.clone();
            next.amount = next.amount.clamp(-1.0, 1.0);
            next.frequency = next.frequency.clamp(0.2, 16.0);
            next.speed = next.speed.clamp(-10.0, 10.0);
            next.luma_displace_px = next.luma_displace_px.clamp(-64.0, 64.0);
            next.ensure_grid_size();
            item.warp_displace = next;
            Ok(())
        }
        EditCommand::SetOpticalVfx { optical_vfx } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut next = optical_vfx.clone();
            next.focus_depth = next.focus_depth.clamp(0.0, 1.0);
            next.focus_band = next.focus_band.clamp(0.02, 0.6);
            next.bokeh_radius_px = next.bokeh_radius_px.clamp(0.0, 20.0);
            next.bokeh_highlight_boost = next.bokeh_highlight_boost.clamp(0.0, 3.0);
            next.bokeh_blades = if next.bokeh_blades == 0 {
                0
            } else {
                next.bokeh_blades.clamp(5, 8)
            };
            next.light_pos = [
                next.light_pos[0].clamp(-0.5, 1.5),
                next.light_pos[1].clamp(-0.5, 1.5),
            ];
            next.god_rays_intensity = next.god_rays_intensity.clamp(0.0, 2.0);
            next.god_rays_decay = next.god_rays_decay.clamp(0.80, 0.99);
            next.god_rays_threshold = next.god_rays_threshold.clamp(0.1, 2.0);
            next.god_rays_samples = next.god_rays_samples.clamp(8, 64);
            next.k1 = next.k1.clamp(-0.6, 0.6);
            next.k2 = next.k2.clamp(-0.4, 0.4);
            next.p1 = next.p1.clamp(-0.15, 0.15);
            next.p2 = next.p2.clamp(-0.15, 0.15);
            next.chromatic_aberration = next.chromatic_aberration.clamp(-0.05, 0.05);
            scene.optical_vfx = next;
            Ok(())
        }
        EditCommand::SetAdrSession { adr_session } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut next = adr_session.clone();
            next.cues.sort_by_key(|c| c.start_time);
            if !next.cues.is_empty() {
                next.active_cue_idx = next.active_cue_idx.min(next.cues.len() - 1);
            } else {
                next.active_cue_idx = 0;
            }
            scene.adr_session = next;
            Ok(())
        }
        EditCommand::SetSpatialBus { spatial_bus } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut next = spatial_bus.clone();
            next.azimuth_deg = next.azimuth_deg.clamp(-180.0, 180.0);
            next.elevation_deg = next.elevation_deg.clamp(-90.0, 90.0);
            next.lfe_crossover_hz = next.lfe_crossover_hz.clamp(40.0, 160.0);
            next.center_dialogue_boost = next.center_dialogue_boost.clamp(0.0, 1.0);
            next.room_width = next.room_width.clamp(0.0, 1.0);
            next.head_yaw_deg = next.head_yaw_deg.clamp(-180.0, 180.0);
            next.head_pitch_deg = next.head_pitch_deg.clamp(-90.0, 90.0);
            scene.spatial_bus = next;
            Ok(())
        }
        EditCommand::SetLiveSwitcher { live_switcher } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut next = live_switcher.clone();
            next.transition_progress = next.transition_progress.clamp(0.0, 1.0);
            if !next.sources.is_empty() {
                let max_i = next.sources.len() - 1;
                next.program_channel = next.program_channel.min(max_i);
                next.preview_channel = next.preview_channel.min(max_i);
            }
            scene.live_switcher = next;
            Ok(())
        }
        EditCommand::SetVtuberAvatar { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            let mut next = (**config).clone();
            next.width = next.width.clamp(64.0, 2048.0);
            next.height = next.height.clamp(64.0, 2048.0);
            next.grid_cols = next.grid_cols.clamp(3, 18);
            next.grid_rows = next.grid_rows.clamp(3, 18);
            item.kind = ItemKind::VtuberAvatar {
                config: Box::new(next),
            };
            Ok(())
        }
        EditCommand::SetHardwareController { hardware_ctrl } => {
            project.hardware_ctrl = hardware_ctrl.clone();
            Ok(())
        }
        EditCommand::SetSmartDamIndex { smart_dam } => {
            project.smart_dam = smart_dam.clone();
            Ok(())
        }
        EditCommand::RelinkAssets { relinks } => {
            for (aid, new_path) in relinks {
                if let Some(asset) = project.assets.iter_mut().find(|a| a.id == *aid) {
                    asset.fallback_absolute_path = Some(std::path::PathBuf::from(new_path));
                    asset.relative_path = None;
                }
                if let Some(rec) = project
                    .smart_dam
                    .records
                    .iter_mut()
                    .find(|r| r.asset_id == Some(*aid))
                {
                    rec.file_path = new_path.clone();
                }
            }
            Ok(())
        }
        EditCommand::Batch { commands, .. } => {
            for cmd in commands {
                apply_command(project, cmd)?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{CubicBezier, Interpolation};
    use crate::project::{ProjectSettings, RgbaColor, ShapeKind};
    use tempfile::tempdir;
    use uuid::Uuid;

    fn sample_item(name: &str) -> TimelineItem {
        TimelineItem::new(
            name,
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 100.0,
                height: 100.0,
                color: RgbaColor::WHITE,
            },
        )
    }

    #[test]
    fn undo_redo_coalescing_and_gui_drag_lock() {
        let mut project = Project::new("BusTest", ProjectSettings::default());
        let mut bus = CommandBus::new();
        let item = sample_item("Clip1");
        let id = item.id;

        bus.execute(
            &mut project,
            EditCommand::AddObject {
                layer_index: 0,
                item,
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();

        // 連続ドラッグ移動を同じ coalesce_key で2回実行 -> Undo単位は1つだけ増える
        for sec in [1.0, 1.5, 2.0] {
            bus.execute(
                &mut project,
                EditCommand::MoveObject {
                    item_id: id,
                    target_layer_index: 0,
                    new_start: RationalTime::from_seconds(sec),
                },
                CommandSource::Gui,
                Some("drag_move"),
            )
            .unwrap();
        }
        assert_eq!(bus.undo_len(), 2); // AddObject + 統合されたMoveObject

        // GUIドラッグ中はMCP操作をブロック
        bus.set_gui_drag_locked(true);
        let err = bus
            .execute(
                &mut project,
                EditCommand::DeleteObject { item_id: id },
                CommandSource::Mcp,
                None,
            )
            .unwrap_err();
        assert!(matches!(err, CommandError::GuiDragLocked));
        bus.set_gui_drag_locked(false);

        // Undoで元の位置 (0.0s) に戻る
        assert!(bus.undo(&mut project));
        assert_eq!(
            project.active_scene().unwrap().layers[0].items[0].start,
            RationalTime::ZERO
        );
        assert!(bus.redo(&mut project));
        assert_eq!(
            project.active_scene().unwrap().layers[0].items[0].start,
            RationalTime::from_seconds(2.0)
        );
    }

    #[test]
    fn split_and_crash_recovery_from_history_jsonl() {
        let tmp = tempdir().unwrap();
        let base_project = Project::new("RecoverTest", ProjectSettings::default());
        let folder = ProjectFolder::init(tmp.path().join("proj"), &base_project).unwrap();

        let mut live_project = base_project.clone();
        let mut bus = CommandBus::with_folder(folder.clone());
        let item = sample_item("SplitMe");
        let id = item.id;
        let right_id = Uuid::new_v4();

        bus.execute(
            &mut live_project,
            EditCommand::Batch {
                label: "Add and Split".into(),
                commands: vec![
                    EditCommand::AddObject {
                        layer_index: 0,
                        item,
                    },
                    EditCommand::SplitObject {
                        item_id: id,
                        split_at: RationalTime::from_seconds(1.0),
                        new_right_id: right_id,
                    },
                ],
            },
            CommandSource::Mcp,
            None,
        )
        .unwrap();
        assert_eq!(
            live_project.active_scene().unwrap().layers[0].items.len(),
            2
        );

        // クラッシュ想定: project.json は初期状態のまま、history.jsonl をリプレイして完全復旧
        let mut recovered = folder.load_project().unwrap();
        let history = folder.read_history().unwrap();
        let replayed = CommandBus::replay_history(&mut recovered, &history).unwrap();
        assert_eq!(replayed, 1);
        assert_eq!(recovered, live_project);
    }

    #[test]
    fn smart_precompose_and_keyframe_are_undoable() {
        let mut project = Project::new("PrecompTest", ProjectSettings::default());
        let mut bus = CommandBus::new();

        let mut a = sample_item("Layer0Rect");
        a.start = RationalTime::from_seconds(1.0);
        let mut b = sample_item("Layer1Rect");
        b.start = RationalTime::from_seconds(1.5);
        let a_id = a.id;
        let b_id = b.id;

        bus.execute(
            &mut project,
            EditCommand::Batch {
                label: "Seed".into(),
                commands: vec![
                    EditCommand::AddObject {
                        layer_index: 0,
                        item: a,
                    },
                    EditCommand::AddObject {
                        layer_index: 1,
                        item: b,
                    },
                    EditCommand::SetKeyframe {
                        item_id: a_id,
                        property: "x".into(),
                        keyframe: Keyframe {
                            time: RationalTime::ZERO,
                            value: 0.0,
                            interpolation: Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
                        },
                    },
                    EditCommand::SetKeyframe {
                        item_id: a_id,
                        property: "x".into(),
                        keyframe: Keyframe {
                            time: RationalTime::from_seconds(1.0),
                            value: 100.0,
                            interpolation: Interpolation::Linear,
                        },
                    },
                ],
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();

        // ワンクリック結合（Smart Precompose）
        let new_scene_id = Uuid::new_v4();
        let precomp_item_id = Uuid::new_v4();
        bus.execute(
            &mut project,
            EditCommand::PrecomposeSelection {
                item_ids: vec![a_id, b_id],
                new_scene_id,
                precomp_name: "Motion Comp 1".into(),
                precomp_item_id,
                target_layer_index: 0,
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();

        assert_eq!(project.scenes.len(), 2);
        let main_scene = project.active_scene().unwrap();
        assert_eq!(main_scene.layers[0].items.len(), 1);
        assert_eq!(main_scene.layers[1].items.len(), 0);
        assert_eq!(
            main_scene.layers[0].items[0].start,
            RationalTime::from_seconds(1.0)
        );

        let nested = project.scene(new_scene_id).unwrap();
        assert_eq!(nested.layers[0].items[0].start, RationalTime::ZERO);
        assert_eq!(
            nested.layers[1].items[0].start,
            RationalTime::from_seconds(0.5)
        );

        // Undo で元の2レイヤー構成へ完全復元
        assert!(bus.undo(&mut project));
        assert_eq!(project.scenes.len(), 1);
        assert_eq!(project.active_scene().unwrap().layers[0].items.len(), 1);
        assert_eq!(project.active_scene().unwrap().layers[1].items.len(), 1);
    }

    #[test]
    fn undo_stack_respects_max_depth_and_increments_revision() {
        let mut project = Project::new("CapTest", ProjectSettings::default());
        let mut bus = CommandBus::new().with_max_undo_depth(3);
        assert_eq!(bus.revision(), 0);

        for i in 0..5 {
            bus.execute(
                &mut project,
                EditCommand::AddObject {
                    layer_index: 0,
                    item: sample_item(&format!("Item {i}")),
                },
                CommandSource::Gui,
                None,
            )
            .unwrap();
        }
        assert_eq!(bus.undo_len(), 3);
        assert_eq!(bus.revision(), 5);

        assert!(bus.undo(&mut project));
        assert_eq!(bus.revision(), 6);
        assert!(bus.redo(&mut project));
        assert_eq!(bus.revision(), 7);
    }

    #[test]
    fn effect_chain_reorder_paste_and_audio_dsp_commands_are_undoable() {
        let mut project = Project::new("FxTest", ProjectSettings::default());
        let mut bus = CommandBus::new();
        let item = sample_item("Clip");
        let item_id = item.id;
        bus.execute(
            &mut project,
            EditCommand::AddObject {
                layer_index: 0,
                item,
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();

        let eff_a = EffectNode {
            id: uuid::Uuid::new_v4(),
            effect_type: "blur".into(),
            enabled: true,
            params: BTreeMap::new(),
        };
        let eff_b = EffectNode {
            id: uuid::Uuid::new_v4(),
            effect_type: "mosaic".into(),
            enabled: true,
            params: BTreeMap::new(),
        };
        let eff_a_id = eff_a.id;
        bus.execute(
            &mut project,
            EditCommand::PasteEffects {
                item_id,
                effects: vec![eff_a, eff_b],
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();

        bus.execute(
            &mut project,
            EditCommand::ReorderEffect {
                item_id,
                from_index: 0,
                to_index: 1,
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();
        let chain = &project.active_scene().unwrap().layers[0].items[0].effect_chain;
        assert_eq!(chain[0].effect_type, "mosaic");
        assert_eq!(chain[1].effect_type, "blur");

        bus.execute(
            &mut project,
            EditCommand::ToggleEffect {
                item_id,
                effect_id: eff_a_id,
                enabled: false,
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();
        assert!(!project.active_scene().unwrap().layers[0].items[0].effect_chain[1].enabled);

        bus.execute(
            &mut project,
            EditCommand::RemoveEffect {
                item_id,
                effect_id: eff_a_id,
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();
        assert_eq!(
            project.active_scene().unwrap().layers[0].items[0]
                .effect_chain
                .len(),
            1
        );

        assert!(bus.undo(&mut project));
        assert_eq!(
            project.active_scene().unwrap().layers[0].items[0]
                .effect_chain
                .len(),
            2
        );
    }

    #[test]
    fn set_scene_settings_rescales_items_and_is_undoable() {
        use crate::project::ScenePreset;
        use crate::time::FrameRate;

        let mut project = Project::new(
            "SceneSettings",
            ProjectSettings {
                width: 1280,
                height: 720,
                fps: FrameRate::FPS_60,
                ..Default::default()
            },
        );
        let mut bus = CommandBus::new();
        let mut item = sample_item("Box");
        item.transform.x = 640.0;
        item.transform.y = 360.0;
        bus.execute(
            &mut project,
            EditCommand::AddObject {
                layer_index: 0,
                item,
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();

        let (sw, sh) = ScenePreset::Shorts9x16.dimensions();
        bus.execute(
            &mut project,
            EditCommand::SetSceneSettings {
                width: sw,
                height: sh,
                fps: FrameRate::FPS_30,
                rescale_items: true,
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();

        let sc = project.active_scene().unwrap();
        assert_eq!((sc.width, sc.height), (1080, 1920));
        assert_eq!(sc.fps, FrameRate::FPS_30);
        // 元の中心 (640, 360) -> 新中心 (540, 960) へ正確にリフレームされる
        assert!((sc.layers[0].items[0].transform.x - 540.0).abs() < 0.5);
        assert!((sc.layers[0].items[0].transform.y - 960.0).abs() < 0.5);

        // Undo で 1280x720・60fps・元座標へ完全復元
        assert!(bus.undo(&mut project));
        let sc = project.active_scene().unwrap();
        assert_eq!((sc.width, sc.height), (1280, 720));
        assert_eq!(sc.fps, FrameRate::FPS_60);
        assert!((sc.layers[0].items[0].transform.x - 640.0).abs() < 0.5);
    }

    #[test]
    fn filter_group_spectrum_and_marker_update_commands_are_undoable() {
        let mut project = Project::new("V071CmdTest", ProjectSettings::default());
        let mut bus = CommandBus::new();

        let filter_item = TimelineItem::new(
            "Filter",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::FilterObject {
                effect_type: "brightness".into(),
                scope: FilterTargetScope::ImmediateLayer,
                params: BTreeMap::from([("gain".into(), 1.2)]),
            },
        );
        let group_item = TimelineItem::new(
            "Group",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::GroupControl {
                target_layer_count: 1,
            },
        );
        let spec_item = TimelineItem::new(
            "Spectrum",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::AudioSpectrum {
                linked_audio_layer: 3,
                width: 400.0,
                height: 120.0,
                bar_count: 16,
                color: RgbaColor::WHITE,
                peak_color: RgbaColor::BLACK,
                sensitivity: 1.0,
            },
        );
        let marker = TimelineMarker::new(RationalTime::from_seconds(5.0), "旧タイトル");
        let (fid, gid, sid, mid) = (filter_item.id, group_item.id, spec_item.id, marker.id);

        bus.execute(
            &mut project,
            EditCommand::Batch {
                label: "seed".into(),
                commands: vec![
                    EditCommand::AddObject {
                        layer_index: 2,
                        item: filter_item,
                    },
                    EditCommand::AddObject {
                        layer_index: 2,
                        item: group_item,
                    },
                    EditCommand::AddObject {
                        layer_index: 1,
                        item: spec_item,
                    },
                    EditCommand::AddMarker { marker },
                ],
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();

        bus.execute(
            &mut project,
            EditCommand::SetFilterObject {
                item_id: fid,
                effect_type: "blur".into(),
                scope: FilterTargetScope::LayerRange { count: 2 },
                params: BTreeMap::from([("radius".into(), 6.0)]),
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();
        bus.execute(
            &mut project,
            EditCommand::SetGroupControl {
                item_id: gid,
                target_layer_count: 3,
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();
        let cyan = RgbaColor {
            r: 0.1,
            g: 0.8,
            b: 1.0,
            a: 1.0,
        };
        bus.execute(
            &mut project,
            EditCommand::SetAudioSpectrum {
                item_id: sid,
                linked_audio_layer: 2,
                width: 640.0,
                height: 180.0,
                bar_count: 32,
                color: cyan,
                peak_color: RgbaColor::WHITE,
                sensitivity: 2.5,
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();
        bus.execute(
            &mut project,
            EditCommand::UpdateMarker {
                marker_id: mid,
                label: "本編ハイライト".into(),
                color: cyan,
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();

        let sc = project.active_scene().unwrap();
        assert_eq!(sc.markers[0].label, "本編ハイライト");
        assert_eq!(sc.markers[0].color, cyan);
        assert!(matches!(
            &sc.layers[2].items[0].kind,
            ItemKind::FilterObject {
                effect_type,
                scope: FilterTargetScope::LayerRange { count: 2 },
                ..
            } if effect_type == "blur"
        ));
        assert!(matches!(
            sc.layers[2].items[1].kind,
            ItemKind::GroupControl {
                target_layer_count: 3
            }
        ));
        assert!(matches!(
            sc.layers[1].items[0].kind,
            ItemKind::AudioSpectrum {
                linked_audio_layer: 2,
                bar_count: 32,
                ..
            }
        ));

        // Undo 4 回で元の状態へ戻る
        for _ in 0..4 {
            assert!(bus.undo(&mut project));
        }
        let sc = project.active_scene().unwrap();
        assert_eq!(sc.markers[0].label, "旧タイトル");
        assert!(matches!(
            sc.layers[2].items[1].kind,
            ItemKind::GroupControl {
                target_layer_count: 1
            }
        ));
    }
}
