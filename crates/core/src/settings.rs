//! v1.0 #20: カスタムキーバインド・多言語 UI（日本語/English）・テーマ＆アクセシビリティ設定

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// キーマッププリセット（業界標準 NLE 切替）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeymapPreset {
    #[default]
    AviUtl2,
    Premiere,
    DaVinci,
    AfterEffects,
}

impl KeymapPreset {
    pub const ALL: &'static [Self] = &[
        Self::AviUtl2,
        Self::Premiere,
        Self::DaVinci,
        Self::AfterEffects,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::AviUtl2 => "AviUtl2 (標準)",
            Self::Premiere => "Premiere Pro 風",
            Self::DaVinci => "DaVinci Resolve 風",
            Self::AfterEffects => "After Effects 風",
        }
    }
}

/// カスタマイズ可能なエディタショートカット操作種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorAction {
    PlayPause,
    ShuttleReverse,
    ShuttlePause,
    ShuttleForward,
    SplitClip,
    Undo,
    Redo,
    Copy,
    Paste,
    Duplicate,
    SelectAll,
    Delete,
    RippleDelete,
    AddMarker,
    GroupSelection,
    UnlinkSelection,
    SlipMode,
    SlideMode,
    RollMode,
    ToggleScopes,
    /// v7.0 #45: `Ctrl+K` ファジー検索コマンドパレットを開く
    OpenCommandPalette,
}

impl EditorAction {
    pub const ALL: &'static [Self] = &[
        Self::PlayPause,
        Self::ShuttleReverse,
        Self::ShuttlePause,
        Self::ShuttleForward,
        Self::SplitClip,
        Self::Undo,
        Self::Redo,
        Self::Copy,
        Self::Paste,
        Self::Duplicate,
        Self::SelectAll,
        Self::Delete,
        Self::RippleDelete,
        Self::AddMarker,
        Self::GroupSelection,
        Self::UnlinkSelection,
        Self::SlipMode,
        Self::SlideMode,
        Self::RollMode,
        Self::ToggleScopes,
        Self::OpenCommandPalette,
    ];

    pub fn label(self, lang: UiLanguage) -> &'static str {
        match (self, lang) {
            (Self::PlayPause, UiLanguage::Japanese) => "再生 / 一時停止",
            (Self::PlayPause, UiLanguage::English) => "Play / Pause",
            (Self::ShuttleReverse, UiLanguage::Japanese) => "シャトル逆再生",
            (Self::ShuttleReverse, UiLanguage::English) => "Shuttle Reverse",
            (Self::ShuttlePause, UiLanguage::Japanese) => "シャトル停止",
            (Self::ShuttlePause, UiLanguage::English) => "Shuttle Stop",
            (Self::ShuttleForward, UiLanguage::Japanese) => "シャトル早送り",
            (Self::ShuttleForward, UiLanguage::English) => "Shuttle Forward",
            (Self::SplitClip, UiLanguage::Japanese) => "クリップ分割",
            (Self::SplitClip, UiLanguage::English) => "Split / Razor Clip",
            (Self::Undo, UiLanguage::Japanese) => "元に戻す (Undo)",
            (Self::Undo, UiLanguage::English) => "Undo",
            (Self::Redo, UiLanguage::Japanese) => "やり直し (Redo)",
            (Self::Redo, UiLanguage::English) => "Redo",
            (Self::Copy, UiLanguage::Japanese) => "コピー",
            (Self::Copy, UiLanguage::English) => "Copy",
            (Self::Paste, UiLanguage::Japanese) => "貼り付け",
            (Self::Paste, UiLanguage::English) => "Paste",
            (Self::Duplicate, UiLanguage::Japanese) => "複製",
            (Self::Duplicate, UiLanguage::English) => "Duplicate",
            (Self::SelectAll, UiLanguage::Japanese) => "全選択",
            (Self::SelectAll, UiLanguage::English) => "Select All",
            (Self::Delete, UiLanguage::Japanese) => "削除",
            (Self::Delete, UiLanguage::English) => "Delete",
            (Self::RippleDelete, UiLanguage::Japanese) => "リップル削除",
            (Self::RippleDelete, UiLanguage::English) => "Ripple Delete",
            (Self::AddMarker, UiLanguage::Japanese) => "マーカー追加",
            (Self::AddMarker, UiLanguage::English) => "Add Marker",
            (Self::GroupSelection, UiLanguage::Japanese) => "クリップをグループ化",
            (Self::GroupSelection, UiLanguage::English) => "Group Clips",
            (Self::UnlinkSelection, UiLanguage::Japanese) => "グループ／リンク解除",
            (Self::UnlinkSelection, UiLanguage::English) => "Unlink / Ungroup",
            (Self::SlipMode, UiLanguage::Japanese) => "スリップトリム",
            (Self::SlipMode, UiLanguage::English) => "Slip Trim Mode",
            (Self::SlideMode, UiLanguage::Japanese) => "スライドトリム",
            (Self::SlideMode, UiLanguage::English) => "Slide Trim Mode",
            (Self::RollMode, UiLanguage::Japanese) => "ロールトリム",
            (Self::RollMode, UiLanguage::English) => "Roll Trim Mode",
            (Self::ToggleScopes, UiLanguage::Japanese) => "ビデオスコープ切替",
            (Self::ToggleScopes, UiLanguage::English) => "Toggle Video Scopes",
            (Self::OpenCommandPalette, UiLanguage::Japanese) => "コマンドパレットを開く",
            (Self::OpenCommandPalette, UiLanguage::English) => "Open Command Palette",
        }
    }
}

/// カスタムキーマップ定義（JSON エクスポート／インポート対応）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorKeymap {
    pub preset: KeymapPreset,
    pub bindings: BTreeMap<EditorAction, String>,
}

impl Default for EditorKeymap {
    fn default() -> Self {
        Self::for_preset(KeymapPreset::AviUtl2)
    }
}

impl EditorKeymap {
    pub fn for_preset(preset: KeymapPreset) -> Self {
        let mut b = BTreeMap::new();
        b.insert(EditorAction::PlayPause, "Space".into());
        b.insert(EditorAction::ShuttleReverse, "J".into());
        b.insert(EditorAction::ShuttlePause, "K".into());
        b.insert(EditorAction::ShuttleForward, "L".into());
        b.insert(EditorAction::Undo, "Ctrl+Z".into());
        b.insert(EditorAction::Redo, "Ctrl+Y".into());
        b.insert(EditorAction::Copy, "Ctrl+C".into());
        b.insert(EditorAction::Paste, "Ctrl+V".into());
        b.insert(EditorAction::Duplicate, "Ctrl+D".into());
        b.insert(EditorAction::SelectAll, "Ctrl+A".into());
        b.insert(EditorAction::Delete, "Delete".into());
        b.insert(EditorAction::RippleDelete, "Shift+Delete".into());
        b.insert(EditorAction::AddMarker, "M".into());
        b.insert(EditorAction::GroupSelection, "Ctrl+G".into());
        b.insert(EditorAction::UnlinkSelection, "Ctrl+Shift+G".into());
        b.insert(EditorAction::SlipMode, "Y".into());
        b.insert(EditorAction::SlideMode, "U".into());
        b.insert(EditorAction::RollMode, "N".into());
        b.insert(EditorAction::ToggleScopes, "Ctrl+Shift+S".into());
        b.insert(EditorAction::OpenCommandPalette, "Ctrl+K".into());

        match preset {
            KeymapPreset::AviUtl2 => {
                b.insert(EditorAction::SplitClip, "S".into());
            }
            KeymapPreset::Premiere => {
                b.insert(EditorAction::SplitClip, "Ctrl+K".into());
                b.insert(EditorAction::OpenCommandPalette, "Ctrl+Shift+P".into());
                b.insert(EditorAction::Redo, "Ctrl+Shift+Z".into());
                b.insert(EditorAction::UnlinkSelection, "Ctrl+L".into());
            }
            KeymapPreset::DaVinci => {
                b.insert(EditorAction::SplitClip, "Ctrl+B".into());
                b.insert(EditorAction::Redo, "Ctrl+Shift+Z".into());
                b.insert(EditorAction::SlipMode, "T".into());
            }
            KeymapPreset::AfterEffects => {
                b.insert(EditorAction::SplitClip, "Ctrl+Shift+D".into());
                b.insert(EditorAction::Redo, "Ctrl+Shift+Z".into());
                b.insert(EditorAction::AddMarker, "*".into());
            }
        }
        Self {
            preset,
            bindings: b,
        }
    }

    pub fn binding_for(&self, action: EditorAction) -> &str {
        self.bindings.get(&action).map(String::as_str).unwrap_or("")
    }

    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

/// UI 表示言語（日本語 / 英語）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiLanguage {
    #[default]
    Japanese,
    English,
}

impl UiLanguage {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Japanese => "日本語 (Japanese)",
            Self::English => "English",
        }
    }
}

/// UI テーマプリセット（ダーク / OLED / ハイコントラスト / ライト）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiThemePreset {
    #[default]
    DarkStudio,
    MidnightOled,
    HighContrast,
    LightDaylight,
}

impl UiThemePreset {
    pub const ALL: &'static [Self] = &[
        Self::DarkStudio,
        Self::MidnightOled,
        Self::HighContrast,
        Self::LightDaylight,
    ];

    pub fn display_name(self, lang: UiLanguage) -> &'static str {
        match (self, lang) {
            (Self::DarkStudio, UiLanguage::Japanese) => "ダークスタジオ (標準)",
            (Self::DarkStudio, UiLanguage::English) => "Dark Studio (Default)",
            (Self::MidnightOled, UiLanguage::Japanese) => "ミッドナイト OLED",
            (Self::MidnightOled, UiLanguage::English) => "Midnight OLED",
            (Self::HighContrast, UiLanguage::Japanese) => "ハイコントラスト (アクセシビリティ)",
            (Self::HighContrast, UiLanguage::English) => "High Contrast (Accessible)",
            (Self::LightDaylight, UiLanguage::Japanese) => "ライト (デイライト)",
            (Self::LightDaylight, UiLanguage::English) => "Light Daylight",
        }
    }
}

/// アプリ全体のユーザー設定（言語・テーマ・フォント倍率・キーマップ・HWデコード＆キャッシュ設定）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorUserSettings {
    pub language: UiLanguage,
    pub theme: UiThemePreset,
    /// フォントサイズ倍率（0.85..=1.50、アクセシビリティ対応）
    pub font_scale: f32,
    pub keymap: EditorKeymap,
    /// SSD 永続フレームキャッシュ上限（MiB、既定 1024 MiB）
    pub disk_frame_cache_mib: u64,
    /// 並列書き出しワーカー数（0 = 自動）
    pub export_worker_threads: usize,
}

impl Default for EditorUserSettings {
    fn default() -> Self {
        Self {
            language: UiLanguage::Japanese,
            theme: UiThemePreset::DarkStudio,
            font_scale: 1.0,
            keymap: EditorKeymap::default(),
            disk_frame_cache_mib: 1024,
            export_worker_threads: 0,
        }
    }
}

/// 簡易多言語辞書ルックアップ（日本語 / 英語）
pub fn tr(lang: UiLanguage, key: &str) -> &'static str {
    match (lang, key) {
        (UiLanguage::Japanese, "workspace.main") => "メイン編集空間 (AviUtl2)",
        (UiLanguage::English, "workspace.main") => "Main Edit Space (NLE)",
        (UiLanguage::Japanese, "workspace.motion") => "モーション合成空間 (AE)",
        (UiLanguage::English, "workspace.motion") => "Motion Composite (AE)",
        (UiLanguage::Japanese, "pane.preview") => "プレビュー",
        (UiLanguage::English, "pane.preview") => "Preview",
        (UiLanguage::Japanese, "pane.timeline") => "タイムライン",
        (UiLanguage::English, "pane.timeline") => "Timeline",
        (UiLanguage::Japanese, "pane.inspector") => "インスペクタ",
        (UiLanguage::English, "pane.inspector") => "Inspector",
        (UiLanguage::Japanese, "pane.graph") => "グラフエディタ",
        (UiLanguage::English, "pane.graph") => "Graph Editor",
        (UiLanguage::Japanese, "pane.assets") => "アセット",
        (UiLanguage::English, "pane.assets") => "Assets",
        (UiLanguage::Japanese, "pane.audio_ai") => "音声・AI・3D・マルチカム",
        (UiLanguage::English, "pane.audio_ai") => "Audio / AI / 3D / Multicam",
        (UiLanguage::Japanese, "btn.scopes") => "📊 スコープ",
        (UiLanguage::English, "btn.scopes") => "📊 Scopes",
        (UiLanguage::Japanese, "btn.multicam") | (UiLanguage::Japanese, "multicam_title") => {
            "🎥 マルチカム"
        }
        (UiLanguage::English, "btn.multicam") | (UiLanguage::English, "multicam_title") => {
            "🎥 Multicam"
        }
        (UiLanguage::Japanese, "btn.settings") | (UiLanguage::Japanese, "settings_title") => {
            "⚙ 設定"
        }
        (UiLanguage::English, "btn.settings") | (UiLanguage::English, "settings_title") => {
            "⚙ Settings"
        }
        (UiLanguage::Japanese, "btn.palette") => "🔍 Ctrl+K",
        (UiLanguage::English, "btn.palette") => "🔍 Ctrl+K",
        (UiLanguage::Japanese, "btn.save") => "💾 保存",
        (UiLanguage::English, "btn.save") => "💾 Save",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keymap_presets_and_json_roundtrip_work() {
        let prem = EditorKeymap::for_preset(KeymapPreset::Premiere);
        assert_eq!(prem.binding_for(EditorAction::SplitClip), "Ctrl+K");

        let dv = EditorKeymap::for_preset(KeymapPreset::DaVinci);
        assert_eq!(dv.binding_for(EditorAction::SplitClip), "Ctrl+B");

        let json = prem.to_json_pretty().unwrap();
        let parsed = EditorKeymap::from_json(&json).unwrap();
        assert_eq!(parsed, prem);

        assert_eq!(tr(UiLanguage::English, "pane.timeline"), "Timeline");
        assert_eq!(tr(UiLanguage::Japanese, "pane.timeline"), "タイムライン");
    }
}
