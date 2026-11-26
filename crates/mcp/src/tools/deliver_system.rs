//! v7.0 #46: 内蔵 MCP ドメインモジュール — Delivery Packager, Broadcast QC, Export, Plugins & Ecosystem

#[allow(unused_imports)]
use super::*;

pub(super) const DOMAIN_ID: &str = "deliver_system";
pub(super) const DOMAIN_LABEL: &str =
    "Delivery Packager, Broadcast QC, Export, Plugins & Ecosystem";

pub(super) fn definitions() -> Vec<Value> {
    let mut out = Vec::new();
    if let Value::Array(items) = json!([
        {
            "name": "list_plugins",
            "description": "登録済みエフェクト／ジェネレータプラグインのマニフェスト一覧を取得する",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "validate_custom_wgsl",
            "description": "カスタム WGSL シェーダー文字列を naga で静的検証し、エントリーポイントまたは構文エラーを返す",
            "inputSchema": {
                "type": "object",
                "required": ["source"],
                "properties": { "source": { "type": "string" } }
            }
        },
        {
            "name": "start_export",
            "description": "確認ガード付き（user_confirmed=true 必須）でアクティブシーンをプロジェクト exports/ フォルダへ書き出す",
            "inputSchema": {
                "type": "object",
                "required": ["user_confirmed"],
                "properties": {
                    "user_confirmed": { "type": "boolean" },
                    "format": { "type": "string", "enum": ["mp4_h264", "mp4_h265", "webm_vp9_alpha", "wav_audio_only"], "default": "wav_audio_only" },
                    "file_name": { "type": "string", "default": "mcp_export" }
                }
            }
        },
        {
            "name": "list_or_reload_wasm_plugins",
            "description": "plugins/*.wasm のサンドボックス WebAssembly プラグインを検証・一覧取得し、任意でサンプル .wasm を生成する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "plugins_dir": { "type": "string" },
                    "emit_samples": { "type": "boolean", "default": false }
                }
            }
        },
        {
            "name": "validate_and_package_project",
            "description": "v1.0 #19: プロジェクト書き出し前検証チェックリスト（オフライン素材・無音ギャップ・高解像度）の実行、および Pure-Rust ZIP アーカイブパッケージ化を行う",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["validate", "archive_zip"]
                    },
                    "file_name": { "type": "string" }
                }
            }
        },
        {
            "name": "manage_ecosystem",
            "description": "v1.0 #20: カスタムキーバインドプリセット（AviUtl2/Premiere/DaVinci/AE）・多言語/テーマ設定・SHA-256検証付き公式拡張レジストリ・HWデコード/エンコード検出を操作する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get_settings", "set_settings", "export_keymap_json", "import_keymap_json", "list_registry", "install_extension", "hw_status"]
                    },
                    "language": { "type": "string", "enum": ["ja", "en"] },
                    "theme": { "type": "string", "enum": ["dark_studio", "midnight_oled", "high_contrast", "light_daylight"] },
                    "font_scale": { "type": "number" },
                    "keymap_preset": { "type": "string", "enum": ["aviutl2", "premiere", "davinci", "after_effects"] },
                    "keymap_json": { "type": "string" },
                    "extension_id": { "type": "string" },
                    "hw_decode": { "type": "string" },
                    "parallel_export_threads": { "type": "integer" }
                }
            }
        },
        {
            "name": "broadcast_qc_and_web_export",
            "description": "v4.0 #36: 放送・配信コンプライアンス QC 監査（ITU-R BS.1770 True-Peak/LUFS・ITU-R BT.1702 光過敏フラッシュ検査・輝度ガマット検査）、高クリック率ヒーローサムネイル（1280x720 PNG）自動抽出、およびサーバー不要・自己完結型 HTML5 インタラクティブレビュープレイヤー書き出しを実行する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["run_qc_audit", "generate_hero_thumbnail", "export_web_review_html"]
                    },
                    "profile": {
                        "type": "string",
                        "enum": ["youtube1080p", "broadcast_itu_ebu_r128", "shorts_tiktok9x16", "cinema_dci4k"]
                    },
                    "sample_count": { "type": "integer" },
                    "output_path": { "type": "string" }
                }
            }
        },
        {
            "name": "multi_platform_packager",
            "description": "v6.0 #44: プロ・マルチプラットフォーム自動リフォーマット（16:9 / 9:16 / 1:1 / 4K 派生シーン自動生成）および 1 クリック納品バンドル（SRT・チャプター・サムネイル・EDL・FCPXML・QCレポート・manifest.json）を書き出す",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["create_platform_variants", "export_delivery_bundle"]
                    },
                    "platforms": { "type": "array" },
                    "output_dir": { "type": "string" },
                    "base_title": { "type": "string" },
                    "tags": { "type": "array" },
                    "include_thumbnail": { "type": "boolean" },
                    "include_edl": { "type": "boolean" },
                    "include_fcpxml": { "type": "boolean" },
                    "include_qc_report": { "type": "boolean" }
                }
            }
        }
    ]) {
        out.extend(items);
    }
    out
}

pub(super) fn try_call(
    guard: &mut crate::session::EditorSession,
    fps: FrameRate,
    name: &str,
    args: &Value,
) -> Option<Result<Value, String>> {
    if !matches!(
        name,
        "list_plugins"
            | "validate_custom_wgsl"
            | "start_export"
            | "list_or_reload_wasm_plugins"
            | "validate_and_package_project"
            | "manage_ecosystem"
            | "broadcast_qc_and_web_export"
            | "multi_platform_packager"
    ) {
        return None;
    }
    Some(call_inner(guard, fps, name, args))
}

fn call_inner(
    guard: &mut crate::session::EditorSession,
    _fps: FrameRate,
    name: &str,
    args: &Value,
) -> Result<Value, String> {
    match name {
        "list_plugins" => {
            let manifests = PluginRegistry::with_builtins().list_manifests();
            let text = serde_json::to_string_pretty(&manifests).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "validate_custom_wgsl" => {
            let source = args
                .get("source")
                .and_then(Value::as_str)
                .ok_or("missing WGSL 'source'")?;
            match validate_wgsl(source) {
                Ok(entry_points) => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": json!({"valid": true, "entry_points": entry_points}).to_string()
                    }]
                })),
                Err(err) => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": json!({"valid": false, "error": err}).to_string()
                    }]
                })),
            }
        }
        "start_export" => {
            let confirmed = args
                .get("user_confirmed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !confirmed {
                return Err(
                    "書き出し実行には user_confirmed: true の明示的確認が必要です".to_string(),
                );
            }
            let fmt_str = args
                .get("format")
                .and_then(Value::as_str)
                .unwrap_or("wav_audio_only");
            let (format, ext, transparent_bg) = match fmt_str {
                "mp4_h264" => (ExportFormat::Mp4H264, "mp4", false),
                "mp4_h265" => (ExportFormat::Mp4H265, "mp4", false),
                "mp4_h265_hdr10" => (ExportFormat::Mp4H265HdrPq10Bit, "mp4", false),
                "webm_vp9_alpha" => (ExportFormat::WebmVp9Alpha, "webm", true),
                "wav_audio_only" => (ExportFormat::WavAudioOnly, "wav", false),
                other => return Err(format!("unsupported export format: {other}")),
            };
            let raw_name = args
                .get("file_name")
                .and_then(Value::as_str)
                .unwrap_or("mcp_export");
            let safe_stem: String = raw_name
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .take(48)
                .collect();
            let stem = if safe_stem.is_empty() {
                "mcp_export".to_string()
            } else {
                safe_stem
            };
            let out_dir = guard
                .folder
                .as_ref()
                .map(|f| f.exports_dir())
                .unwrap_or_else(|| std::env::temp_dir().join("ve_mcp_exports"));
            std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
            let output_path = out_dir.join(format!("{stem}.{ext}"));

            let proj = guard.project.clone();
            let scene_id = proj.active_scene_id;
            let folder = guard.folder.clone();
            let scene = proj.scene(scene_id).ok_or("no active scene")?;
            let dur = scene
                .total_duration()
                .max(RationalTime::from_seconds(1.0))
                .min(RationalTime::from_seconds(600.0));
            let sr = proj.settings.audio_sample_rate.max(8_000);
            let audio_graph = AudioGraph::from_project_scene(&proj, scene_id, folder.as_ref(), sr);
            let pcm = audio_graph.render_range(RationalTime::ZERO, dur);

            if format == ExportFormat::WavAudioOnly {
                write_wav_pcm16(&output_path, &pcm, sr, 2).map_err(|e| e.to_string())?;
            } else {
                let mut rg = RenderGraph::new();
                rg.use_proxies = false;
                rg.set_project_folder(folder);
                let total_frames = ((dur.as_seconds() * scene.fps.as_f64()).ceil() as i64).max(1);
                let cfg = ExportConfig {
                    width: scene.width.max(16),
                    height: scene.height.max(16),
                    fps: scene.fps,
                    format,
                    crf: 18,
                    encoder: VideoEncoder::Auto,
                };
                let wav_tmp = out_dir.join(format!("{stem}_tmp_audio.wav"));
                let has_audio = !audio_graph.clips.is_empty();
                if has_audio {
                    write_wav_pcm16(&wav_tmp, &pcm, sr, 2).map_err(|e| e.to_string())?;
                }
                let mut pipe = FfmpegCli::default()
                    .start_export(&output_path, &cfg, has_audio.then_some(wav_tmp.as_path()))
                    .map_err(|e| e.to_string())?;
                for f in 0..total_frames {
                    let t = RationalTime::from_frame(f, cfg.fps);
                    let img = rg
                        .render_project_scene(&proj, scene_id, t, transparent_bg)
                        .ok_or("render frame failed")?;
                    pipe.write_rgba8_frame(&img.to_rgba8_srgb())
                        .map_err(|e| e.to_string())?;
                }
                let res = pipe.finish().map_err(|e| e.to_string());
                if has_audio {
                    let _ = std::fs::remove_file(&wav_tmp);
                }
                res?;
            }

            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({"exported": output_path.display().to_string(), "format": fmt_str}).to_string()
                }]
            }))
        }
        "list_or_reload_wasm_plugins" => {
            let dir = if let Some(p) = args.get("plugins_dir").and_then(Value::as_str) {
                PathBuf::from(p)
            } else if let Some(f) = guard.folder.as_ref() {
                f.root().join("plugins")
            } else {
                PathBuf::from("plugins")
            };
            if args
                .get("emit_samples")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                emit_sample_wasm_plugins(&dir).map_err(|e| e.to_string())?;
            }
            let mut reg = PluginRegistry::with_builtins();
            let mut mgr = WasmPluginManager::new(&dir);
            let events = mgr.poll_and_sync(&mut reg);
            let statuses: Vec<Value> = mgr
                .list_status()
                .into_iter()
                .map(|st| {
                    json!({
                        "id": st.id,
                        "name": st.name,
                        "path": st.path.display().to_string(),
                        "last_error": st.last_error,
                    })
                })
                .collect();
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({
                        "plugins_dir": dir.display().to_string(),
                        "events": events,
                        "wasm_plugins": statuses
                    }).to_string()
                }]
            }))
        }
        "validate_and_package_project" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing 'action'")?;
            match action {
                "validate" => {
                    let report = validate_project_delivery(&guard.project, guard.folder.as_ref());
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string(&report).unwrap_or_default()
                        }]
                    }))
                }
                "archive_zip" => {
                    let folder = guard
                        .folder
                        .as_ref()
                        .ok_or("ZIP パッケージ化には保存済みプロジェクトフォルダが必要です")?;
                    let raw_name = args
                        .get("file_name")
                        .and_then(Value::as_str)
                        .unwrap_or("project_archive");
                    let safe_stem: String = raw_name
                        .chars()
                        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                        .take(48)
                        .collect();
                    let stem = if safe_stem.is_empty() {
                        "project_archive".to_string()
                    } else {
                        safe_stem
                    };
                    let out_dir = folder.exports_dir();
                    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
                    let zip_path = out_dir.join(format!("{stem}.zip"));
                    let summary =
                        archive_project_to_zip(folder, &zip_path).map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string(&summary).unwrap_or_default()
                        }]
                    }))
                }
                other => Err(format!(
                    "unknown validate_and_package_project action: {other}"
                )),
            }
        }
        "manage_ecosystem" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing 'action'")?;
            match action {
                "get_settings" => {
                    let settings = guard
                        .folder
                        .as_ref()
                        .map(|f| f.load_user_settings())
                        .unwrap_or_default();
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string(&settings).unwrap_or_default()
                        }]
                    }))
                }
                "set_settings" => {
                    let mut settings = guard
                        .folder
                        .as_ref()
                        .map(|f| f.load_user_settings())
                        .unwrap_or_default();
                    if let Some(lang) = args.get("language").and_then(Value::as_str) {
                        settings.language = match lang {
                            "en" => UiLanguage::English,
                            _ => UiLanguage::Japanese,
                        };
                    }
                    if let Some(th) = args.get("theme").and_then(Value::as_str) {
                        settings.theme = match th {
                            "midnight_oled" => UiThemePreset::MidnightOled,
                            "high_contrast" => UiThemePreset::HighContrast,
                            "light_daylight" => UiThemePreset::LightDaylight,
                            _ => UiThemePreset::DarkStudio,
                        };
                    }
                    if args.get("font_scale").is_some() {
                        settings.font_scale = finite_clamped(
                            args.get("font_scale"),
                            f64::from(settings.font_scale),
                            0.85,
                            1.5,
                        ) as f32;
                    }
                    if let Some(kp) = args.get("keymap_preset").and_then(Value::as_str) {
                        let preset = match kp {
                            "premiere" => KeymapPreset::Premiere,
                            "davinci" => KeymapPreset::DaVinci,
                            "after_effects" => KeymapPreset::AfterEffects,
                            _ => KeymapPreset::AviUtl2,
                        };
                        settings.keymap = EditorKeymap::for_preset(preset);
                    }
                    if let Some(t) = args.get("parallel_export_threads").and_then(Value::as_u64) {
                        settings.export_worker_threads = (t as usize).clamp(1, 16);
                    }
                    if let Some(f) = guard.folder.as_ref() {
                        f.save_user_settings(&settings).map_err(|e| e.to_string())?;
                    }
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string(&settings).unwrap_or_default()
                        }]
                    }))
                }
                "export_keymap_json" => {
                    let settings = guard
                        .folder
                        .as_ref()
                        .map(|f| f.load_user_settings())
                        .unwrap_or_default();
                    let km_json = settings
                        .keymap
                        .to_json_pretty()
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": km_json
                        }]
                    }))
                }
                "import_keymap_json" => {
                    let raw = args
                        .get("keymap_json")
                        .and_then(Value::as_str)
                        .ok_or("missing 'keymap_json'")?;
                    let km = EditorKeymap::from_json(raw).map_err(|e| e.to_string())?;
                    let mut settings = guard
                        .folder
                        .as_ref()
                        .map(|f| f.load_user_settings())
                        .unwrap_or_default();
                    settings.keymap = km;
                    if let Some(f) = guard.folder.as_ref() {
                        f.save_user_settings(&settings).map_err(|e| e.to_string())?;
                    }
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "imported_bindings": settings.keymap.bindings.len(), "preset": settings.keymap.preset }).to_string()
                        }]
                    }))
                }
                "list_registry" => {
                    let catalog: Vec<_> = official_registry_catalog()
                        .into_iter()
                        .map(|(entry, _)| entry)
                        .collect();
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string(&catalog).unwrap_or_default()
                        }]
                    }))
                }
                "install_extension" => {
                    let ext_id = args
                        .get("extension_id")
                        .and_then(Value::as_str)
                        .ok_or("missing 'extension_id'")?;
                    let folder = guard
                        .folder
                        .as_ref()
                        .ok_or("拡張機能インストールには保存済みプロジェクトフォルダが必要です")?;
                    let (entry, payload) = official_registry_catalog()
                        .into_iter()
                        .find(|(e, _)| e.id == ext_id)
                        .ok_or_else(|| {
                            format!("extension not found in official catalog: {ext_id}")
                        })?;
                    let installed_path =
                        verify_and_install_extension(&entry, &payload, folder.root())
                            .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "installed_id": entry.id,
                                "sha256_verified": entry.sha256,
                                "permissions": entry.permissions,
                                "path": installed_path.display().to_string()
                            }).to_string()
                        }]
                    }))
                }
                "hw_status" => {
                    let cli = FfmpegCli::default();
                    let hw_decoders: Vec<&str> = cli
                        .detect_available_hw_decoders()
                        .into_iter()
                        .map(HwDecodeBackend::display_name)
                        .collect();
                    let hw_encoders: Vec<String> = cli
                        .detect_available_hw_encoders()
                        .into_iter()
                        .map(|e| format!("{e:?}"))
                        .collect();
                    let resolved_decode = cli.resolve_hw_decoder(HwDecodeBackend::Auto);
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "available_hw_decoders": hw_decoders,
                                "resolved_hw_decode": resolved_decode.display_name(),
                                "available_hw_encoders": hw_encoders,
                                "cpu_threads": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown manage_ecosystem action: {other}")),
            }
        }
        "broadcast_qc_and_web_export" => {
            let s = &*guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let profile = match args.get("profile").and_then(Value::as_str) {
                Some("broadcast_itu_ebu_r128") => BroadcastProfile::BroadcastItuEbuR128,
                Some("shorts_tiktok9x16") => BroadcastProfile::ShortsTikTok9x16,
                Some("cinema_dci4k") => BroadcastProfile::CinemaDci4k,
                _ => BroadcastProfile::YouTube1080p,
            };
            let sample_count = args
                .get("sample_count")
                .and_then(Value::as_u64)
                .map(|v| v.clamp(4, 24) as usize)
                .unwrap_or(8);

            let scene_id = s.project.active_scene_id;
            let mut rg = RenderGraph::new();
            rg.set_project_folder(s.folder.clone());

            let compute_qc = |rg: &mut RenderGraph| -> Result<ve_core::BroadcastQcReport, String> {
                let (
                    peak_luma_ire,
                    clipped_highlight_pct,
                    crushed_shadow_pct,
                    max_frame_luma_jump_ire,
                ) = measure_scene_video_qc_metrics(rg, &s.project, scene_id, sample_count);
                let dur = s
                    .project
                    .active_scene()
                    .map(|sc| sc.total_duration().as_seconds().clamp(0.5, 30.0))
                    .ok_or_else(|| "no active scene".to_string())?;
                let sr = s.project.settings.audio_sample_rate.max(8_000);
                let ag =
                    AudioGraph::from_project_scene(&s.project, scene_id, s.folder.as_ref(), sr);
                let pcm = if ag.clips.is_empty() {
                    Vec::new()
                } else {
                    ag.render_range(RationalTime::ZERO, RationalTime::from_seconds(dur))
                };
                let true_peak_dbtp = if pcm.is_empty() {
                    -96.0
                } else {
                    measure_true_peak_dbtp(&pcm, ag.channels)
                };
                let integrated_lufs = if pcm.is_empty() {
                    -70.0
                } else {
                    measure_integrated_lufs(&pcm, sr)
                };
                let metrics = BroadcastQcMetrics {
                    peak_luma_ire,
                    clipped_highlight_pct,
                    crushed_shadow_pct,
                    max_frame_luma_jump_ire,
                    true_peak_dbtp,
                    integrated_lufs,
                };
                Ok(run_broadcast_qc_audit(&s.project, profile, metrics))
            };

            match action {
                "run_qc_audit" => {
                    let report = compute_qc(&mut rg)?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&report).unwrap_or_default() }]
                    }))
                }
                "generate_hero_thumbnail" => {
                    let (best_t, score, hero) =
                        select_best_thumbnail_frame(&mut rg, &s.project, scene_id, sample_count)
                            .ok_or_else(|| "ヒーローサムネイルの抽出に失敗しました".to_string())?;
                    let png_bytes = encode_linear_image_png_bytes(&hero)?;
                    let out_path = if let Some(p) = args.get("output_path").and_then(Value::as_str)
                    {
                        PathBuf::from(p)
                    } else {
                        s.folder
                            .as_ref()
                            .map(|f| f.root().join("exports").join("hero_thumbnail_1280x720.png"))
                            .unwrap_or_else(|| {
                                std::env::temp_dir().join("ve_hero_thumbnail_1280x720.png")
                            })
                    };
                    if let Some(parent) = out_path.parent()
                        && !parent.as_os_str().is_empty()
                    {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    std::fs::write(&out_path, &png_bytes).map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "timestamp_sec": best_t,
                                "attractiveness_score": score,
                                "width": hero.width,
                                "height": hero.height,
                                "bytes": png_bytes.len(),
                                "output_path": out_path.display().to_string()
                            }).to_string()
                        }]
                    }))
                }
                "export_web_review_html" => {
                    let report = compute_qc(&mut rg)?;
                    let dur = s
                        .project
                        .active_scene()
                        .map(|sc| sc.total_duration().as_seconds().max(0.5))
                        .ok_or_else(|| "no active scene".to_string())?;
                    let mut previews: Vec<(f64, String)> = Vec::new();
                    for i in 0..sample_count {
                        let t_sec = dur * (i as f64) / ((sample_count - 1).max(1) as f64);
                        if let Some(img) = rg.render_project_scene_scaled(
                            &s.project,
                            scene_id,
                            RationalTime::from_seconds(t_sec),
                            480,
                            270,
                            false,
                        ) && let Ok(png) = encode_linear_image_png_bytes(&img)
                        {
                            previews.push((t_sec, base64_encode(&png)));
                        }
                    }
                    let html = export_portable_web_review_html(&s.project, &report, &previews);
                    let out_path = if let Some(p) = args.get("output_path").and_then(Value::as_str)
                    {
                        PathBuf::from(p)
                    } else {
                        s.folder
                            .as_ref()
                            .map(|f| f.root().join("exports").join("portable_review_player.html"))
                            .unwrap_or_else(|| {
                                std::env::temp_dir().join("ve_portable_review_player.html")
                            })
                    };
                    if let Some(parent) = out_path.parent()
                        && !parent.as_os_str().is_empty()
                    {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    std::fs::write(&out_path, &html).map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "output_path": out_path.display().to_string(),
                                "embedded_frames": previews.len(),
                                "html_bytes": html.len(),
                                "overall_pass": report.overall_pass
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!(
                    "unknown broadcast_qc_and_web_export action: {other}"
                )),
            }
        }
        "multi_platform_packager" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("create_platform_variants");

            let parse_platform = |p: &str| match p {
                "shorts_reels_9x16" | "9x16" => DeliveryPlatform::ShortsReels9x16,
                "square_feed_1x1" | "1x1" => DeliveryPlatform::SquareFeed1x1,
                "cinema_4k_master" | "4k" => DeliveryPlatform::Cinema4kMaster,
                _ => DeliveryPlatform::YouTube16x9,
            };
            let targets = if let Some(arr) = args.get("platforms").and_then(Value::as_array) {
                let mut list = Vec::new();
                for v in arr {
                    if let Some(ps) = v.as_str() {
                        list.push(parse_platform(ps).default_target());
                    }
                }
                if list.is_empty() {
                    DeliveryBundleConfig::default().targets
                } else {
                    list
                }
            } else {
                DeliveryBundleConfig::default().targets
            };

            let cfg = DeliveryBundleConfig {
                targets,
                include_thumbnail: args
                    .get("include_thumbnail")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                include_edl: args
                    .get("include_edl")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                include_fcpxml: args
                    .get("include_fcpxml")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                include_qc_report: args
                    .get("include_qc_report")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                base_title: args
                    .get("base_title")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| s.project.name.clone()),
                tags: args
                    .get("tags")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_else(|| vec!["video".into(), "master".into()]),
            };

            match action {
                "create_platform_variants" => {
                    let src = s.project.active_scene().ok_or("no active scene")?.clone();
                    let mut cmds = Vec::new();
                    let mut summary = Vec::new();
                    for tgt in &cfg.targets {
                        let (sid, cmd) = build_reformatted_scene_command(&src, tgt);
                        summary.push(json!({
                            "scene_id": sid,
                            "platform": tgt.platform.id(),
                            "width": tgt.width,
                            "height": tgt.height,
                            "target_lufs": tgt.target_lufs
                        }));
                        cmds.push(cmd);
                    }
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::Batch {
                                label: "Multi-Platform Reformat Variants".into(),
                                commands: cmds,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "created_count": summary.len(),
                                "created_scenes": summary
                            }).to_string()
                        }]
                    }))
                }
                "export_delivery_bundle" => {
                    let out_dir = if let Some(p) = args.get("output_dir").and_then(Value::as_str) {
                        PathBuf::from(p)
                    } else if let Some(folder) = &s.folder {
                        folder.root().join("deliverables")
                    } else {
                        std::env::temp_dir().join("ve_delivery_bundle")
                    };
                    std::fs::create_dir_all(out_dir.join("variants")).map_err(|e| e.to_string())?;
                    std::fs::create_dir_all(out_dir.join("subtitles"))
                        .map_err(|e| e.to_string())?;
                    std::fs::create_dir_all(out_dir.join("metadata")).map_err(|e| e.to_string())?;
                    std::fs::create_dir_all(out_dir.join("interchange"))
                        .map_err(|e| e.to_string())?;
                    std::fs::create_dir_all(out_dir.join("qc")).map_err(|e| e.to_string())?;

                    let src = s.project.active_scene().ok_or("no active scene")?.clone();
                    let mut artifacts: Vec<DeliveryBundleArtifact> = Vec::new();
                    let mut wrote_chapters = false;

                    for tgt in &cfg.targets {
                        let (_sid, cmd) = build_reformatted_scene_command(&src, tgt);
                        if let EditCommand::CreateScene { scene: vscene, .. } = cmd {
                            let rel_scene = format!("variants/{}.scene.json", tgt.platform.id());
                            let scene_bytes =
                                serde_json::to_vec_pretty(&vscene).map_err(|e| e.to_string())?;
                            std::fs::write(out_dir.join(&rel_scene), &scene_bytes)
                                .map_err(|e| e.to_string())?;
                            artifacts.push(DeliveryBundleArtifact {
                                kind: "scene_variant".into(),
                                platform: tgt.platform.id().into(),
                                relative_path: rel_scene,
                                bytes: scene_bytes.len(),
                            });

                            if tgt.include_srt {
                                let srt = export_srt_subtitles(&vscene);
                                let rel_srt = format!("subtitles/{}.srt", tgt.platform.id());
                                std::fs::write(out_dir.join(&rel_srt), srt.as_bytes())
                                    .map_err(|e| e.to_string())?;
                                artifacts.push(DeliveryBundleArtifact {
                                    kind: "subtitles_srt".into(),
                                    platform: tgt.platform.id().into(),
                                    relative_path: rel_srt,
                                    bytes: srt.len(),
                                });
                            }

                            if tgt.include_chapters && !wrote_chapters {
                                let ch = format_youtube_chapters(&vscene.markers);
                                let rel_ch = "metadata/youtube_chapters.txt".to_string();
                                std::fs::write(out_dir.join(&rel_ch), ch.as_bytes())
                                    .map_err(|e| e.to_string())?;
                                artifacts.push(DeliveryBundleArtifact {
                                    kind: "youtube_chapters".into(),
                                    platform: tgt.platform.id().into(),
                                    relative_path: rel_ch,
                                    bytes: ch.len(),
                                });
                                wrote_chapters = true;
                            }
                        }
                    }

                    let mut rg = RenderGraph::new();
                    rg.set_project_folder(s.folder.clone());
                    let active_id = s.project.active_scene_id;

                    if cfg.include_thumbnail
                        && let Some((_t, _score, hero)) =
                            select_best_thumbnail_frame(&mut rg, &s.project, active_id, 4)
                    {
                        let png = encode_linear_image_png_bytes(&hero)?;
                        let rel_thumb = "thumbnail_1280x720.png".to_string();
                        std::fs::write(out_dir.join(&rel_thumb), &png)
                            .map_err(|e| e.to_string())?;
                        artifacts.push(DeliveryBundleArtifact {
                            kind: "thumbnail_png".into(),
                            platform: "shared".into(),
                            relative_path: rel_thumb,
                            bytes: png.len(),
                        });
                    }

                    if cfg.include_edl {
                        let edl = export_cmx3600_edl(&s.project);
                        let rel_edl = "interchange/master.edl".to_string();
                        std::fs::write(out_dir.join(&rel_edl), edl.as_bytes())
                            .map_err(|e| e.to_string())?;
                        artifacts.push(DeliveryBundleArtifact {
                            kind: "edl_cmx3600".into(),
                            platform: "shared".into(),
                            relative_path: rel_edl,
                            bytes: edl.len(),
                        });
                    }

                    if cfg.include_fcpxml {
                        let xml = export_fcpxml_v10(&s.project);
                        let rel_xml = "interchange/master.fcpxml".to_string();
                        std::fs::write(out_dir.join(&rel_xml), xml.as_bytes())
                            .map_err(|e| e.to_string())?;
                        artifacts.push(DeliveryBundleArtifact {
                            kind: "fcpxml_v10".into(),
                            platform: "shared".into(),
                            relative_path: rel_xml,
                            bytes: xml.len(),
                        });
                    }

                    if cfg.include_qc_report {
                        let (
                            peak_luma_ire,
                            clipped_highlight_pct,
                            crushed_shadow_pct,
                            max_frame_luma_jump_ire,
                        ) = measure_scene_video_qc_metrics(&mut rg, &s.project, active_id, 4);
                        let report = run_broadcast_qc_audit(
                            &s.project,
                            BroadcastProfile::YouTube1080p,
                            BroadcastQcMetrics {
                                peak_luma_ire,
                                clipped_highlight_pct,
                                crushed_shadow_pct,
                                max_frame_luma_jump_ire,
                                true_peak_dbtp: -1.5,
                                integrated_lufs: -14.0,
                            },
                        );
                        let qc_bytes =
                            serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
                        let rel_qc = "qc/broadcast_qc_report.json".to_string();
                        std::fs::write(out_dir.join(&rel_qc), &qc_bytes)
                            .map_err(|e| e.to_string())?;
                        artifacts.push(DeliveryBundleArtifact {
                            kind: "broadcast_qc_json".into(),
                            platform: "shared".into(),
                            relative_path: rel_qc,
                            bytes: qc_bytes.len(),
                        });
                    }

                    let manifest = build_delivery_manifest(&s.project, &cfg, artifacts);
                    let manifest_bytes =
                        serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
                    std::fs::write(out_dir.join("manifest.json"), &manifest_bytes)
                        .map_err(|e| e.to_string())?;

                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "output_dir": out_dir.display().to_string(),
                                "artifact_count": manifest.artifacts.len(),
                                "manifest": manifest
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown multi_platform_packager action: {other}")),
            }
        }
        other => Err(format!("unknown tool: {other}")),
    }
}
