use std::sync::{Arc, Mutex};
use ve_core::{
    CommandBus, EditRecipe, ItemId, Project, ProjectFolder, RationalTime, RecipeRunState,
};

/// GUI と 内蔵 MCP サーバーが共有するプロジェクト・CommandBus・選択状態・AIディレクター状態
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
}

impl EditorSession {
    pub fn new(project: Project) -> Self {
        Self {
            project,
            bus: CommandBus::new(),
            selected_items: Vec::new(),
            playhead: RationalTime::ZERO,
            folder: None,
            active_recipe: None,
            recipe_run: None,
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
            active_recipe: None,
            recipe_run: None,
        }
    }
}

pub type SharedEditorSession = Arc<Mutex<EditorSession>>;
