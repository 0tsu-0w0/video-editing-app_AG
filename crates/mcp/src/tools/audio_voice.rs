//! v7.0 #46: 内蔵 MCP ドメインモジュール — Audio Post, Spatial Mastering, BGM, Dubbing & Podcast

#[allow(unused_imports)]
use super::*;

pub(super) const DOMAIN_ID: &str = "audio_voice";
pub(super) const DOMAIN_LABEL: &str = "Audio Post, Spatial Mastering, BGM, Dubbing & Podcast";

pub(super) fn definitions() -> Vec<Value> {
    let mut out = Vec::new();
    if let Value::Array(items) = json!([
        {
            "name": "import_tts_file_pair",
            "description": "voice.wav と同名の .txt / .lab を自動取込し、音声・字幕・音素タイミングを一括配置する",
            "inputSchema": {
                "type": "object",
                "required": ["wav_path"],
                "properties": {
                    "wav_path": { "type": "string" },
                    "audio_layer_index": { "type": "integer" },
                    "subtitle_layer_index": { "type": "integer" },
                    "start_sec": { "type": "number" }
                }
            }
        },
        {
            "name": "add_audio_effect",
            "description": "クリップまたはシーンマスターへ音声DSPエフェクトを追加、または目標ラウドネス(LUFS)を設定する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "effect_type": { "type": "string" },
                    "params": { "type": "object" },
                    "target_lufs": { "type": "number" }
                }
            }
        },
        {
            "name": "set_audio_ducking",
            "description": "発話レイヤー（サイドチェイン）に応じて BGM レイヤーの音量を自動減衰させるダッキングを設定する（Undo対応）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean" },
                    "voice_layer_index": { "type": "integer" },
                    "bgm_layer_index": { "type": "integer" },
                    "threshold_rms": { "type": "number" },
                    "reduction_db": { "type": "number" },
                    "attack_sec": { "type": "number" },
                    "release_sec": { "type": "number" }
                }
            }
        },
        {
            "name": "detect_beats",
            "description": "BGM/音声から BPM と拍・小節位置を自動検出し、任意でビートマーカー一括配置や選択クリップの音ハメ自動カットを行う（Undo対応）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "min_bpm": { "type": "number", "default": 70.0 },
                    "max_bpm": { "type": "number", "default": 180.0 },
                    "sensitivity": { "type": "number", "default": 1.35 },
                    "beats_per_bar": { "type": "integer", "default": 4 },
                    "add_markers": { "type": "boolean", "default": false },
                    "downbeats_only": { "type": "boolean", "default": true },
                    "cut_item_ids": { "type": "array", "items": { "type": "string" } }
                }
            }
        },
        {
            "name": "spatial_and_master_audio",
            "description": "v2.1 #27: 3Dバイノーラル空間オーディオパンナー（ITD+ILD+頭部シャドウ）・3バンドマルチバンドコンプレッサー・4x True-Peakブリックウォールリミッター・dBTP/LUFS計測を操作する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["add_spatial_3d", "add_multiband_comp", "add_true_peak_limiter", "measure_master_true_peak"]
                    },
                    "id": { "type": "string" },
                    "azimuth_deg": { "type": "number" },
                    "elevation_deg": { "type": "number" },
                    "distance": { "type": "number" },
                    "room_mix": { "type": "number" },
                    "low_thresh_db": { "type": "number" },
                    "mid_thresh_db": { "type": "number" },
                    "high_thresh_db": { "type": "number" },
                    "ratio": { "type": "number" },
                    "makeup_db": { "type": "number" },
                    "ceiling_dbtp": { "type": "number" },
                    "release_ms": { "type": "number" },
                    "duration_sec": { "type": "number" }
                }
            }
        },
        {
            "name": "offline_ai_dubbing_lipsync",
            "description": "v3.0 #31: 完全オフライン（クラウド送信ゼロ）の多言語 AI 吹替音声合成（3フォルマント共振 48kHz WAV 生成）・音素アライメント・立ち絵/映像リップシンク自動同期を 1-Undo で実行する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list_voices", "align_phonemes", "generate_locale_dubbing"]
                    },
                    "text": { "type": "string" },
                    "duration_sec": { "type": "number" },
                    "locale": { "type": "string" },
                    "voice": {
                        "type": "string",
                        "enum": ["jp_female", "jp_male", "en_female", "en_male", "narrator_warm"]
                    },
                    "audio_layer_index": { "type": "integer" },
                    "csv": { "type": "string" },
                    "translations": { "type": "object" },
                    "apply": { "type": "boolean" }
                }
            }
        },
        {
            "name": "offline_bgm_composer",
            "description": "v5.0 #38: 完全オフライン・著作権フリーの自動 BGM コンポーザー（6 ジャンル・コード進行・ドラム/ベース/パッド/メロディ 48kHz ステレオ WAV 合成＆小節マーカー自動配置）を実行する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["preview_arrangement", "compose_and_place_bgm"]
                    },
                    "genre": {
                        "type": "string",
                        "enum": ["lofi_chill", "upbeat_pop", "cinematic_epic", "cyber_synthwave", "acoustic_explainer", "suspense_pulse"]
                    },
                    "bpm": { "type": "number" },
                    "key_semitone": { "type": "integer" },
                    "duration_sec": { "type": "number" },
                    "intensity": { "type": "number" },
                    "seed": { "type": "integer" },
                    "start_sec": { "type": "number" },
                    "audio_layer_index": { "type": "integer" },
                    "add_bar_markers": { "type": "boolean" },
                    "apply": { "type": "boolean" }
                }
            }
        },
        {
            "name": "podcast_auto_director",
            "description": "v6.0 #43: マルチトラック・ポッドキャスト＆コラボ実況オート編集（複数話者ダイアリゼーション・アクティブ話者カメラ自動切替キーフレーム＆マーカー・-16 LUFS 均一化レベラー）を実行する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["analyze_and_direct", "level_wav"]
                    },
                    "speakers": { "type": "array" },
                    "min_turn_sec": { "type": "number" },
                    "silence_gate_db": { "type": "number" },
                    "target_lufs": { "type": "number" },
                    "apply_leveler_faders": { "type": "boolean" },
                    "apply": { "type": "boolean" },
                    "input_wav_path": { "type": "string" },
                    "output_wav_path": { "type": "string" }
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
        "import_tts_file_pair"
            | "add_audio_effect"
            | "set_audio_ducking"
            | "detect_beats"
            | "spatial_and_master_audio"
            | "offline_ai_dubbing_lipsync"
            | "offline_bgm_composer"
            | "podcast_auto_director"
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
        "import_tts_file_pair" => {
            let wav_str = args
                .get("wav_path")
                .and_then(Value::as_str)
                .ok_or("missing wav_path")?;
            let wav_path = PathBuf::from(wav_str);
            let ext_ok = wav_path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| ALLOWED_TTS_AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()));
            if !ext_ok || !wav_path.is_file() {
                return Err(format!(
                    "wav_path must point to an existing audio file ({ALLOWED_TTS_AUDIO_EXTS:?})"
                ));
            }
            let audio_layer = args
                .get("audio_layer_index")
                .and_then(Value::as_u64)
                .unwrap_or(3) as usize;
            let sub_layer = args
                .get("subtitle_layer_index")
                .and_then(Value::as_u64)
                .unwrap_or(1) as usize;
            let start = RationalTime::from_seconds(finite_clamped(
                args.get("start_sec"),
                guard.playhead.as_seconds(),
                0.0,
                86_400.0,
            ));

            let bundle = import_tts_file_pair(&wav_path).map_err(|e| e.to_string())?;
            let asset_id = Uuid::new_v4();
            guard.project.assets.push(AssetRef {
                id: asset_id,
                name: wav_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "voice.wav".into()),
                kind: MediaKind::Audio,
                relative_path: None,
                fallback_absolute_path: Some(wav_path),
                collected: false,
            });

            let mut audio_item = TimelineItem::new(
                "TTS音声",
                start,
                bundle.duration,
                ItemKind::Audio {
                    asset_id,
                    source_offset: RationalTime::ZERO,
                    volume: 1.0,
                },
            );
            audio_item.phoneme_segments = bundle.phonemes.clone();
            let sub_item = TimelineItem::new(
                "TTS字幕",
                start,
                bundle.duration,
                ItemKind::Text {
                    content: bundle.subtitle_text.clone(),
                    font_family: "sans-serif".to_string(),
                    font_size: 46.0,
                    color: RgbaColor::WHITE,
                    animator: TextAnimator::default(),
                },
            );

            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::Batch {
                        label: "import_tts_file_pair".into(),
                        commands: vec![
                            EditCommand::AddObject {
                                layer_index: audio_layer,
                                item: audio_item,
                            },
                            EditCommand::AddObject {
                                layer_index: sub_layer,
                                item: sub_item,
                            },
                        ],
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({
                        "subtitle": bundle.subtitle_text,
                        "phoneme_count": bundle.phonemes.len(),
                        "duration_sec": bundle.duration.as_seconds()
                    }).to_string()
                }]
            }))
        }
        "add_audio_effect" => {
            let item_id = match args.get("id").and_then(Value::as_str) {
                Some(s) if !s.is_empty() => {
                    Some(Uuid::parse_str(s).map_err(|e| format!("invalid uuid: {e}"))?)
                }
                _ => None,
            };
            let mut cmds = Vec::new();
            if let Some(eff_type) = args.get("effect_type").and_then(Value::as_str) {
                let mut effect = AudioEffectNode::with_defaults(eff_type);
                for (k, v) in parse_params_map(args.get("params")) {
                    effect.params.insert(k, v);
                }
                cmds.push(EditCommand::AddAudioEffect { item_id, effect });
            }
            if let Some(lufs) = args.get("target_lufs").and_then(Value::as_f64)
                && lufs.is_finite()
            {
                cmds.push(EditCommand::SetTargetLufs {
                    target_lufs: Some(lufs.clamp(-36.0, -3.0) as f32),
                });
            }
            if cmds.is_empty() {
                return Err("provide 'effect_type' or 'target_lufs'".into());
            }
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::Batch {
                        label: "mcp_audio_effect".into(),
                        commands: cmds,
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({ "content": [{ "type": "text", "text": json!({"ok": true}).to_string() }] }))
        }
        "set_audio_ducking" => {
            let mut ducking = guard
                .project
                .active_scene()
                .map(|s| s.audio_ducking)
                .unwrap_or_default();
            let max_layer = guard
                .project
                .active_scene()
                .map(|s| s.layers.len().saturating_sub(1))
                .unwrap_or(0);
            if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                ducking.enabled = en;
            }
            if let Some(vl) = args.get("voice_layer_index").and_then(Value::as_u64) {
                ducking.voice_layer_index = (vl as usize).min(max_layer);
            }
            if let Some(bl) = args.get("bgm_layer_index").and_then(Value::as_u64) {
                ducking.bgm_layer_index = (bl as usize).min(max_layer);
            }
            if args.get("threshold_rms").is_some() {
                ducking.threshold_rms = finite_clamped(
                    args.get("threshold_rms"),
                    f64::from(ducking.threshold_rms),
                    0.001,
                    0.9,
                ) as f32;
            }
            if args.get("reduction_db").is_some() {
                ducking.reduction_db = finite_clamped(
                    args.get("reduction_db"),
                    f64::from(ducking.reduction_db),
                    -48.0,
                    0.0,
                ) as f32;
            }
            if args.get("attack_sec").is_some() {
                ducking.attack_sec = finite_clamped(
                    args.get("attack_sec"),
                    f64::from(ducking.attack_sec),
                    0.002,
                    2.0,
                ) as f32;
            }
            if args.get("release_sec").is_some() {
                ducking.release_sec = finite_clamped(
                    args.get("release_sec"),
                    f64::from(ducking.release_sec),
                    0.01,
                    5.0,
                ) as f32;
            }
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetAudioDucking { ducking },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string(&ducking).unwrap_or_default()
                }]
            }))
        }
        "detect_beats" => {
            let cfg = BeatDetectionConfig {
                min_bpm: finite_clamped(args.get("min_bpm"), 70.0, 40.0, 220.0) as f32,
                max_bpm: finite_clamped(args.get("max_bpm"), 180.0, 50.0, 260.0) as f32,
                sensitivity: finite_clamped(args.get("sensitivity"), 1.35, 0.8, 3.5) as f32,
                beats_per_bar: (args
                    .get("beats_per_bar")
                    .and_then(Value::as_u64)
                    .unwrap_or(4) as usize)
                    .clamp(1, 12),
            };
            let scene_id = guard.project.active_scene_id;
            let total_dur = guard
                .project
                .active_scene()
                .map(|s| s.total_duration())
                .ok_or("no active scene")?;
            let sr = guard.project.settings.audio_sample_rate.max(8_000);
            let graph =
                AudioGraph::from_project_scene(&guard.project, scene_id, guard.folder.as_ref(), sr);
            let capped = total_dur.min(RationalTime::from_seconds(300.0));
            let pcm = if graph.clips.is_empty() {
                Vec::new()
            } else {
                graph.render_range(RationalTime::ZERO, capped)
            };
            let result = detect_audio_beats(&pcm, sr, &cfg);

            let mut markers_added = 0usize;
            if args
                .get("add_markers")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && !result.beats.is_empty()
            {
                let downbeats_only = args
                    .get("downbeats_only")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let pairs: Vec<(f64, bool)> = result
                    .beats
                    .iter()
                    .map(|b| (b.time_sec, b.is_downbeat))
                    .collect();
                if let Some(cmd) = build_beat_markers_command(&pairs, downbeats_only) {
                    if let EditCommand::Batch { ref commands, .. } = cmd {
                        markers_added = commands.len();
                    }
                    let sess = &mut *guard;
                    sess.bus
                        .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                        .map_err(|e| e.to_string())?;
                }
            }

            let mut cuts_performed = 0usize;
            if let Some(arr) = args.get("cut_item_ids").and_then(Value::as_array)
                && !result.beats.is_empty()
            {
                let ids: Vec<Uuid> = arr
                    .iter()
                    .filter_map(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                    .collect();
                let beat_secs: Vec<f64> = result.beats.iter().map(|b| b.time_sec).collect();
                let scene_clone = guard
                    .project
                    .active_scene()
                    .cloned()
                    .ok_or("no active scene")?;
                if let Some((cuts, cmd)) = build_beat_cut_commands(&scene_clone, &ids, &beat_secs) {
                    let sess = &mut *guard;
                    sess.bus
                        .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                        .map_err(|e| e.to_string())?;
                    cuts_performed = cuts;
                }
            }

            let beats_json: Vec<Value> = result
                .beats
                .iter()
                .map(|b| {
                    json!({
                        "time_sec": b.time_sec,
                        "strength": b.strength,
                        "is_downbeat": b.is_downbeat,
                    })
                })
                .collect();
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({
                        "bpm": result.bpm,
                        "confidence": result.confidence,
                        "beat_count": result.beats.len(),
                        "markers_added": markers_added,
                        "cuts_performed": cuts_performed,
                        "beats": beats_json,
                    }).to_string()
                }]
            }))
        }
        "spatial_and_master_audio" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let opt_item_id = if args.get("id").is_some() {
                Some(parse_uuid(args, "id")?)
            } else {
                None
            };

            match action {
                "add_spatial_3d" => {
                    let mut node = AudioEffectNode::with_defaults("spatial_3d");
                    node.params.insert(
                        "azimuth_deg".into(),
                        finite_clamped(args.get("azimuth_deg"), 0.0, -180.0, 180.0) as f32,
                    );
                    node.params.insert(
                        "elevation_deg".into(),
                        finite_clamped(args.get("elevation_deg"), 0.0, -90.0, 90.0) as f32,
                    );
                    node.params.insert(
                        "distance".into(),
                        finite_clamped(args.get("distance"), 1.5, 0.3, 25.0) as f32,
                    );
                    node.params.insert(
                        "room_mix".into(),
                        finite_clamped(args.get("room_mix"), 0.12, 0.0, 0.6) as f32,
                    );
                    let effect_id = node.id;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddAudioEffect {
                                item_id: opt_item_id,
                                effect: node.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "effect_id": effect_id, "target": opt_item_id, "effect": node }).to_string() }]
                    }))
                }
                "add_multiband_comp" => {
                    let mut node = AudioEffectNode::with_defaults("multiband_comp");
                    node.params.insert(
                        "low_thresh_db".into(),
                        finite_clamped(args.get("low_thresh_db"), -18.0, -48.0, 0.0) as f32,
                    );
                    node.params.insert(
                        "mid_thresh_db".into(),
                        finite_clamped(args.get("mid_thresh_db"), -16.0, -48.0, 0.0) as f32,
                    );
                    node.params.insert(
                        "high_thresh_db".into(),
                        finite_clamped(args.get("high_thresh_db"), -20.0, -48.0, 0.0) as f32,
                    );
                    node.params.insert(
                        "ratio".into(),
                        finite_clamped(args.get("ratio"), 3.0, 1.0, 20.0) as f32,
                    );
                    node.params.insert(
                        "makeup_db".into(),
                        finite_clamped(args.get("makeup_db"), 1.5, -12.0, 24.0) as f32,
                    );
                    let effect_id = node.id;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddAudioEffect {
                                item_id: opt_item_id,
                                effect: node.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "effect_id": effect_id, "target": opt_item_id, "effect": node }).to_string() }]
                    }))
                }
                "add_true_peak_limiter" => {
                    let mut node = AudioEffectNode::with_defaults("true_peak_limiter");
                    node.params.insert(
                        "ceiling_dbtp".into(),
                        finite_clamped(args.get("ceiling_dbtp"), -1.0, -12.0, -0.1) as f32,
                    );
                    node.params.insert(
                        "release_ms".into(),
                        finite_clamped(args.get("release_ms"), 60.0, 10.0, 500.0) as f32,
                    );
                    let effect_id = node.id;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddAudioEffect {
                                item_id: opt_item_id,
                                effect: node.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "effect_id": effect_id, "target": opt_item_id, "effect": node }).to_string() }]
                    }))
                }
                "measure_master_true_peak" => {
                    let scene_id = s.project.active_scene_id;
                    let total_dur = s
                        .project
                        .active_scene()
                        .map(|sc| sc.total_duration().as_seconds().max(0.5))
                        .ok_or_else(|| "no active scene".to_string())?;
                    let dur_sec =
                        finite_clamped(args.get("duration_sec"), total_dur.min(15.0), 0.1, 60.0);
                    let sr = s.project.settings.audio_sample_rate.max(8_000);
                    let graph =
                        AudioGraph::from_project_scene(&s.project, scene_id, s.folder.as_ref(), sr);
                    let pcm =
                        graph.render_range(RationalTime::ZERO, RationalTime::from_seconds(dur_sec));
                    let true_peak_dbtp = measure_true_peak_dbtp(&pcm, graph.channels);
                    let integrated_lufs = measure_integrated_lufs(&pcm, sr);
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "duration_sec": dur_sec,
                                "sample_rate": sr,
                                "true_peak_dbtp": true_peak_dbtp,
                                "integrated_lufs": integrated_lufs,
                                "broadcast_safe": true_peak_dbtp <= -1.0
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown spatial_and_master_audio action: {other}")),
            }
        }
        "offline_ai_dubbing_lipsync" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            match action {
                "list_voices" => {
                    let voices: Vec<Value> = DubbingVoicePreset::ALL
                        .iter()
                        .map(|v| {
                            json!({
                                "id": v.id(),
                                "name": v.display_name(),
                                "base_pitch_hz": v.base_pitch_hz()
                            })
                        })
                        .collect();
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "offline_only": true, "voices": voices }).to_string() }]
                    }))
                }
                "align_phonemes" => {
                    let text = args
                        .get("text")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'text'".to_string())?;
                    let dur_sec = finite_clamped(args.get("duration_sec"), 2.0, 0.15, 60.0);
                    let segs = align_text_to_phoneme_segments(
                        text,
                        RationalTime::ZERO,
                        RationalTime::from_seconds(dur_sec),
                    );
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "phoneme_count": segs.len(), "phonemes": segs }).to_string() }]
                    }))
                }
                "generate_locale_dubbing" => {
                    let locale = args.get("locale").and_then(Value::as_str).unwrap_or("en");
                    let voice = DubbingVoicePreset::from_id(
                        args.get("voice").and_then(Value::as_str).unwrap_or(
                            if locale.starts_with("en") {
                                "en_female"
                            } else {
                                "jp_female"
                            },
                        ),
                    );
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
                    let max_layer = s
                        .project
                        .active_scene()
                        .map(|sc| sc.layers.len().saturating_sub(1))
                        .ok_or_else(|| "no active scene".to_string())?;
                    let audio_layer_index = (args
                        .get("audio_layer_index")
                        .and_then(Value::as_u64)
                        .unwrap_or(max_layer as u64)
                        as usize)
                        .min(max_layer);

                    let targets: Vec<(Uuid, String, RationalTime, RationalTime)> =
                        if let Some(csv) = args.get("csv").and_then(Value::as_str) {
                            let tbl = import_localization_csv(csv);
                            let sc = s
                                .project
                                .active_scene()
                                .ok_or_else(|| "no active scene".to_string())?;
                            ve_audio::collect_dubbing_targets_from_table(sc, &tbl, locale)
                        } else if let Some(map) =
                            args.get("translations").and_then(Value::as_object)
                        {
                            let sc = s
                                .project
                                .active_scene()
                                .ok_or_else(|| "no active scene".to_string())?;
                            let mut out = Vec::new();
                            for (k, v) in map {
                                if let (Ok(id), Some(txt)) = (Uuid::parse_str(k), v.as_str())
                                    && let Some(item) = sc
                                        .layers
                                        .iter()
                                        .flat_map(|l| l.items.iter())
                                        .find(|i| i.id == id)
                                {
                                    out.push((id, txt.to_string(), item.start, item.duration));
                                }
                            }
                            out
                        } else {
                            let sc = s
                                .project
                                .active_scene()
                                .ok_or_else(|| "no active scene".to_string())?;
                            sc.layers
                                .iter()
                                .flat_map(|l| l.items.iter())
                                .filter_map(|i| {
                                    if let ItemKind::Text { content, .. } = &i.kind {
                                        Some((i.id, content.clone(), i.start, i.duration))
                                    } else {
                                        None
                                    }
                                })
                                .collect()
                        };

                    if targets.is_empty() {
                        return Err("吹替対象の字幕アイテムがありません".into());
                    }

                    let out_dir = s
                        .folder
                        .as_ref()
                        .map(|f| f.assets_dir().join("dubbing"))
                        .unwrap_or_else(|| std::env::temp_dir().join("ve_dubbing"));
                    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
                    let sr = s.project.settings.audio_sample_rate.max(8_000);

                    let mut generated_clips = Vec::new();
                    let mut wav_paths = Vec::new();
                    for (idx, (sub_id, txt, start, dur)) in targets.into_iter().enumerate() {
                        let phonemes = align_text_to_phoneme_segments(&txt, start, dur);
                        let pcm = synthesize_offline_dubbing_pcm(&phonemes, start, dur, sr, voice);
                        let wav_name = format!("dub_{locale}_{idx}.wav");
                        let wav_path = out_dir.join(&wav_name);
                        write_wav_pcm16(&wav_path, &pcm, sr, 2).map_err(|e| e.to_string())?;
                        wav_paths.push(wav_path.display().to_string());

                        let asset_id = Uuid::new_v4();
                        s.project.assets.push(AssetRef {
                            id: asset_id,
                            name: wav_name.clone(),
                            relative_path: Some(PathBuf::from("assets/dubbing").join(&wav_name)),
                            fallback_absolute_path: Some(wav_path),
                            collected: true,
                            kind: MediaKind::Audio,
                        });
                        generated_clips.push(GeneratedDubClip {
                            subtitle_item_id: sub_id,
                            translated_text: txt,
                            start,
                            duration: dur,
                            audio_asset_id: asset_id,
                            clip_name: format!("Dub[{locale}] #{idx}"),
                            phonemes,
                        });
                    }

                    let dub_count = generated_clips.len();
                    let cmds = {
                        let sc = s
                            .project
                            .active_scene()
                            .ok_or_else(|| "no active scene".to_string())?;
                        build_dubbing_timeline_commands(sc, audio_layer_index, &generated_clips)
                    };
                    let applied = if apply && !cmds.is_empty() {
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::Batch {
                                    label: format!(
                                        "オフライン多言語吹替＆リップシンク [{locale}] ({dub_count}件)"
                                    ),
                                    commands: cmds,
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                        true
                    } else {
                        false
                    };

                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "offline_only": true,
                                "locale": locale,
                                "voice": voice.id(),
                                "generated_clips": dub_count,
                                "wav_paths": wav_paths,
                                "applied": applied
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!(
                    "unknown offline_ai_dubbing_lipsync action: {other}"
                )),
            }
        }
        "offline_bgm_composer" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let genre = BgmGenre::from_id(
                args.get("genre")
                    .and_then(Value::as_str)
                    .unwrap_or("lofi_chill"),
            );
            let cfg = BgmComposeConfig {
                genre,
                bpm: finite_clamped(args.get("bpm"), f64::from(genre.default_bpm()), 55.0, 200.0)
                    as f32,
                key_semitone: args
                    .get("key_semitone")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    .rem_euclid(12) as i32,
                duration_sec: finite_clamped(args.get("duration_sec"), 8.0, 1.0, 120.0) as f32,
                intensity: finite_clamped(args.get("intensity"), 1.0, 0.3, 1.5) as f32,
                seed: args.get("seed").and_then(Value::as_u64).unwrap_or(50_2026),
            };
            let sr = s.project.settings.audio_sample_rate.max(8_000);
            let composed = compose_bgm_track(&cfg, sr);

            match action {
                "preview_arrangement" => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": json!({
                            "genre": genre.id(),
                            "bpm": composed.bpm,
                            "chord_progression": composed.chord_progression,
                            "beat_count": composed.beat_times_sec.len(),
                            "bar_count": composed.bar_times_sec.len(),
                            "bar_times_sec": composed.bar_times_sec
                        }).to_string()
                    }]
                })),
                "compose_and_place_bgm" => {
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
                    let add_bar_markers = args
                        .get("add_bar_markers")
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    let start = RationalTime::from_seconds(finite_clamped(
                        args.get("start_sec"),
                        0.0,
                        0.0,
                        36000.0,
                    ));
                    let out_dir = s
                        .folder
                        .as_ref()
                        .map(|f| f.assets_dir().join("bgm"))
                        .unwrap_or_else(|| std::env::temp_dir().join("ve_bgm"));
                    let wav_name = format!("bgm_{}_{}.wav", genre.id(), cfg.seed);
                    let wav_path = out_dir.join(&wav_name);
                    write_stereo_wav_file(&wav_path, &composed.pcm, composed.sample_rate)?;

                    let asset_id = Uuid::new_v4();
                    s.project.assets.push(AssetRef {
                        id: asset_id,
                        name: wav_name.clone(),
                        relative_path: Some(PathBuf::from("assets/bgm").join(&wav_name)),
                        fallback_absolute_path: Some(wav_path.clone()),
                        collected: true,
                        kind: MediaKind::Audio,
                    });

                    let mut clip_id: Option<String> = None;
                    if apply {
                        let layer_count = s
                            .project
                            .active_scene()
                            .map(|sc| sc.layers.len())
                            .ok_or_else(|| "no active scene".to_string())?;
                        let audio_layer = args
                            .get("audio_layer_index")
                            .and_then(Value::as_u64)
                            .map(|v| (v as usize).min(layer_count.saturating_sub(1)))
                            .unwrap_or_else(|| layer_count.saturating_sub(1));
                        let clip = TimelineItem::new(
                            format!("BGM: {} ({:.0}BPM)", genre.id(), composed.bpm),
                            start,
                            RationalTime::from_seconds(f64::from(cfg.duration_sec)),
                            ItemKind::Audio {
                                asset_id,
                                source_offset: RationalTime::ZERO,
                                volume: 0.72,
                            },
                        );
                        clip_id = Some(clip.id.to_string());
                        let mut cmds = vec![EditCommand::AddObject {
                            layer_index: audio_layer,
                            item: clip,
                        }];
                        if add_bar_markers {
                            for (idx, &bt) in composed.bar_times_sec.iter().enumerate() {
                                cmds.push(EditCommand::AddMarker {
                                    marker: TimelineMarker::new(
                                        start + RationalTime::from_seconds(bt),
                                        format!("Bar {}", idx + 1),
                                    ),
                                });
                            }
                        }
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::Batch {
                                    label: format!("自動 BGM 作曲＆配置 ({})", genre.id()),
                                    commands: cmds,
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                    }

                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "genre": genre.id(),
                                "bpm": composed.bpm,
                                "chord_progression": composed.chord_progression,
                                "bar_markers_added": if apply && add_bar_markers { composed.bar_times_sec.len() } else { 0 },
                                "wav_path": wav_path.display().to_string(),
                                "asset_id": asset_id.to_string(),
                                "item_id": clip_id,
                                "applied": apply
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown offline_bgm_composer action: {other}")),
            }
        }
        "podcast_auto_director" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("analyze_and_direct");
            let target_lufs = finite_clamped(args.get("target_lufs"), -16.0, -30.0, -8.0) as f32;

            match action {
                "level_wav" => {
                    let in_path = args
                        .get("input_wav_path")
                        .and_then(Value::as_str)
                        .ok_or("missing 'input_wav_path'")?;
                    let out_path = args
                        .get("output_wav_path")
                        .and_then(Value::as_str)
                        .ok_or("missing 'output_wav_path'")?;
                    let buf = decode_audio_file(&PathBuf::from(in_path))
                        .map_err(|e| format!("decode failed: {e}"))?;
                    let sr = buf.sample_rate.max(8_000);
                    let mut stereo = Vec::with_capacity(buf.frame_count() * 2);
                    if buf.channels <= 1 {
                        for &v in &buf.samples {
                            stereo.push(v);
                            stereo.push(v);
                        }
                    } else {
                        for f in 0..buf.frame_count() {
                            let base = f * buf.channels;
                            stereo.push(buf.samples[base]);
                            stereo.push(buf.samples[base + 1]);
                        }
                    }
                    let before_lufs = measure_integrated_lufs(&stereo, sr);
                    apply_podcast_voice_leveler(&mut stereo, sr, target_lufs);
                    let after_lufs = measure_integrated_lufs(&stereo, sr);
                    let out_pb = PathBuf::from(out_path);
                    if let Some(parent) = out_pb.parent()
                        && !parent.as_os_str().is_empty()
                    {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    write_stereo_wav_file(&out_pb, &stereo, sr).map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "before_lufs": before_lufs,
                                "after_lufs": after_lufs,
                                "target_lufs": target_lufs,
                                "saved_path": out_pb.display().to_string()
                            }).to_string()
                        }]
                    }))
                }
                "analyze_and_direct" => {
                    let min_turn_sec = finite_clamped(args.get("min_turn_sec"), 0.4, 0.1, 5.0);
                    let silence_gate_db =
                        finite_clamped(args.get("silence_gate_db"), -42.0, -70.0, -10.0) as f32;
                    let apply_leveler_faders = args
                        .get("apply_leveler_faders")
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);

                    let scene = s.project.active_scene().ok_or("no active scene")?.clone();
                    let sr = s.project.settings.audio_sample_rate.max(8_000);
                    let scene_dur_sec = scene.total_duration().as_seconds().clamp(1.0, 120.0);

                    let mut specs: Vec<(SpeakerTrackSpec, Option<String>)> = Vec::new();
                    if let Some(arr) = args.get("speakers").and_then(Value::as_array) {
                        for (idx, sp) in arr.iter().enumerate() {
                            let name = sp
                                .get("name")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                                .unwrap_or_else(|| format!("Speaker {}", idx + 1));
                            let audio_layer_index =
                                sp.get("audio_layer_index")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(idx as u64) as usize;
                            let camera_layer_index = sp
                                .get("camera_layer_index")
                                .and_then(Value::as_u64)
                                .map(|v| v as usize);
                            let wav_path = sp
                                .get("wav_path")
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                            specs.push((
                                SpeakerTrackSpec {
                                    name,
                                    audio_layer_index,
                                    camera_layer_index,
                                },
                                wav_path,
                            ));
                        }
                    } else {
                        for (li, layer) in scene.layers.iter().enumerate() {
                            let has_audio = layer
                                .items
                                .iter()
                                .any(|it| matches!(it.kind, ItemKind::Audio { .. }));
                            if has_audio {
                                specs.push((
                                    SpeakerTrackSpec {
                                        name: layer.name.clone(),
                                        audio_layer_index: li,
                                        camera_layer_index: None,
                                    },
                                    None,
                                ));
                            }
                        }
                    }

                    let mut track_pcms: Vec<(String, Vec<f32>)> = Vec::with_capacity(specs.len());
                    for (sp_idx, (spec, wav_opt)) in specs.iter().enumerate() {
                        let loaded_pcm = wav_opt
                            .as_ref()
                            .and_then(|p| decode_audio_file(&PathBuf::from(p)).ok())
                            .map(|buf| {
                                let mut st = Vec::with_capacity(buf.frame_count() * 2);
                                if buf.channels <= 1 {
                                    for &v in &buf.samples {
                                        st.push(v);
                                        st.push(v);
                                    }
                                } else {
                                    for f in 0..buf.frame_count() {
                                        let base = f * buf.channels;
                                        st.push(buf.samples[base]);
                                        st.push(buf.samples[base + 1]);
                                    }
                                }
                                st
                            });

                        let pcm = if let Some(st) = loaded_pcm {
                            st
                        } else {
                            // タイムライン上の音声クリップ区間から決定論的な発話活性波形を生成
                            let frames = ((scene_dur_sec * f64::from(sr)).round() as usize)
                                .clamp(sr as usize, sr as usize * 120);
                            let mut st = vec![0.0f32; frames * 2];
                            let freq = 180.0 + (sp_idx as f32) * 45.0;
                            let amp = (0.18 + (sp_idx as f32) * 0.08).min(0.45);
                            if let Some(layer) = scene.layers.get(spec.audio_layer_index) {
                                for it in &layer.items {
                                    let f0 = ((it.start.as_seconds() * f64::from(sr)).round()
                                        as usize)
                                        .min(frames);
                                    let f1 = ((it.end().as_seconds() * f64::from(sr)).round()
                                        as usize)
                                        .min(frames);
                                    for fi in f0..f1 {
                                        let t = fi as f32 / sr as f32;
                                        let v = (std::f32::consts::TAU * freq * t).sin() * amp;
                                        st[fi * 2] = v;
                                        st[fi * 2 + 1] = v;
                                    }
                                }
                            }
                            st
                        };
                        track_pcms.push((spec.name.clone(), pcm));
                    }

                    let analysis = analyze_podcast_speakers(
                        &track_pcms,
                        sr,
                        min_turn_sec,
                        silence_gate_db,
                        target_lufs,
                    );
                    let only_specs: Vec<SpeakerTrackSpec> =
                        specs.iter().map(|(sp, _)| sp.clone()).collect();
                    let cmds = build_podcast_auto_switch_commands(
                        &scene,
                        &only_specs,
                        &analysis,
                        apply_leveler_faders,
                    );
                    let command_count = cmds.len();
                    if apply && !cmds.is_empty() {
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::Batch {
                                    label: "Podcast Auto-Director".into(),
                                    commands: cmds,
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                    }
                    let turns_json: Vec<Value> = analysis
                        .turns
                        .iter()
                        .map(|t| {
                            json!({
                                "speaker_idx": t.speaker_idx,
                                "speaker_name": t.speaker_name,
                                "start_sec": t.start_sec,
                                "end_sec": t.end_sec,
                                "avg_rms_db": t.avg_rms_db
                            })
                        })
                        .collect();
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "applied": apply && command_count > 0,
                                "turn_count": turns_json.len(),
                                "command_count": command_count,
                                "target_lufs": analysis.target_lufs,
                                "speaker_lufs": analysis.speaker_lufs,
                                "recommended_gains_db": analysis.recommended_gains_db,
                                "turns": turns_json
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown podcast_auto_director action: {other}")),
            }
        }
        other => Err(format!("unknown tool: {other}")),
    }
}
