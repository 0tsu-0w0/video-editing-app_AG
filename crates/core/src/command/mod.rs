//! 非破壊・決定論的編集コマンド群と Undo/Redo・クラッシュ復旧 `CommandBus`
//! （v10.0.0 モジュール分割: `mod.rs` [型定義/CommandBus] / `apply.rs` [コマンド適用器] / `tests.rs`）

mod apply;
#[cfg(test)]
mod tests;

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
use apply::apply_command;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
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
    /// v10.0 #57: アクティブシーンの 360° 全天球＆ステレオスコピック空間 3D 設定を更新する
    SetSpatialVideo {
        spatial_video: crate::spatial_video::SpatialVideoConfig,
    },
    /// v10.0 #58: アクティブシーンの 3D シャドウマップ＆HDRI IBL 環境照明設定を更新する
    SetLighting3DAdv {
        lighting_3d_adv: crate::fluid_cloth::Lighting3DAdvConfig,
    },
    /// v10.0 #58: 決定論 2D Navier-Stokes 流体アイテム設定を更新する
    SetFluidSim {
        item_id: ItemId,
        config: Box<crate::fluid_cloth::FluidSimConfig>,
    },
    /// v10.0 #58: 決定論 PBD クロス物理アイテム設定を更新する
    SetClothSim {
        item_id: ItemId,
        config: Box<crate::fluid_cloth::ClothSimConfig>,
    },
    /// v10.0 #60: プロジェクトの LAN 分散レンダーファーム設定を更新する
    SetRenderFarm {
        render_farm: crate::cinema_os::RenderFarmConfig,
    },
    /// v11.0 #61: アクティブシーンのゼロコピー GPU レンダリンググラフ設定を更新する
    SetGpuRenderGraph {
        gpu_graph: crate::perf_engine::GpuRenderGraphConfig,
    },
    /// v11.0 #62: プロジェクトの適応解像度再生＆スマート事前レンダリング設定を更新する
    SetAdaptivePlayback {
        adaptive_playback: crate::perf_engine::AdaptivePlaybackConfig,
    },
    /// v11.0 #64: プロジェクトのリアルタイム・パフォーマンスプロファイラ HUD 設定を更新する
    SetProfilerHud {
        profiler_hud: crate::perf_engine::ProfilerHudConfig,
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
            Self::SetSpatialVideo { .. } => "set_spatial_video",
            Self::SetLighting3DAdv { .. } => "set_lighting_3d_adv",
            Self::SetFluidSim { .. } => "set_fluid_sim",
            Self::SetClothSim { .. } => "set_cloth_sim",
            Self::SetRenderFarm { .. } => "set_render_farm",
            Self::SetGpuRenderGraph { .. } => "set_gpu_render_graph",
            Self::SetAdaptivePlayback { .. } => "set_adaptive_playback",
            Self::SetProfilerHud { .. } => "set_profiler_hud",
            Self::Batch { .. } => "batch_execute",
        }
    }
}

#[derive(Debug, Clone)]
struct UndoRecord {
    coalesce_key: Option<String>,
    before: Arc<Project>,
    after: Arc<Project>,
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

    /// Undo / Redo スタック上で実際に確保されている一意な `Arc<Project>` スナップショット数（隣接共有による省メモリ計測用）
    pub fn unique_snapshot_count(&self) -> usize {
        let mut ptrs: HashSet<*const Project> =
            HashSet::with_capacity(self.undo_stack.len() + self.redo_stack.len() + 2);
        for rec in self.undo_stack.iter().chain(self.redo_stack.iter()) {
            ptrs.insert(Arc::as_ptr(&rec.before));
            ptrs.insert(Arc::as_ptr(&rec.after));
        }
        ptrs.len()
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

        // コミット確定 & Command Coalescing ＋ 隣接 `Arc<Project>` 共有（Undo メモリ使用量を約 50% 削減）
        self.redo_stack.clear();
        let can_coalesce = coalesce_key.is_some_and(|key| {
            self.undo_stack.back().is_some_and(|last| {
                last.source == source && last.coalesce_key.as_deref() == Some(key)
            })
        });

        if can_coalesce {
            if let Some(last) = self.undo_stack.back_mut() {
                last.after = Arc::new(working.clone());
                last.command = command.clone();
            }
            *project = working;
        } else {
            let shared_prev_after = self
                .undo_stack
                .back()
                .filter(|last| last.after.as_ref() == project)
                .map(|last| Arc::clone(&last.after));
            let after_arc = Arc::new(working.clone());
            let before_arc = match shared_prev_after {
                Some(prev) => {
                    *project = working;
                    prev
                }
                None => Arc::new(std::mem::replace(project, working)),
            };
            if self.undo_stack.len() >= self.max_undo_depth.max(1) {
                self.undo_stack.pop_front();
            }
            self.undo_stack.push_back(UndoRecord {
                coalesce_key: coalesce_key.map(str::to_owned),
                before: before_arc,
                after: after_arc,
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
            *project = (*record.before).clone();
            self.redo_stack.push(record);
            self.revision = self.revision.wrapping_add(1);
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self, project: &mut Project) -> bool {
        if let Some(record) = self.redo_stack.pop() {
            *project = (*record.after).clone();
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
