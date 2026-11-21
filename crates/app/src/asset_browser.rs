use crate::project_io;
use crate::watch_folder::WatchFolderHandle;
use egui::Ui;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use uuid::Uuid;
use ve_audio::{AudioClip, AudioEngineState, AudioGraph, WaveformPeaks};
use ve_core::{
    CommandSource, EditCommand, EffectNode, FilterTargetScope, ItemAlias, ItemKind, RationalTime,
    RgbaColor, ShapeKind, TextAnimator, TextStyle, TimelineItem, Transform2D, TransitionKind,
};
use ve_mcp::EditorSession;
use ve_media::decode_audio_file;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserTab {
    Files,
    Aliases,
    WatchFolder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKindFilter {
    All,
    Audio,
    Video,
    ImagePsd,
}

#[derive(Debug, Clone)]
struct AssetFileEntry {
    path: PathBuf,
    name: String,
    ext: String,
    size_bytes: u64,
}

/// 内蔵アセットブラウザ・クイック配置ツールバー・エイリアス棚・かんしくん風フォルダ監視パネル
/// （MyAssetManager / Nao式ツールボックス / AviUtl2 エイリアス / かんしくん相当）
pub struct AssetBrowserPanel {
    active_tab: BrowserTab,
    // 素材ファイラ
    browse_dir_input: String,
    file_filter: FileKindFilter,
    file_search: String,
    scanned_files: Vec<AssetFileEntry>,
    audition_status: String,
    // エイリアス棚
    alias_name_input: String,
    alias_category_input: String,
    aliases: Vec<ItemAlias>,
    alias_status: String,
    // かんしくん風フォルダ監視
    watch_dir_input: String,
    watcher: Option<WatchFolderHandle>,
    pub watch_auto_advance: bool,
    watch_log: Vec<String>,
    watch_status: String,
}

impl Default for AssetBrowserPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetBrowserPanel {
    pub fn new() -> Self {
        let mut panel = Self {
            active_tab: BrowserTab::Files,
            browse_dir_input: ".".to_string(),
            file_filter: FileKindFilter::All,
            file_search: String::new(),
            scanned_files: Vec::new(),
            audition_status: String::new(),
            alias_name_input: "お気に入り字幕".to_string(),
            alias_category_input: "字幕".to_string(),
            aliases: built_in_starter_aliases(),
            alias_status: "スターターエイリアス読込済み".to_string(),
            watch_dir_input: "./watch_inbox".to_string(),
            watcher: None,
            watch_auto_advance: true,
            watch_log: Vec::new(),
            watch_status: "停止中".to_string(),
        };
        panel.rescan_directory();
        panel
    }

    pub fn is_watching(&self) -> bool {
        self.watcher.is_some()
    }

    /// プロジェクトフォルダを開いた／作成した際に `aliases/` と `assets/` を同期する
    pub fn sync_with_project_folder(&mut self, folder: &ve_core::ProjectFolder) {
        self.browse_dir_input = folder.assets_dir().to_string_lossy().into_owned();
        self.rescan_directory();
        let mut merged = built_in_starter_aliases();
        for user_alias in folder.load_aliases() {
            merged.retain(|a| a.name != user_alias.name);
            merged.push(user_alias);
        }
        self.aliases = merged;
    }

    /// かんしくん風フォルダ監視で安定検出された新着ファイル一覧を回収する
    pub fn poll_watch_folder(&mut self) -> Vec<PathBuf> {
        let Some(watcher) = self.watcher.as_ref() else {
            return Vec::new();
        };
        let files = watcher.poll_ready_files();
        for f in &files {
            let name = f
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.watch_log.push(format!("自動配置: {name}"));
            if self.watch_log.len() > 30 {
                self.watch_log.remove(0);
            }
        }
        files
    }

    pub fn rescan_directory(&mut self) {
        let dir = PathBuf::from(self.browse_dir_input.trim());
        self.scanned_files.clear();
        let Ok(entries) = fs::read_dir(&dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
                .unwrap_or_default();
            if !is_media_ext(&ext) {
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
            self.scanned_files.push(AssetFileEntry {
                path,
                name,
                ext,
                size_bytes,
            });
        }
        self.scanned_files.sort_by(|a, b| a.name.cmp(&b.name));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut Ui,
        session: &mut EditorSession,
        waveforms: &mut BTreeMap<Uuid, WaveformPeaks>,
        proxy_tx: Option<&mpsc::Sender<(Uuid, PathBuf)>>,
        audio_state: &Arc<AudioEngineState>,
    ) -> bool {
        let mut dirty = false;

        // Nao式ツールボックス風: クイック配置バー
        ui.horizontal_wrapped(|ui| {
            ui.strong("🧰 クイック配置:");
            if ui.small_button("＋座布団字幕").clicked() {
                let mut item = TimelineItem::new(
                    "座布団字幕",
                    session.playhead,
                    RationalTime::from_seconds(3.0),
                    ItemKind::Text {
                        content: "解説テロップ（<#ffdd33>強調</>対応）".into(),
                        font_family: "sans-serif".into(),
                        font_size: 46.0,
                        color: RgbaColor::WHITE,
                        animator: TextAnimator::default(),
                    },
                );
                item.transform = Transform2D {
                    x: 120.0,
                    y: 520.0,
                    ..Default::default()
                };
                item.text_style = TextStyle {
                    outline_width: 3.0,
                    box_enabled: true,
                    ..Default::default()
                };
                let id = item.id;
                if session
                    .bus
                    .execute(
                        &mut session.project,
                        EditCommand::AddObject {
                            layer_index: 1,
                            item,
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    session.selected_items = vec![id];
                    dirty = true;
                }
            }
            if ui.small_button("＋角丸ワイプ枠").clicked() {
                let mut item = TimelineItem::new(
                    "角丸ワイプ枠",
                    session.playhead,
                    RationalTime::from_seconds(4.0),
                    ItemKind::Shape {
                        shape: ShapeKind::RoundedRectangle,
                        width: 360.0,
                        height: 220.0,
                        color: RgbaColor {
                            r: 0.16,
                            g: 0.24,
                            b: 0.38,
                            a: 0.9,
                        },
                    },
                );
                item.transform = Transform2D {
                    x: 860.0,
                    y: 40.0,
                    ..Default::default()
                };
                let id = item.id;
                if session
                    .bus
                    .execute(
                        &mut session.project,
                        EditCommand::AddObject {
                            layer_index: 0,
                            item,
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    session.selected_items = vec![id];
                    dirty = true;
                }
            }
            if ui.small_button("＋発光トランジション").clicked() {
                let item = TimelineItem::new(
                    "発光シーンチェンジ",
                    session.playhead,
                    RationalTime::from_seconds(0.8),
                    ItemKind::Transition {
                        transition: TransitionKind::Flash,
                        scope: FilterTargetScope::AllRemaining,
                        softness: 0.08,
                    },
                );
                let id = item.id;
                if session
                    .bus
                    .execute(
                        &mut session.project,
                        EditCommand::AddObject {
                            layer_index: 2,
                            item,
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    session.selected_items = vec![id];
                    dirty = true;
                }
            }
        });

        ui.separator();

        // タブ切替
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.active_tab, BrowserTab::Files, "📂 素材ファイラ");
            ui.selectable_value(&mut self.active_tab, BrowserTab::Aliases, "⭐ エイリアス棚");
            let watch_label = if self.watcher.is_some() {
                "👁 監視中 (かんしくん)"
            } else {
                "👁 フォルダ監視"
            };
            ui.selectable_value(&mut self.active_tab, BrowserTab::WatchFolder, watch_label);
        });
        ui.separator();

        match self.active_tab {
            BrowserTab::Files => {
                dirty |= self.show_files_tab(ui, session, waveforms, proxy_tx, audio_state);
            }
            BrowserTab::Aliases => {
                dirty |= self.show_aliases_tab(ui, session);
            }
            BrowserTab::WatchFolder => {
                self.show_watch_folder_tab(ui);
            }
        }

        dirty
    }

    fn show_files_tab(
        &mut self,
        ui: &mut Ui,
        session: &mut EditorSession,
        waveforms: &mut BTreeMap<Uuid, WaveformPeaks>,
        proxy_tx: Option<&mpsc::Sender<(Uuid, PathBuf)>>,
        audio_state: &Arc<AudioEngineState>,
    ) -> bool {
        let mut dirty = false;
        ui.horizontal(|ui| {
            ui.label("フォルダ:");
            ui.add(egui::TextEdit::singleline(&mut self.browse_dir_input).desired_width(150.0));
            if ui
                .button("📂")
                .on_hover_text("OSフォルダ選択ダイアログを開く")
                .clicked()
                && let Some(dir) = crate::native_dialog::pick_folder("素材フォルダを選択")
            {
                self.browse_dir_input = dir.display().to_string();
                self.rescan_directory();
            }
            if ui.button("🔄 更新").clicked() {
                self.rescan_directory();
            }
            if let Some(folder) = session.folder.as_ref()
                && ui.small_button("assets/").clicked()
            {
                self.browse_dir_input = folder.assets_dir().to_string_lossy().into_owned();
                self.rescan_directory();
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.file_filter, FileKindFilter::All, "すべて");
            ui.selectable_value(&mut self.file_filter, FileKindFilter::Audio, "🎵 音声/SE");
            ui.selectable_value(&mut self.file_filter, FileKindFilter::Video, "🎬 動画");
            ui.selectable_value(
                &mut self.file_filter,
                FileKindFilter::ImagePsd,
                "🖼 画像/PSD",
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.file_search)
                    .hint_text("ファイル名検索...")
                    .desired_width(120.0),
            );
        });

        if !self.audition_status.is_empty() {
            ui.weak(&self.audition_status);
        }

        let query = self.file_search.trim().to_lowercase();
        let entries = self.scanned_files.clone();
        egui::ScrollArea::vertical()
            .id_salt("asset_browser_files_scroll")
            .show(ui, |ui| {
                let mut shown = 0usize;
                for entry in &entries {
                    if !matches_filter(&entry.ext, self.file_filter) {
                        continue;
                    }
                    if !query.is_empty() && !entry.name.to_lowercase().contains(&query) {
                        continue;
                    }
                    shown += 1;
                    ui.horizontal(|ui| {
                        let icon = ext_icon(&entry.ext);
                        ui.label(format!(
                            "{icon} {} ({:.1} KB)",
                            entry.name,
                            entry.size_bytes as f64 / 1024.0
                        ));
                        if is_audio_ext(&entry.ext) && ui.small_button("▶ 試聴").clicked() {
                            self.audition_audio_file(&entry.path, audio_state);
                        }
                        if ui.small_button("＋ 配置").clicked() {
                            let summary = project_io::import_dropped_files(
                                std::slice::from_ref(&entry.path),
                                session,
                                waveforms,
                                proxy_tx,
                            );
                            if summary.imported_count > 0 {
                                dirty = true;
                                self.audition_status = summary.messages.join(" / ");
                            }
                        }
                    });
                }
                if shown == 0 {
                    ui.weak("表示対象のメディアファイルがありません（フォルダパスを変更して「更新」してください）。");
                }
            });

        dirty
    }

    fn audition_audio_file(&mut self, path: &Path, audio_state: &Arc<AudioEngineState>) {
        match decode_audio_file(path) {
            Ok(buf) => {
                let dur = RationalTime::from_seconds(buf.duration_sec().max(0.05));
                let sr = buf.sample_rate.max(8_000);
                let mut preview_graph = AudioGraph::new(sr);
                preview_graph.add_clip(AudioClip {
                    layer_index: 0,
                    timeline_start: RationalTime::ZERO,
                    duration: dur,
                    source_offset: RationalTime::ZERO,
                    gain: 1.0,
                    muted: false,
                    speed: 1.0,
                    loop_playback: false,
                    time_remap_kfs: Vec::new(),
                    effects: Vec::new(),
                    buffer: Arc::new(buf),
                });
                if let Ok(mut g) = audio_state.graph.write() {
                    *g = preview_graph;
                }
                if let Ok(mut c) = audio_state.clock.write() {
                    c.seek(RationalTime::ZERO);
                    c.play();
                }
                self.audition_status = format!(
                    "試聴再生中: {} ({:.2}s)",
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    dur.as_seconds()
                );
            }
            Err(e) => {
                self.audition_status = format!("試聴デコード失敗: {e}");
            }
        }
    }

    fn show_aliases_tab(&mut self, ui: &mut Ui, session: &mut EditorSession) -> bool {
        let mut dirty = false;

        // 選択中アイテムをエイリアスとして保存
        let selected_item = session.selected_items.first().copied().and_then(|sel_id| {
            session.project.active_scene().and_then(|s| {
                s.layers
                    .iter()
                    .flat_map(|l| l.items.iter())
                    .find(|i| i.id == sel_id)
                    .cloned()
            })
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("名前:");
            ui.add(egui::TextEdit::singleline(&mut self.alias_name_input).desired_width(120.0));
            ui.label("分類:");
            ui.add(egui::TextEdit::singleline(&mut self.alias_category_input).desired_width(70.0));
            if ui
                .add_enabled(
                    selected_item.is_some() && !self.alias_name_input.trim().is_empty(),
                    egui::Button::new("💾 選択中をエイリアス保存"),
                )
                .clicked()
                && let Some(item) = selected_item
            {
                let alias = ItemAlias::from_item(
                    self.alias_name_input.trim(),
                    self.alias_category_input.trim(),
                    &item,
                );
                if let Some(folder) = session.folder.as_ref() {
                    match folder.save_alias(&alias) {
                        Ok(p) => {
                            self.alias_status = format!("保存完了: {}", p.display());
                        }
                        Err(e) => {
                            self.alias_status = format!("ディスク保存失敗(メモリ保持): {e}");
                        }
                    }
                } else {
                    self.alias_status = format!("エイリアス登録: {}", alias.name);
                }
                self.aliases.retain(|a| a.name != alias.name);
                self.aliases.push(alias);
            }
        });
        ui.weak(&self.alias_status);
        ui.separator();

        let mut to_insert: Option<ItemAlias> = None;
        let mut to_delete: Option<String> = None;

        egui::ScrollArea::vertical()
            .id_salt("asset_browser_aliases_scroll")
            .show(ui, |ui| {
                for alias in &self.aliases {
                    ui.horizontal(|ui| {
                        let cat = if alias.category.is_empty() {
                            "一般"
                        } else {
                            alias.category.as_str()
                        };
                        ui.label(format!("[{cat}] {}", alias.name));
                        if ui.small_button("＋ 再生ヘッドへ配置").clicked() {
                            to_insert = Some(alias.clone());
                        }
                        if ui.small_button("🗑").clicked() {
                            to_delete = Some(alias.name.clone());
                        }
                    });
                }
            });

        if let Some(alias) = to_insert {
            let item = alias.instantiate_at(session.playhead);
            let target_layer = match &item.kind {
                ItemKind::Audio { .. } => 3,
                ItemKind::FilterObject { .. } | ItemKind::Transition { .. } => 2,
                ItemKind::Text { .. } => 1,
                _ => 0,
            };
            let layer_count = session
                .project
                .active_scene()
                .map(|s| s.layers.len())
                .unwrap_or(1);
            let new_id = item.id;
            if session
                .bus
                .execute(
                    &mut session.project,
                    EditCommand::AddObject {
                        layer_index: target_layer.min(layer_count.saturating_sub(1)),
                        item,
                    },
                    CommandSource::Gui,
                    None,
                )
                .is_ok()
            {
                session.selected_items = vec![new_id];
                self.alias_status = format!("エイリアス配置: {}", alias.name);
                dirty = true;
            }
        }

        if let Some(del_name) = to_delete {
            self.aliases.retain(|a| a.name != del_name);
            if let Some(folder) = session.folder.as_ref() {
                let _ = folder.delete_alias(&del_name);
            }
            self.alias_status = format!("エイリアス削除: {del_name}");
        }

        dirty
    }

    fn show_watch_folder_tab(&mut self, ui: &mut Ui) {
        ui.label("VOICEVOX / A.I.VOICE / 録画ツールの出力フォルダを監視し、新着ファイルを自動でタイムラインへ配置します。");
        ui.horizontal(|ui| {
            ui.label("監視フォルダ:");
            ui.add(egui::TextEdit::singleline(&mut self.watch_dir_input).desired_width(160.0));
            if ui
                .button("📂")
                .on_hover_text("監視するフォルダを選択")
                .clicked()
                && let Some(dir) = crate::native_dialog::pick_folder("監視フォルダを選択")
            {
                self.watch_dir_input = dir.display().to_string();
            }
            if let Some(w) = self.watcher.as_ref() {
                let watching_path = w.dir().display().to_string();
                if ui.button("⏹ 監視停止").clicked() {
                    if let Some(handle) = self.watcher.take() {
                        handle.stop();
                    }
                    self.watch_status = format!("監視停止 ({watching_path})");
                }
            } else if ui.button("▶ 監視開始").clicked() {
                let dir = PathBuf::from(self.watch_dir_input.trim());
                if !dir.exists() {
                    let _ = fs::create_dir_all(&dir);
                }
                match WatchFolderHandle::start(dir.clone()) {
                    Ok(handle) => {
                        self.watcher = Some(handle);
                        self.watch_status = format!("監視中: {}", dir.display());
                    }
                    Err(e) => {
                        self.watch_status = format!("開始失敗: {e}");
                    }
                }
            }
        });
        ui.checkbox(
            &mut self.watch_auto_advance,
            "自動配置後に再生ヘッドをクリップ末尾へ進める（連続投入モード）",
        );
        ui.weak(&self.watch_status);
        ui.separator();
        ui.label("自動取込ログ:");
        egui::ScrollArea::vertical()
            .id_salt("watch_folder_log_scroll")
            .max_height(150.0)
            .show(ui, |ui| {
                if self.watch_log.is_empty() {
                    ui.weak("（新着ファイル待機中）");
                } else {
                    for line in self.watch_log.iter().rev() {
                        ui.monospace(line);
                    }
                }
            });
    }
}

fn built_in_starter_aliases() -> Vec<ItemAlias> {
    let mut emphasis_text = TimelineItem::new(
        "ツッコミ強調テロップ",
        RationalTime::ZERO,
        RationalTime::from_seconds(2.5),
        ItemKind::Text {
            content: "<@ツッコミ黄><s64><b1>ここ重要！</>".into(),
            font_family: "sans-serif".into(),
            font_size: 48.0,
            color: RgbaColor::WHITE,
            animator: TextAnimator::default(),
        },
    );
    emphasis_text.transform = Transform2D {
        x: 160.0,
        y: 480.0,
        ..Default::default()
    };
    emphasis_text.text_style = TextStyle {
        outline_width: 4.0,
        shadow_offset: [4.0, 4.0],
        shadow_blur: 3.0,
        ..Default::default()
    };

    let mut box_sub = TimelineItem::new(
        "解説座布団テロップ",
        RationalTime::ZERO,
        RationalTime::from_seconds(3.5),
        ItemKind::Text {
            content: "解説テキスト（<@解説シアン>ポイント</>）".into(),
            font_family: "sans-serif".into(),
            font_size: 40.0,
            color: RgbaColor::WHITE,
            animator: TextAnimator::default(),
        },
    );
    box_sub.transform = Transform2D {
        x: 100.0,
        y: 540.0,
        ..Default::default()
    };
    box_sub.text_style = TextStyle {
        outline_width: 2.0,
        box_enabled: true,
        ..Default::default()
    };

    let mut wipe_card = TimelineItem::new(
        "小窓ワイプカード",
        RationalTime::ZERO,
        RationalTime::from_seconds(5.0),
        ItemKind::Shape {
            shape: ShapeKind::RoundedRectangle,
            width: 340.0,
            height: 200.0,
            color: RgbaColor {
                r: 0.12,
                g: 0.16,
                b: 0.24,
                a: 0.92,
            },
        },
    );
    wipe_card.transform = Transform2D {
        x: 880.0,
        y: 36.0,
        ..Default::default()
    };
    let mut shadow_params = BTreeMap::new();
    shadow_params.insert("offset_x".into(), 6.0);
    shadow_params.insert("offset_y".into(), 6.0);
    shadow_params.insert("blur".into(), 5.0);
    shadow_params.insert("opacity".into(), 0.55);
    wipe_card.effect_chain.push(EffectNode {
        id: Uuid::new_v4(),
        effect_type: "drop_shadow".into(),
        enabled: true,
        params: shadow_params,
    });

    let flash_tr = TimelineItem::new(
        "発光シーンチェンジ",
        RationalTime::ZERO,
        RationalTime::from_seconds(0.8),
        ItemKind::Transition {
            transition: TransitionKind::Flash,
            scope: FilterTargetScope::AllRemaining,
            softness: 0.08,
        },
    );

    vec![
        ItemAlias::from_item("ツッコミ強調テロップ", "字幕", &emphasis_text),
        ItemAlias::from_item("解説座布団テロップ", "字幕", &box_sub),
        ItemAlias::from_item("小窓ワイプカード", "装飾", &wipe_card),
        ItemAlias::from_item("発光シーンチェンジ", "演出", &flash_tr),
    ]
}

fn is_audio_ext(ext: &str) -> bool {
    matches!(ext, "wav" | "mp3" | "flac" | "ogg" | "m4a" | "aac")
}

fn is_video_ext(ext: &str) -> bool {
    matches!(ext, "mp4" | "mov" | "mkv" | "webm" | "avi")
}

fn is_image_psd_ext(ext: &str) -> bool {
    matches!(ext, "png" | "jpg" | "jpeg" | "webp" | "bmp" | "psd")
}

fn is_media_ext(ext: &str) -> bool {
    is_audio_ext(ext) || is_video_ext(ext) || is_image_psd_ext(ext)
}

fn matches_filter(ext: &str, filter: FileKindFilter) -> bool {
    match filter {
        FileKindFilter::All => true,
        FileKindFilter::Audio => is_audio_ext(ext),
        FileKindFilter::Video => is_video_ext(ext),
        FileKindFilter::ImagePsd => is_image_psd_ext(ext),
    }
}

fn ext_icon(ext: &str) -> &'static str {
    if is_audio_ext(ext) {
        "🎵"
    } else if is_video_ext(ext) {
        "🎬"
    } else if ext == "psd" {
        "🎭"
    } else {
        "🖼"
    }
}
