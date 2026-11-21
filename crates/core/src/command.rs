use crate::animation::Keyframe;
use crate::camera::{Camera3D, Transform3D};
use crate::folder::{FolderError, HistoryEntry, ProjectFolder};
use crate::lipsync::CharacterSpriteConfig;
use crate::preset::ColorPalette;
use crate::project::{
    AudioEffectNode, BlendMode, EffectId, EffectNode, ItemId, ItemKind, Layer, Project, RgbaColor,
    Scene, SceneId, TextAnimator, TextStyle, TimelineItem, TimelineMarker, TrackMatte, Transform2D,
    TransitionKind,
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
                markers: Vec::new(),
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
                            | ItemKind::CharacterSprite { width, height, .. } => {
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
}
