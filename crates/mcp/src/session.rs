use std::sync::{Arc, Mutex};
use ve_core::{
    CommandBus, CrdtSyncLog, EditRecipe, ItemId, Project, ProjectFolder, RationalTime,
    RecipeRunState,
};

/// GUI と 内蔵 MCP サーバーが共有するプロジェクト・CommandBus・選択状態・AIディレクター・CRDT 同期状態
pub struct EditorSession {
    pub project: Project,
    pub bus: CommandBus,
    pub selected_items: Vec<ItemId>,
    pub playhead: RationalTime,
    pub folder: Option<ProjectFolder>,
    /// v2.0 #23: マルチエージェント AI 編集ディレクターのアクティブレシピ
    pub active_recipe: Option<EditRecipe>,
    /// v2.0 #23: マルチエージェント AI 編集ディレクターの実行状態（承認ゲート・視覚自己検査ログ）
    pub recipe_run: Option<RecipeRunState>,
    /// v3.0 #32: リアルタイム P2P / LAN CRDT 同時共同編集ログ（Lamport/Vector Clock・在席カーソル）
    pub crdt_log: CrdtSyncLog,
}

impl EditorSession {
    pub fn new(project: Project) -> Self {
        let peer_id = format!("peer_{}", &uuid::Uuid::new_v4().to_string()[..8]);
        Self {
            project,
            bus: CommandBus::new(),
            selected_items: Vec::new(),
            playhead: RationalTime::ZERO,
            folder: None,
            active_recipe: None,
            recipe_run: None,
            crdt_log: CrdtSyncLog::new(peer_id, "Local Editor", "#44ccff"),
        }
    }

    pub fn with_folder(project: Project, folder: ProjectFolder) -> Self {
        let bus = CommandBus::with_folder(folder.clone());
        let peer_id = format!("peer_{}", &uuid::Uuid::new_v4().to_string()[..8]);
        Self {
            project,
            bus,
            selected_items: Vec::new(),
            playhead: RationalTime::ZERO,
            folder: Some(folder),
            active_recipe: None,
            recipe_run: None,
            crdt_log: CrdtSyncLog::new(peer_id, "Local Editor", "#44ccff"),
        }
    }
}

pub type SharedEditorSession = Arc<Mutex<EditorSession>>;
