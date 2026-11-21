use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use uuid::Uuid;
use ve_ai::import_tts_file_pair;
use ve_audio::WaveformPeaks;
use ve_core::{
    AssetRef, CharacterSpriteConfig, CommandSource, EditCommand, HistoryEntry, ItemKind,
    LipSyncMode, MediaKind, Project, ProjectFolder, PsdCharacterConfig, RationalTime, RgbaColor,
    TextAnimator, TimelineItem, content_hash_hex,
};
use ve_mcp::EditorSession;
use ve_media::{FfmpegCli, decode_audio_file};
use ve_render::PsdCharacterAsset;

pub struct OpenProjectResult {
    pub folder: ProjectFolder,
    pub project: Project,
    pub pending_recovery: Vec<HistoryEntry>,
}

#[derive(Debug, Default)]
pub struct ImportSummary {
    pub imported_count: usize,
    pub spawned_proxies: usize,
    pub max_end: Option<RationalTime>,
    pub messages: Vec<String>,
}

/// 新規 1 フォルダプロジェクトを初期化する
pub fn create_new_project_folder(dir: &Path, project: &Project) -> Result<ProjectFolder, String> {
    ProjectFolder::init(dir, project).map_err(|e| e.to_string())
}

/// 既存プロジェクトフォルダを開き、`history.jsonl` に未反映の操作ジャーナルがあれば復旧候補として返す
pub fn open_project_folder(dir: &Path) -> Result<OpenProjectResult, String> {
    let (folder, project) = ProjectFolder::open(dir).map_err(|e| e.to_string())?;
    let pending_recovery = folder.read_history().unwrap_or_default();
    Ok(OpenProjectResult {
        folder,
        project,
        pending_recovery,
    })
}

/// プロジェクトを `project.json` へ停電安全に原子的保存し、ジャーナル `history.jsonl` をコンパクションする
pub fn save_project_snapshot(session: &EditorSession) -> Result<PathBuf, String> {
    let folder = session.folder.as_ref().ok_or_else(|| {
        "プロジェクトフォルダが未設定です（先に「新規」で作成してください）".to_string()
    })?;
    folder
        .save_project(&session.project)
        .map_err(|e| e.to_string())?;
    folder.clear_history().map_err(|e| e.to_string())?;
    Ok(folder.project_json_path())
}

/// 全外部アセットを `assets/` へ収集し、`project.json` を更新保存する
pub fn collect_project_assets(session: &mut EditorSession) -> Result<(usize, usize), String> {
    let folder = session
        .folder
        .clone()
        .ok_or_else(|| "プロジェクトフォルダが未設定です".to_string())?;
    let mut collected = 0usize;
    let mut offline = 0usize;
    for asset in &mut session.project.assets {
        match folder.collect_asset(asset) {
            Ok(_) => collected += 1,
            Err(_) => offline += 1,
        }
    }
    folder
        .save_project(&session.project)
        .map_err(|e| e.to_string())?;
    folder.clear_history().map_err(|e| e.to_string())?;
    Ok((collected, offline))
}

/// ドラッグ＆ドロップされたファイル群を拡張子から自動判別し、CommandBus 経由（Ctrl+Z Undo対応）でタイムラインへ配置する。
/// - 動画 (`.mp4/.mov/.mkv/.webm/.avi`): プローブ＋配置＋バックグラウンド 540p プロキシ生成 (`cache/proxy/`)
/// - 音声 (`.wav/.mp3/.flac/.ogg/.m4a/.aac`): 同名 `.txt`/`.lab` があれば TTS ペア一括取込、なければ音声クリップ＋波形生成
/// - 画像 (`.png/.jpg/.jpeg/.webp/.bmp`): 静止画クリップ配置
/// - PSD (`.psd`): ネイティブ立ち絵スプライト配置
pub fn import_dropped_files(
    paths: &[PathBuf],
    session: &mut EditorSession,
    waveforms: &mut BTreeMap<Uuid, WaveformPeaks>,
    proxy_tx: Option<&mpsc::Sender<(Uuid, PathBuf)>>,
) -> ImportSummary {
    let mut summary = ImportSummary::default();
    let cli = FfmpegCli::default();
    let start_base = session.playhead;

    for path in paths {
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Media".into());

        match ext.as_str() {
            "mp4" | "mov" | "mkv" | "webm" | "avi" => {
                let probe = cli.probe(path).ok();
                let dur_sec = probe
                    .as_ref()
                    .map(|p| p.duration_sec)
                    .filter(|d| d.is_finite() && *d > 0.05)
                    .unwrap_or(5.0);
                let asset_id = Uuid::new_v4();
                session.project.assets.push(AssetRef {
                    id: asset_id,
                    name: file_name.clone(),
                    kind: MediaKind::Video,
                    relative_path: None,
                    fallback_absolute_path: Some(path.clone()),
                    collected: false,
                });

                // 動画に音声トラックがあれば波形ピークを抽出
                if probe.as_ref().and_then(|p| p.audio_sample_rate).is_some()
                    && let Ok(buf) = decode_audio_file(path)
                {
                    waveforms.insert(asset_id, WaveformPeaks::from_buffer(&buf, 100));
                }

                // プロジェクトフォルダがあればバックグラウンドで軽量プロキシを `cache/proxy/` へ生成
                if let (Some(folder), Some(tx)) = (session.folder.as_ref(), proxy_tx) {
                    let key = content_hash_hex(path.to_string_lossy().as_bytes());
                    let proxy_path = folder.cache_proxy_dir().join(format!("{key}_540p.mp4"));
                    let src = path.clone();
                    let tx_clone = tx.clone();
                    summary.spawned_proxies += 1;
                    thread::spawn(move || {
                        if proxy_path.exists()
                            || FfmpegCli::default()
                                .generate_proxy(&src, &proxy_path, 540)
                                .is_ok()
                        {
                            let _ = tx_clone.send((asset_id, proxy_path));
                        }
                    });
                }

                let dur = RationalTime::from_seconds(dur_sec);
                let mut item = TimelineItem::new(
                    file_name.clone(),
                    start_base,
                    dur,
                    ItemKind::Video {
                        asset_id,
                        source_offset: RationalTime::ZERO,
                        volume: 1.0,
                    },
                );
                item.playback.source_duration = Some(dur);
                let end_t = item.end();
                let item_id = item.id;
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
                    session.selected_items = vec![item_id];
                    summary.imported_count += 1;
                    summary.max_end = Some(summary.max_end.map_or(end_t, |cur| cur.max(end_t)));
                    summary
                        .messages
                        .push(format!("動画読込: {file_name} ({dur_sec:.1}s)"));
                }
            }
            "wav" | "mp3" | "flac" | "ogg" | "m4a" | "aac" => {
                // 同名 .txt または .lab が存在する WAV は TTS ファイルペアとして自動一括取込
                let has_tts_sidecar = ext == "wav"
                    && (path.with_extension("txt").exists() || path.with_extension("lab").exists());
                if has_tts_sidecar && let Ok(bundle) = import_tts_file_pair(path) {
                    let asset_id = Uuid::new_v4();
                    if let Ok(buf) = decode_audio_file(path) {
                        waveforms.insert(asset_id, WaveformPeaks::from_buffer(&buf, 100));
                    }
                    session.project.assets.push(AssetRef {
                        id: asset_id,
                        name: file_name.clone(),
                        kind: MediaKind::Audio,
                        relative_path: None,
                        fallback_absolute_path: Some(path.clone()),
                        collected: false,
                    });
                    let mut audio_item = TimelineItem::new(
                        file_name.clone(),
                        start_base,
                        bundle.duration,
                        ItemKind::Audio {
                            asset_id,
                            source_offset: RationalTime::ZERO,
                            volume: 1.0,
                        },
                    );
                    audio_item.playback.source_duration = Some(bundle.duration);
                    audio_item.phoneme_segments = bundle.phonemes.clone();
                    let end_t = audio_item.end();
                    let sub_item = TimelineItem::new(
                        "TTS字幕",
                        start_base,
                        bundle.duration,
                        ItemKind::Text {
                            content: bundle.subtitle_text.clone(),
                            font_family: "sans-serif".to_string(),
                            font_size: 46.0,
                            color: RgbaColor::WHITE,
                            animator: TextAnimator::default(),
                        },
                    );
                    if session
                        .bus
                        .execute(
                            &mut session.project,
                            EditCommand::Batch {
                                label: "drop_tts_pair".into(),
                                commands: vec![
                                    EditCommand::AddObject {
                                        layer_index: 3,
                                        item: audio_item,
                                    },
                                    EditCommand::AddObject {
                                        layer_index: 1,
                                        item: sub_item,
                                    },
                                ],
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        summary.imported_count += 1;
                        summary.max_end = Some(summary.max_end.map_or(end_t, |cur| cur.max(end_t)));
                        summary.messages.push(format!(
                            "TTSペア読込: {file_name} (字幕+音素{}個)",
                            bundle.phonemes.len()
                        ));
                        continue;
                    }
                }

                let (dur_sec, wf_opt) = match decode_audio_file(path) {
                    Ok(buf) => (
                        buf.duration_sec().max(0.1),
                        Some(WaveformPeaks::from_buffer(&buf, 100)),
                    ),
                    Err(_) => (
                        cli.probe(path)
                            .ok()
                            .map(|p| p.duration_sec)
                            .filter(|d| *d > 0.05)
                            .unwrap_or(3.0),
                        None,
                    ),
                };
                let asset_id = Uuid::new_v4();
                if let Some(wf) = wf_opt {
                    waveforms.insert(asset_id, wf);
                }
                session.project.assets.push(AssetRef {
                    id: asset_id,
                    name: file_name.clone(),
                    kind: MediaKind::Audio,
                    relative_path: None,
                    fallback_absolute_path: Some(path.clone()),
                    collected: false,
                });
                let dur = RationalTime::from_seconds(dur_sec);
                let mut item = TimelineItem::new(
                    file_name.clone(),
                    start_base,
                    dur,
                    ItemKind::Audio {
                        asset_id,
                        source_offset: RationalTime::ZERO,
                        volume: 1.0,
                    },
                );
                item.playback.source_duration = Some(dur);
                let end_t = item.end();
                let item_id = item.id;
                if session
                    .bus
                    .execute(
                        &mut session.project,
                        EditCommand::AddObject {
                            layer_index: 3,
                            item,
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    session.selected_items = vec![item_id];
                    summary.imported_count += 1;
                    summary.max_end = Some(summary.max_end.map_or(end_t, |cur| cur.max(end_t)));
                    summary
                        .messages
                        .push(format!("音声読込: {file_name} ({dur_sec:.1}s)"));
                }
            }
            "png" | "jpg" | "jpeg" | "webp" | "bmp" => {
                let asset_id = Uuid::new_v4();
                session.project.assets.push(AssetRef {
                    id: asset_id,
                    name: file_name.clone(),
                    kind: MediaKind::Image,
                    relative_path: None,
                    fallback_absolute_path: Some(path.clone()),
                    collected: false,
                });
                let item = TimelineItem::new(
                    file_name.clone(),
                    start_base,
                    RationalTime::from_seconds(5.0),
                    ItemKind::Image { asset_id },
                );
                let item_id = item.id;
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
                    session.selected_items = vec![item_id];
                    summary.imported_count += 1;
                    summary.messages.push(format!("画像読込: {file_name}"));
                }
            }
            "psd" => {
                let (w, h) = PsdCharacterAsset::from_file(path)
                    .map(|a| (a.width.min(600) as f32, a.height.min(600) as f32))
                    .unwrap_or((300.0, 400.0));
                let item = TimelineItem::new(
                    file_name.clone(),
                    start_base,
                    RationalTime::from_seconds(5.0),
                    ItemKind::CharacterSprite {
                        config: CharacterSpriteConfig {
                            linked_audio_layer: Some(3),
                            mode: LipSyncMode::default(),
                            png_paths: BTreeMap::new(),
                            psd_config: Some(PsdCharacterConfig {
                                psd_path: path.clone(),
                                ..Default::default()
                            }),
                            phoneme_override: Vec::new(),
                        },
                        width: w.max(64.0),
                        height: h.max(64.0),
                    },
                );
                let item_id = item.id;
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
                    session.selected_items = vec![item_id];
                    summary.imported_count += 1;
                    summary.messages.push(format!("PSD立ち絵読込: {file_name}"));
                }
            }
            _ => {
                summary
                    .messages
                    .push(format!("未対応ファイル形式をスキップ: {file_name}"));
            }
        }
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;
    use ve_core::{CommandBus, FrameRate, ProjectSettings, ShapeKind};
    use ve_media::{ExportConfig, ExportFormat, VideoEncoder, write_wav_pcm16};

    #[test]
    fn project_open_detects_history_jsonl_and_replays_crash_recovery() {
        let tmp = tempdir().unwrap();
        let proj_dir = tmp.path().join("CrashProject");
        let initial = Project::new("CrashTest", ProjectSettings::default());
        let folder = create_new_project_folder(&proj_dir, &initial).unwrap();

        // project.json を保存せずに CommandBus 経由で編集（クラッシュ想定）
        let mut live_session = EditorSession::with_folder(initial, folder);
        live_session
            .bus
            .execute(
                &mut live_session.project,
                EditCommand::AddObject {
                    layer_index: 0,
                    item: TimelineItem::new(
                        "UnsavedRect",
                        RationalTime::ZERO,
                        RationalTime::from_seconds(2.0),
                        ItemKind::Shape {
                            shape: ShapeKind::Rectangle,
                            width: 100.0,
                            height: 50.0,
                            color: RgbaColor::WHITE,
                        },
                    ),
                },
                CommandSource::Gui,
                None,
            )
            .unwrap();

        // 再起動してフォルダを開く -> 未反映の history.jsonl 1件を検出
        let mut opened = open_project_folder(&proj_dir).unwrap();
        assert_eq!(opened.pending_recovery.len(), 1);
        assert_eq!(
            opened.project.active_scene().unwrap().layers[0].items.len(),
            0
        );

        // クラッシュ復旧リプレイ
        let count =
            CommandBus::replay_history(&mut opened.project, &opened.pending_recovery).unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            opened.project.active_scene().unwrap().layers[0].items.len(),
            1
        );

        // 正常スナップショット保存 -> history.jsonl がコンパクションされる
        let clean_session = EditorSession::with_folder(opened.project, opened.folder);
        save_project_snapshot(&clean_session).unwrap();
        let reopened = open_project_folder(&proj_dir).unwrap();
        assert!(reopened.pending_recovery.is_empty());
        assert_eq!(
            reopened.project.active_scene().unwrap().layers[0]
                .items
                .len(),
            1
        );
    }

    #[test]
    fn drag_and_drop_imports_video_image_and_tts_pair_undoably() {
        let tmp = tempdir().unwrap();
        let proj_dir = tmp.path().join("DropProject");
        let project = Project::new("DropTest", ProjectSettings::default());
        let folder = create_new_project_folder(&proj_dir, &project).unwrap();
        let mut session = EditorSession::with_folder(project, folder);
        let mut waveforms = BTreeMap::new();

        // 1) ダミーPNG
        let png_path = tmp.path().join("photo.png");
        fs::write(&png_path, b"PNG_PLACEHOLDER").unwrap();

        // 2) TTSペア (voice.wav + voice.txt + voice.lab)
        let wav_path = tmp.path().join("voice.wav");
        write_wav_pcm16(&wav_path, &vec![0.2f32; 24_000], 48_000, 1).unwrap();
        fs::write(tmp.path().join("voice.txt"), "テスト発話").unwrap();
        fs::write(tmp.path().join("voice.lab"), "0 2000000 a\n").unwrap();

        // 3) 実MP4動画
        let mp4_path = tmp.path().join("clip.mp4");
        let cfg = ExportConfig {
            width: 64,
            height: 36,
            fps: FrameRate::FPS_30,
            format: ExportFormat::Mp4H264,
            crf: 23,
            encoder: VideoEncoder::Software,
        };
        let mut exp = FfmpegCli::default()
            .start_export(&mp4_path, &cfg, None)
            .unwrap();
        let frame = vec![200u8; 64 * 36 * 4];
        for _ in 0..10 {
            exp.write_rgba8_frame(&frame).unwrap();
        }
        exp.finish().unwrap();

        let (tx, rx) = mpsc::channel();
        let summary = import_dropped_files(
            &[mp4_path, png_path, wav_path],
            &mut session,
            &mut waveforms,
            Some(&tx),
        );
        assert_eq!(summary.imported_count, 3);
        assert_eq!(summary.spawned_proxies, 1);
        assert_eq!(session.project.assets.len(), 3);
        assert!(!waveforms.is_empty());

        // バックグラウンドプロキシ生成の完了を受信
        let (_aid, proxy_path) = rx.recv_timeout(std::time::Duration::from_secs(15)).unwrap();
        assert!(proxy_path.exists());

        // 3回のドロップ操作はすべて Ctrl+Z (Undo) で取り消し可能
        assert!(session.bus.undo(&mut session.project));
        assert!(session.bus.undo(&mut session.project));
        assert!(session.bus.undo(&mut session.project));
        let scene = session.project.active_scene().unwrap();
        assert!(scene.layers.iter().all(|l| l.items.is_empty()));
    }
}
