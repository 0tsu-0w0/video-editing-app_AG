use std::sync::{Arc, Mutex};
use ve_core::{CommandBus, ItemId, Project, ProjectFolder, RationalTime};

/// GUI と 内蔵 MCP サーバーが共有するプロジェクト・CommandBus・選択状態
pub struct EditorSession {
    pub project: Project,
    pub bus: CommandBus,
    pub selected_items: Vec<ItemId>,
    pub playhead: RationalTime,
    pub folder: Option<ProjectFolder>,
}

impl EditorSession {
    pub fn new(project: Project) -> Self {
        Self {
            project,
            bus: CommandBus::new(),
            selected_items: Vec::new(),
            playhead: RationalTime::ZERO,
            folder: None,
        }
    }

    pub fn with_folder(project: Project, folder: ProjectFolder) -> Self {
        let bus = CommandBus::with_folder(folder.clone());
        Self {
            project,
            bus,
            selected_items: Vec::new(),
            playhead: RationalTime::ZERO,
            folder: Some(folder),
        }
    }
}

pub type SharedEditorSession = Arc<Mutex<EditorSession>>;
