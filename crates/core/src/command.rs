use crate::folder::{FolderError, HistoryEntry, ProjectFolder};
use crate::project::{EffectId, EffectNode, ItemId, ItemKind, Project, TimelineItem, Transform2D};
use crate::time::RationalTime;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("Active scene not found")]
    NoActiveScene,
    #[error("Layer index out of bounds: {0}")]
    InvalidLayer(usize),
    #[error("Timeline item not found: {0}")]
    ItemNotFound(ItemId),
    #[error("Effect not found: {0}")]
    EffectNotFound(EffectId),
    #[error("Split position {0:?} is outside item bounds")]
    InvalidSplitPosition(RationalTime),
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
    SetTextContent {
        item_id: ItemId,
        content: String,
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
            Self::SetTextContent { .. } => "set_text_content",
            Self::AddEffect { .. } => "add_effect",
            Self::UpdateEffectParams { .. } => "update_effect_params",
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

/// GUI・AI（MCP）・Undo/Redo・Command Coalescing・`history.jsonl` 復旧を一元管理する Command Bus
#[derive(Debug, Default)]
pub struct CommandBus {
    undo_stack: Vec<UndoRecord>,
    redo_stack: Vec<UndoRecord>,
    next_seq: u64,
    gui_drag_locked: bool,
    journal_folder: Option<ProjectFolder>,
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
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            next_seq,
            gui_drag_locked: false,
            journal_folder: Some(folder),
        }
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

        let before = project.clone();
        let mut working = project.clone();
        apply_command(&mut working, &command)?;

        // コミット確定
        *project = working.clone();
        self.redo_stack.clear();

        // Command Coalescing: 連続ドラッグ等の同一キー操作を1つのUndo単位へ集約
        let coalesced = if let (Some(key), Some(last)) = (coalesce_key, self.undo_stack.last_mut())
        {
            if last.source == source && last.coalesce_key.as_deref() == Some(key) {
                last.after = working.clone();
                last.command = command.clone();
                true
            } else {
                false
            }
        } else {
            false
        };

        if !coalesced {
            self.undo_stack.push(UndoRecord {
                coalesce_key: coalesce_key.map(str::to_owned),
                before,
                after: working,
                command: command.clone(),
                source,
            });
        }

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
        if let Some(record) = self.undo_stack.pop() {
            *project = record.before.clone();
            self.redo_stack.push(record);
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self, project: &mut Project) -> bool {
        if let Some(record) = self.redo_stack.pop() {
            *project = record.after.clone();
            self.undo_stack.push(record);
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

fn find_item_mut(project: &mut Project, item_id: ItemId) -> Result<&mut TimelineItem, CommandError> {
    let scene = project.active_scene_mut().ok_or(CommandError::NoActiveScene)?;
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
            let scene = project.active_scene_mut().ok_or(CommandError::NoActiveScene)?;
            let layer = scene
                .layers
                .get_mut(*layer_index)
                .ok_or(CommandError::InvalidLayer(*layer_index))?;
            layer.items.push(item.clone());
            Ok(())
        }
        EditCommand::DeleteObject { item_id } => {
            let scene = project.active_scene_mut().ok_or(CommandError::NoActiveScene)?;
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
            let scene = project.active_scene_mut().ok_or(CommandError::NoActiveScene)?;
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
            let scene = project.active_scene_mut().ok_or(CommandError::NoActiveScene)?;
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
    use crate::project::{ProjectSettings, RgbaColor, ShapeKind};
    use tempfile::tempdir;
    use uuid::Uuid;

    fn sample_item(name: &str) -> TimelineItem {
        TimelineItem {
            id: Uuid::new_v4(),
            name: name.to_string(),
            start: RationalTime::ZERO,
            duration: RationalTime::from_seconds(2.0),
            transform: Transform2D::default(),
            effect_chain: vec![],
            kind: ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 100.0,
                height: 100.0,
                color: RgbaColor::WHITE,
            },
        }
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
        assert_eq!(project.active_scene().unwrap().layers[0].items[0].start, RationalTime::ZERO);
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
        assert_eq!(live_project.active_scene().unwrap().layers[0].items.len(), 2);

        // クラッシュ想定: project.json は初期状態のまま、history.jsonl をリプレイして完全復旧
        let mut recovered = folder.load_project().unwrap();
        let history = folder.read_history().unwrap();
        let replayed = CommandBus::replay_history(&mut recovered, &history).unwrap();
        assert_eq!(replayed, 1);
        assert_eq!(recovered, live_project);
    }
}
