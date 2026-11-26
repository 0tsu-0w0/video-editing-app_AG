//! v7.0 #45: 役割別 6 スタジオ Hub UX カタログ ＆ `Ctrl+K` ファジー検索コマンドパレット
//! ＋ v7.0 #48: A/B スプリット比較ビュー設定モデル (`AbCompareConfig`)

use crate::project::SceneId;
use crate::settings::UiLanguage;
use serde::{Deserialize, Serialize};

/// v7.0 #45: トップバー統合用の役割別 6 スタジオ Hub 識別子
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StudioHubId {
    Edit,
    VfxMotion,
    Color,
    Audio,
    AiCollab,
    Deliver,
}

impl StudioHubId {
    pub const ALL: &'static [Self] = &[
        Self::Edit,
        Self::VfxMotion,
        Self::Color,
        Self::Audio,
        Self::AiCollab,
        Self::Deliver,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::VfxMotion => "vfx_motion",
            Self::Color => "color",
            Self::Audio => "audio",
            Self::AiCollab => "ai_collab",
            Self::Deliver => "deliver",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug.trim().to_ascii_lowercase().as_str() {
            "edit" | "nle" => Some(Self::Edit),
            "vfx_motion" | "vfx" | "motion" => Some(Self::VfxMotion),
            "color" | "grading" => Some(Self::Color),
            "audio" | "sound" => Some(Self::Audio),
            "ai_collab" | "ai" | "collab" => Some(Self::AiCollab),
            "deliver" | "export" => Some(Self::Deliver),
            _ => None,
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Edit => "✂",
            Self::VfxMotion => "✨",
            Self::Color => "🎨",
            Self::Audio => "🎧",
            Self::AiCollab => "🤖",
            Self::Deliver => "📦",
        }
    }

    pub fn button_label(self, lang: UiLanguage) -> &'static str {
        match (self, lang) {
            (Self::Edit, UiLanguage::Japanese) => "✂ Edit",
            (Self::Edit, UiLanguage::English) => "✂ Edit",
            (Self::VfxMotion, UiLanguage::Japanese) => "✨ VFX & Motion",
            (Self::VfxMotion, UiLanguage::English) => "✨ VFX & Motion",
            (Self::Color, UiLanguage::Japanese) => "🎨 Color",
            (Self::Color, UiLanguage::English) => "🎨 Color",
            (Self::Audio, UiLanguage::Japanese) => "🎧 Audio",
            (Self::Audio, UiLanguage::English) => "🎧 Audio",
            (Self::AiCollab, UiLanguage::Japanese) => "🤖 AI & Collab",
            (Self::AiCollab, UiLanguage::English) => "🤖 AI & Collab",
            (Self::Deliver, UiLanguage::Japanese) => "📦 Deliver",
            (Self::Deliver, UiLanguage::English) => "📦 Deliver",
        }
    }

    pub fn summary(self, lang: UiLanguage) -> &'static str {
        match (self, lang) {
            (Self::Edit, UiLanguage::Japanese) => {
                "マルチカム・速度曲線/自動カット・キネティック字幕・.mogrtテンプレート・10k区間木＆プロキシ⇔原本TCコンフォーム"
            }
            (Self::Edit, UiLanguage::English) => {
                "Multicam, Speed Ramp & Cut Detect, Kinetic Subtitles, .mogrt Templates, 10k Interval Index & Proxy TC Conform"
            }
            (Self::VfxMotion, UiLanguage::Japanese) => {
                "手ぶれ補正/4点ピン・VFXペイント/ワイヤー消し・glTF 3D・2D IKリグ・2D物理・チャート・生成背景・3D照明・GPU 4K/8Kプール"
            }
            (Self::VfxMotion, UiLanguage::English) => {
                "Stabilizer/Corner Pin, VFX Paint/Inpaint, glTF 3D, 2D IK Rig, Physics, Data Charts, Generative BG & GPU 4K/8K Pool"
            }
            (Self::Color, UiLanguage::Japanese) => {
                "3-Wayホイール＆.cube 3D LUT・RGBカーブ/HSL/自動カラーマッチ・ACEScg/OCIO・35mmハレーション/フレア・A/B比較ビュー"
            }
            (Self::Color, UiLanguage::English) => {
                "3-Way Wheels & .cube LUT, RGB Curves/HSL/Color Match, ACEScg/OCIO, 35mm Cinema Finishing & A/B Split Compare"
            }
            (Self::Audio, UiLanguage::Japanese) => {
                "トラックミキサー/NR/ボーカル分離・3Dバイノーラル/True-Peak・自動BGM作曲・オフラインAI吹替・ポッドキャスト自動編集"
            }
            (Self::Audio, UiLanguage::English) => {
                "Track Mixer/NR/Stems, 3D Binaural/True-Peak, BGM Composer, Offline AI Dubbing & Podcast Auto-Director"
            }
            (Self::AiCollab, UiLanguage::Japanese) => {
                "ローカルAIビジョン・AI編集ディレクター＆レシピ・タイムラインDiff/3-Wayマージ＆レビュー・P2P CRDT・MCPカタログ"
            }
            (Self::AiCollab, UiLanguage::English) => {
                "Local AI Vision, Multi-Agent Director Recipes, Git Diff/3-Way Merge/Review, P2P CRDT & MCP Catalog"
            }
            (Self::Deliver, UiLanguage::Japanese) => {
                "マルチプラットフォーム1クリック納品・放送QC監査＆HTML5レビュー・CMX3600 EDL/FCPXML・納品検証＆ZIP・レンダーキュー"
            }
            (Self::Deliver, UiLanguage::English) => {
                "Multi-Platform Packager, Broadcast QC & HTML5 Review, CMX3600 EDL/FCPXML, Delivery ZIP & Render Queue"
            }
        }
    }
}

/// `Ctrl+K` コマンドパレットに登録される単一操作・パネル定義
#[derive(Debug, Clone, Copy)]
pub struct PaletteCommandEntry {
    pub id: &'static str,
    pub hub: Option<StudioHubId>,
    pub title_ja: &'static str,
    pub title_en: &'static str,
    pub roadmap_tag: &'static str,
    pub shortcut_hint: Option<&'static str>,
    pub keywords: &'static [&'static str],
}

impl PaletteCommandEntry {
    pub fn title(&self, lang: UiLanguage) -> &'static str {
        match lang {
            UiLanguage::Japanese => self.title_ja,
            UiLanguage::English => self.title_en,
        }
    }
}

/// v0.1〜v7.0 の全ワークスペース・全 6 スタジオ Hub パネル・主要クイックアクションを網羅する統合カタログ（機能ゼロ欠落）
pub const STUDIO_PALETTE_CATALOG: &[PaletteCommandEntry] = &[
    // --- 3 Workspaces (Tri-Workspace UX preserved) ---
    PaletteCommandEntry {
        id: "workspace.main_edit",
        hub: None,
        title_ja: "🎬 空間切替: メイン編集空間 (AviUtl2)",
        title_en: "🎬 Switch Workspace: Main Edit (AviUtl2 NLE)",
        roadmap_tag: "Core",
        shortcut_hint: Some("F1"),
        keywords: &["workspace", "main", "aviutl", "nle", "メイン編集", "空間"],
    },
    PaletteCommandEntry {
        id: "workspace.motion_composite",
        hub: None,
        title_ja: "✨ 空間切替: モーション合成空間 (After Effects)",
        title_en: "✨ Switch Workspace: Motion Composite (AE)",
        roadmap_tag: "Core",
        shortcut_hint: Some("F2"),
        keywords: &[
            "workspace",
            "motion",
            "ae",
            "graph",
            "keyframe",
            "モーション合成",
            "グラフエディタ",
        ],
    },
    PaletteCommandEntry {
        id: "workspace.node_composite",
        hub: None,
        title_ja: "🔀 空間切替: ノード合成空間 (Fusion / Nuke)",
        title_en: "🔀 Switch Workspace: Node Composite (Fusion / Nuke)",
        roadmap_tag: "#21",
        shortcut_hint: Some("F3"),
        keywords: &[
            "workspace",
            "node",
            "fusion",
            "nuke",
            "depth",
            "ノード合成",
            "#21",
        ],
    },
    // --- Hub 1: Edit ---
    PaletteCommandEntry {
        id: "hub.edit.multicam",
        hub: Some(StudioHubId::Edit),
        title_ja: "🎥 マルチカメラ編集 ＆ 音声相互相関自動同期",
        title_en: "🎥 Multicam Studio & Audio XCorr Auto-Sync",
        roadmap_tag: "#17",
        shortcut_hint: Some("1-9"),
        keywords: &[
            "multicam",
            "sync",
            "camera",
            "switch",
            "マルチカム",
            "同期",
            "#17",
        ],
    },
    PaletteCommandEntry {
        id: "hub.edit.speed_ramp",
        hub: Some(StudioHubId::Edit),
        title_ja: "⚡ スピードランプ曲線 ＆ 自動シーンカット検出",
        title_en: "⚡ Speed Ramp Time-Remap & Scene Edit Detection",
        roadmap_tag: "#39",
        shortcut_hint: None,
        keywords: &[
            "speed",
            "ramp",
            "bullet",
            "slow",
            "scene",
            "cut",
            "detect",
            "スピードランプ",
            "速度曲線",
            "カット検出",
            "#39",
        ],
    },
    PaletteCommandEntry {
        id: "hub.edit.kinetic_sub",
        hub: Some(StudioHubId::Edit),
        title_ja: "💬 ダイナミック・キネティック字幕 ＆ 多言語 CSV ローカライズ",
        title_en: "💬 Kinetic Subtitles & Multi-Locale CSV Switcher",
        roadmap_tag: "#28",
        shortcut_hint: None,
        keywords: &[
            "kinetic",
            "subtitle",
            "karaoke",
            "localize",
            "csv",
            "translate",
            "キネティック字幕",
            "多言語",
            "翻訳",
            "#28",
        ],
    },
    PaletteCommandEntry {
        id: "hub.edit.templates",
        hub: Some(StudioHubId::Edit),
        title_ja: "📐 モーショングラフィックス (.mogrt) ＆ プロジェクトテンプレート",
        title_en: "📐 Motion Graphics (.mogrt) & Project Templates",
        roadmap_tag: "#15",
        shortcut_hint: None,
        keywords: &[
            "template",
            "mogrt",
            "title",
            "lower third",
            "テンプレート",
            "雛形",
            "#15",
        ],
    },
    PaletteCommandEntry {
        id: "hub.edit.conform_index",
        hub: Some(StudioHubId::Edit),
        title_ja: "🌲 10,000+ 区間木インデックス ＆ プロキシ⇔原本タイムコード自動コンフォーム",
        title_en: "🌲 10k+ Interval Index & Proxy/Original Timecode Auto-Conform",
        roadmap_tag: "#48",
        shortcut_hint: None,
        keywords: &[
            "interval",
            "tree",
            "10000",
            "index",
            "proxy",
            "original",
            "timecode",
            "conform",
            "smpte",
            "区間木",
            "プロキシ",
            "コンフォーム",
            "タイムコード",
            "#48",
        ],
    },
    // --- Hub 2: VFX & Motion ---
    PaletteCommandEntry {
        id: "hub.vfx.stabilize_pin",
        hub: Some(StudioHubId::VfxMotion),
        title_ja: "📹 自動 2D 手ぶれ補正 ＆ 平面 4 点コーナーピン合成",
        title_en: "📹 2D Video Stabilizer & Planar 4-Point Corner Pin",
        roadmap_tag: "#26",
        shortcut_hint: None,
        keywords: &[
            "stabilize",
            "shake",
            "corner",
            "pin",
            "homography",
            "手ぶれ補正",
            "コーナーピン",
            "#26",
        ],
    },
    PaletteCommandEntry {
        id: "hub.vfx.paint_inpaint",
        hub: Some(StudioHubId::VfxMotion),
        title_ja: "🖌 非破壊 VFX ペイント / クローンスタンプ / ワイヤー消し / 秘匿ぼかし",
        title_en: "🖌 Non-Destructive VFX Paint, Clone Stamp, Wire Erase & Redact",
        roadmap_tag: "#42",
        shortcut_hint: None,
        keywords: &[
            "vfx",
            "paint",
            "clone",
            "wire",
            "erase",
            "inpaint",
            "blur",
            "redact",
            "mosaic",
            "ペイント",
            "ワイヤー消し",
            "クローン",
            "モザイク",
            "#42",
        ],
    },
    PaletteCommandEntry {
        id: "hub.vfx.scene3d",
        hub: Some(StudioHubId::VfxMotion),
        title_ja: "🧊 3D メッシュ / glTF 2.0 PBR シーン ＆ 3D カメラトラッカー",
        title_en: "🧊 3D Mesh / glTF 2.0 PBR & 3D Camera Tracker",
        roadmap_tag: "#30",
        shortcut_hint: None,
        keywords: &[
            "3d",
            "mesh",
            "gltf",
            "glb",
            "pbr",
            "extrude",
            "camera",
            "tracker",
            "立体テキスト",
            "#30",
        ],
    },
    PaletteCommandEntry {
        id: "hub.vfx.rig2d_ik",
        hub: Some(StudioHubId::VfxMotion),
        title_ja: "🦴 2D スケルトンボーン ＆ 2-Bone IK パペットリギング",
        title_en: "🦴 2D Skeleton Bones & 2-Bone IK Puppet Rigging",
        roadmap_tag: "#37",
        shortcut_hint: None,
        keywords: &[
            "rig",
            "bone",
            "ik",
            "fk",
            "puppet",
            "character",
            "ボーン",
            "パペット",
            "立ち絵",
            "#37",
        ],
    },
    PaletteCommandEntry {
        id: "hub.vfx.physics2d",
        hub: Some(StudioHubId::VfxMotion),
        title_ja: "🏀 決定論的 2D リジッドボディ物理演算 ＆ キーフレームベイク",
        title_en: "🏀 Deterministic 2D Rigid-Body Physics & Keyframe Bake",
        roadmap_tag: "#34",
        shortcut_hint: None,
        keywords: &[
            "physics",
            "gravity",
            "bounce",
            "domino",
            "vortex",
            "bake",
            "物理演算",
            "重力",
            "#34",
        ],
    },
    PaletteCommandEntry {
        id: "hub.vfx.data_chart",
        hub: Some(StudioHubId::VfxMotion),
        title_ja: "📊 データ駆動モーショングラフィックス ＆ インフォグラフィックチャート",
        title_en: "📊 Data-Driven Motion Graphics & Infographic Charts",
        roadmap_tag: "#41",
        shortcut_hint: None,
        keywords: &[
            "data",
            "chart",
            "bar",
            "line",
            "donut",
            "kpi",
            "csv",
            "infographic",
            "チャート",
            "グラフ",
            "#41",
        ],
    },
    PaletteCommandEntry {
        id: "hub.vfx.generative_bg_foley",
        hub: Some(StudioHubId::VfxMotion),
        title_ja: "🌌 プロシージャル動画背景ジェネレーター ＆ Foley/SFX 効果音シンセ",
        title_en: "🌌 Procedural Video Backgrounds & 48kHz Foley/SFX Synth",
        roadmap_tag: "#35",
        shortcut_hint: None,
        keywords: &[
            "generative",
            "background",
            "aurora",
            "bokeh",
            "foley",
            "sfx",
            "whoosh",
            "背景生成",
            "効果音",
            "#35",
        ],
    },
    PaletteCommandEntry {
        id: "hub.vfx.lighting_mblur",
        hub: Some(StudioHubId::VfxMotion),
        title_ja: "💡 3D ライティング・シャドウ ＆ シャッター角モーションブラー",
        title_en: "💡 3D Scene Lighting & Shutter-Angle Motion Blur",
        roadmap_tag: "#16",
        shortcut_hint: None,
        keywords: &[
            "light",
            "shadow",
            "motion blur",
            "shutter",
            "particle",
            "vector",
            "ライト",
            "モーションブラー",
            "#16",
        ],
    },
    PaletteCommandEntry {
        id: "hub.vfx.gpu_pool",
        hub: Some(StudioHubId::VfxMotion),
        title_ja: "⚡ GPU-Resident 4K/8K フレームプール ＆ WGSL 高速コンピュートパス",
        title_en: "⚡ GPU-Resident 4K/8K Frame Pool & WGSL Fast-Path Compute",
        roadmap_tag: "#29/#47",
        shortcut_hint: None,
        keywords: &[
            "gpu",
            "wgpu",
            "wgsl",
            "compute",
            "4k",
            "8k",
            "pool",
            "vram",
            "benchmark",
            "フレームプール",
            "#29",
            "#47",
        ],
    },
    // --- Hub 3: Color ---
    PaletteCommandEntry {
        id: "hub.color.primary_lut",
        hub: Some(StudioHubId::Color),
        title_ja: "🎡 3-Way カラーホイール ＆ .cube 3D LUT ＆ ビデオスコープ",
        title_en: "🎡 3-Way Color Wheels, .cube 3D LUT & Video Scopes",
        roadmap_tag: "#13",
        shortcut_hint: Some("Ctrl+Shift+S"),
        keywords: &[
            "color",
            "lut",
            "cube",
            "lift",
            "gamma",
            "gain",
            "waveform",
            "vectorscope",
            "parade",
            "カラーホイール",
            "スコープ",
            "#13",
        ],
    },
    PaletteCommandEntry {
        id: "hub.color.curves_hsl_match",
        hub: Some(StudioHubId::Color),
        title_ja: "📈 RGB トーンカーブ / HSL 二次色補正 / 自動ショットカラーマッチ",
        title_en: "📈 RGB Tone Curves, HSL Qualifier & Auto Shot Color Match",
        roadmap_tag: "#25",
        shortcut_hint: None,
        keywords: &[
            "curve",
            "hsl",
            "qualifier",
            "skin",
            "color match",
            "reinhard",
            "トーンカーブ",
            "二次色補正",
            "カラーマッチ",
            "#25",
        ],
    },
    PaletteCommandEntry {
        id: "hub.color.aces_ocio",
        hub: Some(StudioHubId::Color),
        title_ja: "🎬 ACEScg / OpenColorIO v2 プロカラー管理 ＆ カメラ Log IDT",
        title_en: "🎬 ACEScg / OpenColorIO v2 Pipeline & Camera Log IDT",
        roadmap_tag: "#33",
        shortcut_hint: None,
        keywords: &[
            "aces",
            "acescg",
            "ocio",
            "slog3",
            "vlog",
            "logc3",
            "agx",
            "p3",
            "rec2020",
            "カラー管理",
            "#33",
        ],
    },
    PaletteCommandEntry {
        id: "hub.color.cinema_finishing",
        hub: Some(StudioHubId::Color),
        title_ja: "🎞 35mm ハレーション・アナモルフィックフレア・グレイン・SMPTE バーンイン",
        title_en: "🎞 35mm Halation, Anamorphic Flare, Film Grain & Burn-In",
        roadmap_tag: "#40",
        shortcut_hint: None,
        keywords: &[
            "halation",
            "anamorphic",
            "flare",
            "grain",
            "letterbox",
            "burnin",
            "ハレーション",
            "フレア",
            "グレイン",
            "#40",
        ],
    },
    PaletteCommandEntry {
        id: "hub.color.ab_compare",
        hub: Some(StudioHubId::Color),
        title_ja: "🌓 A/B スプリット比較ビュー（垂直/水平/斜めワイプ・差分・静止画A比較）",
        title_en: "🌓 A/B Split Wipe Comparison View (Vertical / Horizontal / Diagonal / Diff)",
        roadmap_tag: "#48",
        shortcut_hint: Some("Alt+W"),
        keywords: &[
            "ab",
            "split",
            "wipe",
            "compare",
            "before",
            "after",
            "still",
            "diff",
            "スプリット",
            "比較",
            "ワイプ",
            "#48",
        ],
    },
    // --- Hub 4: Audio ---
    PaletteCommandEntry {
        id: "hub.audio.mixer_nr_stems",
        hub: Some(StudioHubId::Audio),
        title_ja: "🎚 トラックミキサー・マイク録音・STFT ノイズ除去・ディエッサー・ボーカル分離",
        title_en: "🎚 Track Mixer, Voiceover Mic, STFT NR, De-Esser & Stem Separation",
        roadmap_tag: "#19",
        shortcut_hint: None,
        keywords: &[
            "mixer",
            "mic",
            "record",
            "voiceover",
            "noise",
            "deesser",
            "vocal",
            "stem",
            "ミキサー",
            "録音",
            "ノイズ除去",
            "ボーカル分離",
            "#19",
        ],
    },
    PaletteCommandEntry {
        id: "hub.audio.spatial_master",
        hub: Some(StudioHubId::Audio),
        title_ja: "🎧 3D 空間オーディオ（バイノーラル）＆ マルチバンドコンプ ＆ True-Peak リミッター",
        title_en: "🎧 3D Binaural Spatial Audio, Multiband Comp & True-Peak Limiter",
        roadmap_tag: "#27",
        shortcut_hint: None,
        keywords: &[
            "spatial",
            "binaural",
            "3d audio",
            "multiband",
            "true peak",
            "lufs",
            "limiter",
            "空間オーディオ",
            "バイノーラル",
            "マスタリング",
            "#27",
        ],
    },
    PaletteCommandEntry {
        id: "hub.audio.bgm_composer",
        hub: Some(StudioHubId::Audio),
        title_ja: "🎼 オフライン自動プロシージャル BGM コンポーザー（6 ジャンル ＆ 小節マーカー）",
        title_en: "🎼 Offline Procedural BGM Composer (6 Genres & Bar Markers)",
        roadmap_tag: "#38",
        shortcut_hint: None,
        keywords: &[
            "bgm",
            "compose",
            "music",
            "lofi",
            "synthwave",
            "cinematic",
            "自動作曲",
            "音楽",
            "#38",
        ],
    },
    PaletteCommandEntry {
        id: "hub.audio.offline_dubbing",
        hub: Some(StudioHubId::Audio),
        title_ja: "🗣 オフライン多言語 AI 吹替 WAV 合成 ＆ 立ち絵リップシンク自動生成",
        title_en: "🗣 Offline Multi-Language AI Dubbing & Phoneme Lip-Sync",
        roadmap_tag: "#31",
        shortcut_hint: None,
        keywords: &[
            "dubbing",
            "tts",
            "voice",
            "lipsync",
            "phoneme",
            "吹替",
            "リップシンク",
            "口パク",
            "#31",
        ],
    },
    PaletteCommandEntry {
        id: "hub.audio.podcast_director",
        hub: Some(StudioHubId::Audio),
        title_ja: "🎙 マルチトラック・ポッドキャスト話者自動スイッチング ＆ -16 LUFS レベラー",
        title_en: "🎙 Multi-Track Podcast Speaker Diarization & -16 LUFS Auto-Leveler",
        roadmap_tag: "#43",
        shortcut_hint: None,
        keywords: &[
            "podcast",
            "diarization",
            "speaker",
            "leveler",
            "lufs",
            "ポッドキャスト",
            "対談",
            "話者",
            "#43",
        ],
    },
    // --- Hub 5: AI & Collab ---
    PaletteCommandEntry {
        id: "hub.ai.local_vision",
        hub: Some(StudioHubId::AiCollab),
        title_ja: "🧠 ローカル AI ビジョン（人物切り抜き・9:16 自動リフレーム・超解像・光学フロー）",
        title_en: "🧠 Local AI Vision (Segmentation, 9:16 Auto-Reframe, Super-Res & Optical Flow)",
        roadmap_tag: "#22",
        shortcut_hint: None,
        keywords: &[
            "ai",
            "vision",
            "segment",
            "rotoscope",
            "reframe",
            "super resolution",
            "optical flow",
            "切り抜き",
            "自動リフレーム",
            "超解像",
            "#22",
        ],
    },
    PaletteCommandEntry {
        id: "hub.ai.director_recipe",
        hub: Some(StudioHubId::AiCollab),
        title_ja: "🤖 マルチエージェント AI 編集ディレクター ＆ YAML/JSON 編集レシピ",
        title_en: "🤖 Multi-Agent AI Editing Director & YAML/JSON Recipes",
        roadmap_tag: "#23",
        shortcut_hint: None,
        keywords: &[
            "director",
            "agent",
            "recipe",
            "yaml",
            "approval",
            "macro",
            "ディレクター",
            "レシピ",
            "#23",
        ],
    },
    PaletteCommandEntry {
        id: "hub.ai.collab_diff_review",
        hub: Some(StudioHubId::AiCollab),
        title_ja: "🤝 Git-Friendly タイムライン視覚 Diff・3-Way マージ ＆ Frame.io/YouTube レビュー",
        title_en: "🤝 Visual Timeline Diff, 3-Way Merge & Timecoded Review Comments",
        roadmap_tag: "#24",
        shortcut_hint: None,
        keywords: &[
            "diff",
            "merge",
            "git",
            "branch",
            "review",
            "frameio",
            "youtube",
            "comment",
            "差分",
            "マージ",
            "レビュー",
            "#24",
        ],
    },
    PaletteCommandEntry {
        id: "hub.ai.crdt_lan",
        hub: Some(StudioHubId::AiCollab),
        title_ja: "🌐 リアルタイム P2P / LAN TCP ＆ Lamport CRDT 同時共同編集",
        title_en: "🌐 Realtime P2P / LAN TCP & Op-Based CRDT Co-Editing",
        roadmap_tag: "#32",
        shortcut_hint: None,
        keywords: &[
            "crdt",
            "p2p",
            "lan",
            "collab",
            "peer",
            "cursor",
            "共同編集",
            "同時編集",
            "#32",
        ],
    },
    PaletteCommandEntry {
        id: "hub.ai.mcp_inspector",
        hub: Some(StudioHubId::AiCollab),
        title_ja: "🔌 内蔵 MCP ドメイン別ツールカタログ ＆ クライアント接続インスペクタ",
        title_en: "🔌 Embedded MCP Modular Domain Catalog & Connection Inspector",
        roadmap_tag: "#46",
        shortcut_hint: None,
        keywords: &[
            "mcp",
            "claude",
            "cursor",
            "codex",
            "tools",
            "domain",
            "catalog",
            "カタログ",
            "#46",
        ],
    },
    // --- Hub 6: Deliver ---
    PaletteCommandEntry {
        id: "hub.deliver.multi_packager",
        hub: Some(StudioHubId::Deliver),
        title_ja: "📦 マルチプラットフォーム自動リフォーマット ＆ 1 クリック納品バンドルパッケージャー",
        title_en: "📦 Multi-Platform Auto-Reformat & 1-Click Delivery Bundle Packager",
        roadmap_tag: "#44",
        shortcut_hint: None,
        keywords: &[
            "deliver",
            "bundle",
            "package",
            "shorts",
            "9:16",
            "1:1",
            "manifest",
            "マルチ納品",
            "パッケージ",
            "#44",
        ],
    },
    PaletteCommandEntry {
        id: "hub.deliver.broadcast_qc_html",
        hub: Some(StudioHubId::Deliver),
        title_ja: "📡 放送コンプライアンス QC 監査・高 CTR サムネイル抽出・HTML5 レビュー書き出し",
        title_en: "📡 Broadcast QC Audit (BS.1770/BT.1702), Hero Thumbnail & HTML5 Review",
        roadmap_tag: "#36",
        shortcut_hint: None,
        keywords: &[
            "qc",
            "broadcast",
            "ebu",
            "flash",
            "thumbnail",
            "ctr",
            "html5",
            "放送qc",
            "サムネイル",
            "#36",
        ],
    },
    PaletteCommandEntry {
        id: "hub.deliver.edl_fcpxml",
        hub: Some(StudioHubId::Deliver),
        title_ja: "📄 CMX 3600 EDL / Final Cut Pro XML v1.10 (FCPXML) タイムライン交換書き出し",
        title_en: "📄 CMX 3600 EDL & Final Cut Pro XML v1.10 (FCPXML) Interchange Export",
        roadmap_tag: "#40",
        shortcut_hint: None,
        keywords: &[
            "edl", "cmx3600", "fcpxml", "xml", "davinci", "premiere", "fcp", "互換", "#40",
        ],
    },
    PaletteCommandEntry {
        id: "hub.deliver.validate_zip",
        hub: Some(StudioHubId::Deliver),
        title_ja: "✅ 納品前検証チェックリスト ＆ Pure-Rust ZIP アーカイブパッケージ化",
        title_en: "✅ Pre-Delivery Validation Checklist & Pure-Rust ZIP Archive",
        roadmap_tag: "#19",
        shortcut_hint: None,
        keywords: &[
            "validate",
            "check",
            "zip",
            "archive",
            "collect",
            "検証",
            "アーカイブ",
            "#19",
        ],
    },
    PaletteCommandEntry {
        id: "hub.deliver.render_queue",
        hub: Some(StudioHubId::Deliver),
        title_ja: "📋 バッチ連続書き出しレンダーキューを開く",
        title_en: "📋 Open Batch Render Queue",
        roadmap_tag: "#7",
        shortcut_hint: None,
        keywords: &[
            "render",
            "queue",
            "batch",
            "export",
            "mp4",
            "hevc",
            "hdr10",
            "レンダーキュー",
            "バッチ書き出し",
            "#7",
        ],
    },
    // --- Quick Timeline / Preview / System Actions ---
    PaletteCommandEntry {
        id: "action.jump_cut_bar",
        hub: None,
        title_ja: "✂ タイムライン: 無音区間自動カット（ジェットカット）バー切替",
        title_en: "✂ Timeline: Toggle Auto Silence Jump-Cut Bar",
        roadmap_tag: "v0.4",
        shortcut_hint: None,
        keywords: &["silence", "jump", "jet", "cut", "無音", "ジェットカット"],
    },
    PaletteCommandEntry {
        id: "action.beat_detect_bar",
        hub: None,
        title_ja: "🥁 タイムライン: BGM ビート自動検出 ＆ ビートスナップバー切替",
        title_en: "🥁 Timeline: Toggle BGM Beat Detection & Beat-Snap Bar",
        roadmap_tag: "#12",
        shortcut_hint: None,
        keywords: &["beat", "bpm", "snap", "music", "ビート", "音ハメ", "#12"],
    },
    PaletteCommandEntry {
        id: "action.search_replace_bar",
        hub: None,
        title_ja: "🔍 タイムライン: 字幕・テキスト検索＆一括置換バー切替",
        title_en: "🔍 Timeline: Toggle Subtitle Search & Replace Bar",
        roadmap_tag: "v0.3",
        shortcut_hint: Some("Ctrl+F"),
        keywords: &["search", "replace", "find", "text", "検索", "置換"],
    },
    PaletteCommandEntry {
        id: "action.toggle_scopes",
        hub: None,
        title_ja: "📊 プレビュー: リアルタイムビデオスコープ表示切替",
        title_en: "📊 Preview: Toggle Realtime Video Scopes",
        roadmap_tag: "#13",
        shortcut_hint: Some("Ctrl+Shift+S"),
        keywords: &["scope", "waveform", "parade", "vectorscope", "スコープ"],
    },
    PaletteCommandEntry {
        id: "action.toggle_ab_split",
        hub: None,
        title_ja: "🌓 プレビュー: A/B スプリット比較ワイプ ON/OFF トグル",
        title_en: "🌓 Preview: Toggle A/B Split Wipe Comparison",
        roadmap_tag: "#48",
        shortcut_hint: Some("Alt+W"),
        keywords: &["ab", "split", "wipe", "compare", "比較", "ワイプ", "#48"],
    },
    PaletteCommandEntry {
        id: "action.toggle_proxies",
        hub: None,
        title_ja: "⚡ プレビュー: 軽量プロキシ再生 ON/OFF 切替",
        title_en: "⚡ Preview: Toggle Lightweight Proxy Playback",
        roadmap_tag: "#18",
        shortcut_hint: None,
        keywords: &["proxy", "lightweight", "プロキシ", "軽量"],
    },
    PaletteCommandEntry {
        id: "action.clear_ram_cache",
        hub: None,
        title_ja: "🧹 キャッシュ: RAM 先読みプレビュー＆メディアキャッシュをクリア",
        title_en: "🧹 Cache: Clear RAM Preview & Media Frame Caches",
        roadmap_tag: "#2",
        shortcut_hint: None,
        keywords: &["ram", "cache", "clear", "purge", "キャッシュ", "クリア"],
    },
    PaletteCommandEntry {
        id: "action.scene_settings",
        hub: None,
        title_ja: "⚙ シーン設定: 解像度・FPS・縦動画 (9:16) プリセットを開く",
        title_en: "⚙ Scene Settings: Resolution, FPS & 9:16 Vertical Presets",
        roadmap_tag: "#3",
        shortcut_hint: None,
        keywords: &[
            "scene",
            "settings",
            "resolution",
            "fps",
            "1080p",
            "4k",
            "9:16",
            "シーン設定",
            "解像度",
        ],
    },
    PaletteCommandEntry {
        id: "action.user_settings",
        hub: None,
        title_ja: "🛠 環境設定: キーバインド・テーマ・多言語・HWデコード・公式拡張レジストリ",
        title_en: "🛠 Preferences: Keymap, Theme, Language, HW Decode & Extension Registry",
        roadmap_tag: "#20",
        shortcut_hint: Some("Ctrl+,"),
        keywords: &[
            "settings",
            "preferences",
            "keymap",
            "shortcut",
            "theme",
            "language",
            "registry",
            "wasm",
            "設定",
            "キーバインド",
            "テーマ",
            "#20",
        ],
    },
    PaletteCommandEntry {
        id: "action.undo",
        hub: None,
        title_ja: "↩ 元に戻す (Undo)",
        title_en: "↩ Undo Last Edit",
        roadmap_tag: "Core",
        shortcut_hint: Some("Ctrl+Z"),
        keywords: &["undo", "revert", "元に戻す"],
    },
    PaletteCommandEntry {
        id: "action.redo",
        hub: None,
        title_ja: "↪ やり直し (Redo)",
        title_en: "↪ Redo Edit",
        roadmap_tag: "Core",
        shortcut_hint: Some("Ctrl+Y"),
        keywords: &["redo", "やり直し"],
    },
    PaletteCommandEntry {
        id: "action.save_project",
        hub: None,
        title_ja: "💾 プロジェクトを保存 (project.veproj.json)",
        title_en: "💾 Save Project Snapshot",
        roadmap_tag: "Core",
        shortcut_hint: Some("Ctrl+S"),
        keywords: &["save", "project", "保存"],
    },
];

pub fn studio_palette_catalog() -> &'static [PaletteCommandEntry] {
    STUDIO_PALETTE_CATALOG
}

pub fn commands_for_hub(hub: StudioHubId) -> Vec<&'static PaletteCommandEntry> {
    STUDIO_PALETTE_CATALOG
        .iter()
        .filter(|e| e.hub == Some(hub))
        .collect()
}

/// ファジー検索結果の 1 ヒット項目
#[derive(Debug, Clone)]
pub struct PaletteSearchHit {
    pub entry: &'static PaletteCommandEntry,
    pub score: i32,
}

/// 決定論的ファジー文字列スコアラー（部分文字列一致・単語先頭一致・順序付き部分列一致）
fn fuzzy_token_score(haystack_lower: &str, needle_lower: &str) -> Option<i32> {
    if needle_lower.is_empty() {
        return Some(0);
    }
    if haystack_lower == needle_lower {
        return Some(240);
    }
    if haystack_lower.starts_with(needle_lower) {
        return Some(180);
    }
    if let Some(pos) = haystack_lower.find(needle_lower) {
        let boundary_bonus = if pos == 0 {
            40
        } else {
            let prev = haystack_lower
                .as_bytes()
                .get(pos.saturating_sub(1))
                .copied();
            if matches!(
                prev,
                Some(b' ' | b'_' | b'.' | b'/' | b'-' | b':' | b'(' | b'#')
            ) {
                32
            } else {
                0
            }
        };
        return Some(120 + boundary_bonus - (pos.min(40) as i32));
    }

    // 文字単位の順序部分列（Subsequence）マッチ + 連続一致ボーナス
    let needle_chars: Vec<char> = needle_lower.chars().collect();
    let mut n_idx = 0usize;
    let mut score = 20i32;
    let mut streak = 0i32;
    for (h_pos, ch) in haystack_lower.chars().enumerate() {
        if n_idx < needle_chars.len() && ch == needle_chars[n_idx] {
            n_idx += 1;
            streak += 1;
            score += 8 + streak * 4;
            if h_pos == 0 {
                score += 12;
            }
        } else {
            streak = 0;
        }
    }
    if n_idx == needle_chars.len() {
        Some(score)
    } else {
        None
    }
}

/// `Ctrl+K` コマンドパレット用の決定論ファジー検索（空白区切り AND トークン・日英・`#番号`・MRU 履歴ブースト対応）
pub fn fuzzy_search_commands(
    query: &str,
    recent_ids: &[String],
    limit: usize,
) -> Vec<PaletteSearchHit> {
    let max_out = limit.max(1);
    let tokens: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();

    let mut hits: Vec<PaletteSearchHit> = Vec::new();
    for (ordinal, entry) in STUDIO_PALETTE_CATALOG.iter().enumerate() {
        let mru_bonus = recent_ids
            .iter()
            .position(|id| id == entry.id)
            .map(|rank| (80_i32 - (rank as i32) * 12).max(12))
            .unwrap_or(0);

        if tokens.is_empty() {
            hits.push(PaletteSearchHit {
                entry,
                score: 1000 + mru_bonus * 10 - (ordinal as i32),
            });
            continue;
        }

        let hub_slug = entry.hub.map(StudioHubId::slug).unwrap_or("");
        let shortcut = entry.shortcut_hint.unwrap_or("").to_lowercase();
        let id_low = entry.id.to_lowercase();
        let ja_low = entry.title_ja.to_lowercase();
        let en_low = entry.title_en.to_lowercase();
        let tag_low = entry.roadmap_tag.to_lowercase();

        let mut total_score = mru_bonus;
        let mut all_matched = true;

        for tok in &tokens {
            let mut best_tok: Option<i32> = None;
            let mut consider = |cand: Option<i32>, weight: i32| {
                if let Some(s) = cand {
                    let weighted = s + weight;
                    best_tok = Some(best_tok.map_or(weighted, |cur| cur.max(weighted)));
                }
            };

            consider(fuzzy_token_score(&ja_low, tok), 30);
            consider(fuzzy_token_score(&en_low, tok), 28);
            consider(fuzzy_token_score(&tag_low, tok), 45);
            consider(fuzzy_token_score(&id_low, tok), 20);
            consider(fuzzy_token_score(hub_slug, tok), 18);
            if !shortcut.is_empty() {
                consider(fuzzy_token_score(&shortcut, tok), 25);
            }
            for kw in entry.keywords {
                consider(fuzzy_token_score(&kw.to_lowercase(), tok), 35);
            }

            match best_tok {
                Some(s) => total_score += s,
                None => {
                    all_matched = false;
                    break;
                }
            }
        }

        if all_matched {
            hits.push(PaletteSearchHit {
                entry,
                score: total_score,
            });
        }
    }

    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.entry.id.cmp(b.entry.id))
    });
    hits.truncate(max_out);
    hits
}

// ============================================================================
// v7.0 #48: A/B スプリット比較ビュー設定モデル
// ============================================================================

/// A/B 比較ワイプの表示モード
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbCompareMode {
    #[default]
    Off,
    /// 左 = A / 右 = B の垂直分割ワイプ
    VerticalSplit,
    /// 上 = A / 下 = B の水平分割ワイプ
    HorizontalSplit,
    /// 左上 = A / 右下 = B の斜めシネマワイプ
    DiagonalSplit,
    /// `|A - B| * diff_gain` によるピクセル差分強調検査
    DifferenceAmplified,
}

impl AbCompareMode {
    pub const ALL: &'static [Self] = &[
        Self::Off,
        Self::VerticalSplit,
        Self::HorizontalSplit,
        Self::DiagonalSplit,
        Self::DifferenceAmplified,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Off => "OFF (通常単一プレビュー)",
            Self::VerticalSplit => "◧ 垂直スプリット (左 A | 右 B)",
            Self::HorizontalSplit => "⬒ 水平スプリット (上 A / 下 B)",
            Self::DiagonalSplit => "◸ 斜めワイプ (左上 A / 右下 B)",
            Self::DifferenceAmplified => "🔬 ピクセル差分強調 (|A - B|)",
        }
    }
}

/// A 側（比較基準リファレンス）に何を表示するか
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbReferenceSource {
    /// カラーグレーディング・ACEScg 色管理・35mm シネマ仕上げ適用前のオリジナル合成画 (Before)
    #[default]
    UngradedBypass,
    /// メモリへキャプチャ保存した任意の静止画フレーム (Frozen Still A)
    FrozenStill,
    /// 同一シーンの指定基準タイムコード秒 (`reference_sec`) のフレーム（ショット間色合わせ確認向け）
    ReferenceTimecode,
    /// 別シーン (`reference_scene_id`) の同一時刻フレーム
    AlternateScene,
}

impl AbReferenceSource {
    pub const ALL: &'static [Self] = &[
        Self::UngradedBypass,
        Self::FrozenStill,
        Self::ReferenceTimecode,
        Self::AlternateScene,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::UngradedBypass => "Before 色補正/仕上げバイパス (未グレーディング A)",
            Self::FrozenStill => "📸 キャプチャ静止画 A (Frozen Still)",
            Self::ReferenceTimecode => "⏱ 基準タイムコード秒フレーム",
            Self::AlternateScene => "🎞 別シーン比較",
        }
    }
}

fn default_ab_split_ratio() -> f32 {
    0.5
}

fn default_ab_diff_gain() -> f32 {
    4.0
}

fn default_true() -> bool {
    true
}

/// シーン単位で保持・Undo 可能な A/B スプリット比較ビュー設定
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AbCompareConfig {
    #[serde(default)]
    pub mode: AbCompareMode,
    #[serde(default)]
    pub reference: AbReferenceSource,
    /// 分割ライン位置 (`0.05..=0.95`, 既定 `0.5`)
    #[serde(default = "default_ab_split_ratio")]
    pub split_ratio: f32,
    /// `AbReferenceSource::ReferenceTimecode` 時の基準時刻（秒）
    #[serde(default)]
    pub reference_sec: f64,
    /// `AbReferenceSource::AlternateScene` 時の比較先シーン ID
    #[serde(default)]
    pub reference_scene_id: Option<SceneId>,
    /// `AbCompareMode::DifferenceAmplified` 時の差分増幅ゲイン (`1.0..=16.0`)
    #[serde(default = "default_ab_diff_gain")]
    pub diff_gain: f32,
    /// 分割境界ラインと A/B インジケータを描画するか
    #[serde(default = "default_true")]
    pub show_divider: bool,
    /// A と B の左右/上下を入れ替えるか
    #[serde(default)]
    pub swap_sides: bool,
}

impl Default for AbCompareConfig {
    fn default() -> Self {
        Self {
            mode: AbCompareMode::Off,
            reference: AbReferenceSource::UngradedBypass,
            split_ratio: default_ab_split_ratio(),
            reference_sec: 0.0,
            reference_scene_id: None,
            diff_gain: default_ab_diff_gain(),
            show_divider: true,
            swap_sides: false,
        }
    }
}

impl AbCompareConfig {
    pub fn is_active(&self) -> bool {
        self.mode != AbCompareMode::Off
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_studio_hubs_cover_all_hub_commands_and_fuzzy_search_works() {
        assert_eq!(StudioHubId::ALL.len(), 6);
        for &hub in StudioHubId::ALL {
            let cmds = commands_for_hub(hub);
            assert!(
                cmds.len() >= 5,
                "hub {:?} should contain at least 5 integrated tools, got {}",
                hub,
                cmds.len()
            );
            assert_eq!(StudioHubId::from_slug(hub.slug()), Some(hub));
        }

        // Japanese keyword search ("ワイヤー消し") -> VFX Paint (#42)
        let wire_hits = fuzzy_search_commands("ワイヤー消し", &[], 5);
        assert!(!wire_hits.is_empty());
        assert_eq!(wire_hits[0].entry.id, "hub.vfx.paint_inpaint");

        // Roadmap number search ("#48") -> Conform/Index & A/B compare
        let v70_hits = fuzzy_search_commands("#48", &[], 10);
        assert!(
            v70_hits
                .iter()
                .any(|h| h.entry.id == "hub.edit.conform_index")
        );
        assert!(
            v70_hits
                .iter()
                .any(|h| h.entry.id == "hub.color.ab_compare")
        );

        // Subsequence fuzzy search ("acso" -> ACEScg/OCIO)
        let aces_hits = fuzzy_search_commands("aces ocio", &[], 5);
        assert!(!aces_hits.is_empty());
        assert_eq!(aces_hits[0].entry.id, "hub.color.aces_ocio");

        // MRU boost surfaces recently executed command at top when query is empty
        let recent = vec!["hub.audio.podcast_director".to_string()];
        let empty_hits = fuzzy_search_commands("", &recent, 5);
        assert_eq!(empty_hits[0].entry.id, "hub.audio.podcast_director");
    }
}
