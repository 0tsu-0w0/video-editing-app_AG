//! v7.0 #46: 内蔵 MCP ドメインモジュール — Local AI Vision, Director Recipes, Git Diff/Review & CRDT

#[allow(unused_imports)]
use super::*;

pub(super) const DOMAIN_ID: &str = "ai_collab";
pub(super) const DOMAIN_LABEL: &str = "Local AI Vision, Director Recipes, Git Diff/Review & CRDT";

pub(super) fn definitions() -> Vec<Value> {
    let mut out = Vec::new();
    if let Value::Array(items) = json!([
        {
            "name": "estimate_transcription_cost",
            "description": "外部AI文字起こし（Gemini / Claude）送信前の概算トークンとコストを安全見積りする",
            "inputSchema": {
                "type": "object",
                "required": ["provider", "duration_sec"],
                "properties": {
                    "provider": { "type": "string", "enum": ["gemini", "claude"] },
                    "duration_sec": { "type": "number" }
                }
            }
        },
        {
            "name": "refine_subtitles",
            "description": "タイムライン上の字幕を整形する（mode='local_wrap' で18文字自動改行、mode='claude' で Claude API 文脈補正・要 user_confirmed=true）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "mode": { "type": "string", "enum": ["local_wrap", "claude"], "default": "local_wrap" },
                    "max_chars_per_line": { "type": "integer", "default": 18 },
                    "user_confirmed": { "type": "boolean", "default": false }
                }
            }
        },
        {
            "name": "orchestrate_edit_recipe",
            "description": "v2.0 #23: マルチエージェント AI 編集ディレクター（YAML/JSON 編集レシピの記録・共有・人間承認ゲート付きステップ実行・render_frame_preview 視覚自己検査・Undo ロールバック）を操作する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list_recipes", "save_recipe", "import_recipe", "export_yaml", "record_from_history", "start_run", "step", "run_until_gate", "approve_gate", "reject_gate", "get_status"]
                    },
                    "recipe_name": { "type": "string" },
                    "recipe_text": { "type": "string" },
                    "recipe": { "type": "object" },
                    "variables": { "type": "object" },
                    "name": { "type": "string" },
                    "description": { "type": "string" },
                    "limit": { "type": "integer" },
                    "auto_run": { "type": "boolean" },
                    "continue_after": { "type": "boolean" },
                    "rollback_last": { "type": "boolean" },
                    "skip_step": { "type": "boolean" },
                    "reason": { "type": "string" }
                }
            }
        },
        {
            "name": "run_local_ai_vision",
            "description": "v2.0 #22: 完全オフライン（クラウド送信ゼロ）のローカル AI ビジョン（グリーンバック不要人物切り抜き・顔/被写体追尾 9:16 自動リフレーム・ローカル超解像・オプティカルフロースロー補間）を実行する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["status", "apply_segmentation", "auto_reframe", "apply_super_resolution", "set_optical_flow_slowmo"]
                    },
                    "id": { "type": "string" },
                    "sensitivity": { "type": "number" },
                    "edge_feather_px": { "type": "number" },
                    "skin_boost": { "type": "number" },
                    "despill_strength": { "type": "number" },
                    "invert": { "type": "boolean" },
                    "target_width": { "type": "integer" },
                    "target_height": { "type": "integer" },
                    "sample_interval_sec": { "type": "number" },
                    "smoothing_strength": { "type": "number" },
                    "apply": { "type": "boolean" },
                    "sharpen": { "type": "number" },
                    "denoise": { "type": "number" },
                    "enabled": { "type": "boolean" },
                    "speed": { "type": "number" }
                }
            }
        },
        {
            "name": "collab_diff_and_review",
            "description": "v2.0 #24: Git-Friendly NLE のスナップショット保存・視覚的タイムライン Diff・3-Way 意味的ブランチマージ・Frame.io/YouTube レビューコメント入出力＆マーカー自動変換を行う",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["save_snapshot", "list_snapshots", "diff_snapshots", "merge_three_way", "add_review_comment", "resolve_review_comment", "import_reviews", "export_reviews", "convert_reviews_to_markers"]
                    },
                    "name": { "type": "string" },
                    "base_snapshot": { "type": "string" },
                    "target_snapshot": { "type": "string" },
                    "theirs_snapshot": { "type": "string" },
                    "apply": { "type": "boolean" },
                    "resolutions": { "type": "object" },
                    "time_sec": { "type": "number" },
                    "author": { "type": "string" },
                    "body": { "type": "string" },
                    "comment_id": { "type": "string" },
                    "status": { "type": "string", "enum": ["open", "in_progress", "resolved"] },
                    "format": { "type": "string", "enum": ["frameio_csv", "json", "youtube_text"] },
                    "content": { "type": "string" },
                    "only_unresolved": { "type": "boolean" }
                }
            }
        },
        {
            "name": "crdt_realtime_collab",
            "description": "v3.0 #32: Lamport/Vector Clock ベースの Op-Based CRDT 同時共同編集（ローカル操作記録・ライブカーソルプレゼンス更新・デルタ JSON 書き出し・リモートデルタ冪等マージ＆ 1-Undo 反映）を操作する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["status", "set_peer_profile", "update_cursor", "record_marker_op", "record_text_op", "export_delta", "merge_remote_delta"]
                    },
                    "peer_id": { "type": "string" },
                    "display_name": { "type": "string" },
                    "color_hex": { "type": "string" },
                    "playhead_sec": { "type": "number" },
                    "selected_item": { "type": "string" },
                    "id": { "type": "string" },
                    "content": { "type": "string" },
                    "time_sec": { "type": "number" },
                    "label": { "type": "string" },
                    "delta_json": { "type": "string" },
                    "apply": { "type": "boolean" }
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
        "estimate_transcription_cost"
            | "refine_subtitles"
            | "run_local_ai_vision"
            | "collab_diff_and_review"
            | "crdt_realtime_collab"
    ) {
        return None;
    }
    Some(call_inner(guard, fps, name, args))
}

fn call_inner(
    guard: &mut crate::session::EditorSession,
    fps: FrameRate,
    name: &str,
    args: &Value,
) -> Result<Value, String> {
    match name {
        "estimate_transcription_cost" => {
            let provider = match args.get("provider").and_then(Value::as_str) {
                Some("claude") => ApiProvider::Claude,
                _ => ApiProvider::Gemini,
            };
            let duration_sec = finite_clamped(args.get("duration_sec"), 60.0, 0.0, 86_400.0);
            let est =
                CloudUploadGuard::default().estimate_audio_transcription(provider, duration_sec);
            let text = serde_json::to_string_pretty(&est).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "refine_subtitles" => {
            let mode = args
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("local_wrap");
            let max_chars = args
                .get("max_chars_per_line")
                .and_then(Value::as_u64)
                .unwrap_or(18)
                .clamp(8, 40) as usize;
            let user_confirmed = args
                .get("user_confirmed")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let scene = guard
                .project
                .active_scene()
                .ok_or("no active scene")?
                .clone();
            let mut text_items: Vec<(Uuid, RationalTime, RationalTime, String)> = Vec::new();
            for layer in &scene.layers {
                for item in &layer.items {
                    if let ItemKind::Text { content, .. } = &item.kind {
                        text_items.push((item.id, item.start, item.end(), content.clone()));
                    }
                }
            }
            text_items.sort_by_key(|(_, st, _, _)| *st);
            if text_items.is_empty() {
                return Ok(
                    json!({ "content": [{ "type": "text", "text": json!({"updated": 0}).to_string() }] }),
                );
            }

            let cmds: Vec<EditCommand> = match mode {
                "local_wrap" => text_items
                    .iter()
                    .filter_map(|(id, _, _, content)| {
                        let wrapped = wrap_subtitle_japanese(content, max_chars);
                        if wrapped != *content {
                            Some(EditCommand::SetTextContent {
                                item_id: *id,
                                content: wrapped,
                            })
                        } else {
                            None
                        }
                    })
                    .collect(),
                "claude" => {
                    let segments: Vec<SubtitleSegment> = text_items
                        .iter()
                        .map(|(_, st, en, txt)| SubtitleSegment {
                            start: *st,
                            end: *en,
                            text: txt.clone(),
                            phonemes: Vec::new(),
                        })
                        .collect();
                    let dur_sec = scene.total_duration().as_seconds().max(1.0);
                    let store = OsCredentialStore::new();
                    let refined = ClaudeRefiner::default()
                        .refine_subtitles(&store, &segments, dur_sec, user_confirmed)
                        .map_err(|e| e.to_string())?;
                    text_items
                        .iter()
                        .zip(refined.iter())
                        .filter_map(|((id, _, _, before), after)| {
                            if &after.text != before {
                                Some(EditCommand::SetTextContent {
                                    item_id: *id,
                                    content: after.text.clone(),
                                })
                            } else {
                                None
                            }
                        })
                        .collect()
                }
                other => return Err(format!("unknown refine mode: {other}")),
            };
            let updated = cmds.len();
            if updated > 0 {
                let sess = &mut *guard;
                sess.bus
                    .execute(
                        &mut sess.project,
                        EditCommand::Batch {
                            label: format!("mcp_refine_{mode}"),
                            commands: cmds,
                        },
                        CommandSource::Mcp,
                        None,
                    )
                    .map_err(|e| e.to_string())?;
            }
            Ok(json!({
                "content": [{ "type": "text", "text": json!({"mode": mode, "updated": updated}).to_string() }]
            }))
        }
        "run_local_ai_vision" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing string argument 'action'".to_string())?;
            let s = &mut *guard;
            match action {
                "status" => {
                    let models_dir = s.folder.as_ref().map(|f| f.models_dir());
                    let st = detect_local_vision_status(models_dir.as_deref());
                    let backend_name = match &st.active_backend {
                        LocalVisionBackend::PureRustBuiltIn => "pure_rust_builtin",
                        LocalVisionBackend::LocalOnnxCli { .. } => "local_onnx_cli",
                    };
                    let text = serde_json::to_string_pretty(&json!({
                        "backend": backend_name,
                        "local_only_guaranteed": st.local_only_guaranteed,
                        "onnx_models_found": st.onnx_models_found,
                        "summary": st.summary,
                    }))
                    .unwrap_or_default();
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                "apply_segmentation" => {
                    let id = parse_uuid(args, "id")?;
                    let mut params = BTreeMap::new();
                    params.insert(
                        "sensitivity".into(),
                        finite_clamped(args.get("sensitivity"), 0.55, 0.05, 1.0) as f32,
                    );
                    params.insert(
                        "edge_feather_px".into(),
                        finite_clamped(args.get("edge_feather_px"), 1.5, 0.0, 6.0) as f32,
                    );
                    params.insert(
                        "skin_boost".into(),
                        finite_clamped(args.get("skin_boost"), 0.65, 0.0, 1.0) as f32,
                    );
                    params.insert(
                        "despill_strength".into(),
                        finite_clamped(args.get("despill_strength"), 0.35, 0.0, 1.0) as f32,
                    );
                    params.insert(
                        "invert".into(),
                        if args.get("invert").and_then(Value::as_bool).unwrap_or(false) {
                            1.0
                        } else {
                            0.0
                        },
                    );
                    let eff = EffectNode {
                        id: Uuid::new_v4(),
                        effect_type: "ai_segment".to_string(),
                        enabled: true,
                        params,
                    };
                    let eid = eff.id;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddEffect {
                                item_id: id,
                                effect: eff,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "status": "applied_ai_segment", "effect_id": eid, "cloud_upload": false }).to_string() }] }),
                    )
                }
                "auto_reframe" => {
                    let tw = args
                        .get("target_width")
                        .and_then(Value::as_u64)
                        .unwrap_or(1080)
                        .clamp(64, 4096) as u32;
                    let th = args
                        .get("target_height")
                        .and_then(Value::as_u64)
                        .unwrap_or(1920)
                        .clamp(64, 4096) as u32;
                    let cfg = AutoReframeConfig {
                        target_width: tw,
                        target_height: th,
                        sample_interval_sec: finite_clamped(
                            args.get("sample_interval_sec"),
                            0.4,
                            0.1,
                            5.0,
                        ),
                        smoothing_strength: finite_clamped(
                            args.get("smoothing_strength"),
                            0.72,
                            0.0,
                            0.95,
                        ) as f32,
                        ..Default::default()
                    };
                    let active_id = s.project.active_scene_id;
                    let mut rg = RenderGraph::new();
                    rg.set_project_folder(s.folder.clone());
                    let samples = analyze_scene_auto_reframe(&mut rg, &s.project, active_id, &cfg);
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
                    let mut applied_cmds = 0usize;
                    if apply {
                        let cmds = {
                            let scene = s
                                .project
                                .active_scene()
                                .ok_or_else(|| "no active scene".to_string())?;
                            build_auto_reframe_commands(scene, &cfg, &samples)
                        };
                        applied_cmds = cmds.len();
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::Batch {
                                    label: format!("AI 自動リフレーム ({tw}x{th})"),
                                    commands: cmds,
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                    }
                    let sample_json: Vec<Value> = samples
                        .iter()
                        .map(|sm| {
                            json!({
                                "time_sec": sm.time.as_seconds(),
                                "focus_x": sm.focus_x_norm,
                                "focus_y": sm.focus_y_norm,
                                "confidence": sm.confidence,
                            })
                        })
                        .collect();
                    let text = serde_json::to_string_pretty(&json!({
                        "target": [tw, th],
                        "applied": apply,
                        "commands_applied": applied_cmds,
                        "sample_count": sample_json.len(),
                        "samples": sample_json,
                        "cloud_upload": false,
                    }))
                    .unwrap_or_default();
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                "apply_super_resolution" => {
                    let id = parse_uuid(args, "id")?;
                    let mut params = BTreeMap::new();
                    params.insert(
                        "sharpen".into(),
                        finite_clamped(args.get("sharpen"), 0.55, 0.0, 2.0) as f32,
                    );
                    params.insert(
                        "denoise".into(),
                        finite_clamped(args.get("denoise"), 0.2, 0.0, 1.0) as f32,
                    );
                    let eff = EffectNode {
                        id: Uuid::new_v4(),
                        effect_type: "ai_super_res".to_string(),
                        enabled: true,
                        params,
                    };
                    let eid = eff.id;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddEffect {
                                item_id: id,
                                effect: eff,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "status": "applied_ai_super_res", "effect_id": eid, "cloud_upload": false }).to_string() }] }),
                    )
                }
                "set_optical_flow_slowmo" => {
                    let id = parse_uuid(args, "id")?;
                    let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                    let mut cmds = vec![EditCommand::SetOpticalFlowInterp {
                        item_id: id,
                        enabled,
                    }];
                    if let Some(spd) = args.get("speed").and_then(Value::as_f64) {
                        cmds.push(EditCommand::SetPlaybackSpeed {
                            item_id: id,
                            speed: (spd as f32).clamp(-8.0, 8.0),
                            loop_playback: false,
                        });
                    }
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::Batch {
                                label: "オプティカルフロースロー設定".into(),
                                commands: cmds,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "item_id": id, "optical_flow_interp": enabled }).to_string() }] }),
                    )
                }
                other => Err(format!("unknown run_local_ai_vision action: {other}")),
            }
        }
        "collab_diff_and_review" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing string argument 'action'".to_string())?;
            let s = &mut *guard;
            match action {
                "save_snapshot" => {
                    let folder = s
                        .folder
                        .as_ref()
                        .ok_or_else(|| "no project folder opened".to_string())?;
                    let name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("branch_snapshot");
                    let path = folder
                        .save_snapshot(name, &s.project)
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "saved_snapshot": name, "path": path.display().to_string() }).to_string() }] }),
                    )
                }
                "list_snapshots" => {
                    let list = s
                        .folder
                        .as_ref()
                        .map(|f| f.list_snapshots())
                        .unwrap_or_default();
                    Ok(
                        json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&list).unwrap_or_default() }] }),
                    )
                }
                "diff_snapshots" => {
                    let folder = s
                        .folder
                        .as_ref()
                        .ok_or_else(|| "no project folder opened".to_string())?;
                    let base_name = args
                        .get("base_snapshot")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'base_snapshot'".to_string())?;
                    let base_proj = folder.load_snapshot(base_name).map_err(|e| e.to_string())?;
                    let target_proj =
                        if let Some(tn) = args.get("target_snapshot").and_then(Value::as_str) {
                            folder.load_snapshot(tn).map_err(|e| e.to_string())?
                        } else {
                            s.project.clone()
                        };
                    let diff = diff_projects(&base_proj, &target_proj);
                    let report = format_diff_report(&diff);
                    let text = serde_json::to_string_pretty(&json!({
                        "total_changes": diff.total_changes(),
                        "report": report,
                        "diff": diff,
                    }))
                    .unwrap_or_default();
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                "merge_three_way" => {
                    let folder = s
                        .folder
                        .as_ref()
                        .ok_or_else(|| "no project folder opened".to_string())?;
                    let base_name = args
                        .get("base_snapshot")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'base_snapshot'".to_string())?;
                    let theirs_name = args
                        .get("theirs_snapshot")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'theirs_snapshot'".to_string())?;
                    let base_proj = folder.load_snapshot(base_name).map_err(|e| e.to_string())?;
                    let theirs_proj = folder
                        .load_snapshot(theirs_name)
                        .map_err(|e| e.to_string())?;

                    let mut resolutions = BTreeMap::new();
                    if let Some(obj) = args.get("resolutions").and_then(Value::as_object) {
                        for (k, v) in obj {
                            let res = match v.as_str().unwrap_or("keep_ours") {
                                "keep_theirs" => ConflictResolution::KeepTheirs,
                                "keep_both_duplicate" => ConflictResolution::KeepBothDuplicate,
                                _ => ConflictResolution::KeepOurs,
                            };
                            resolutions.insert(k.clone(), res);
                        }
                    }

                    let merged = merge_projects_three_way(
                        &base_proj,
                        &s.project,
                        &theirs_proj,
                        &resolutions,
                    );
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(false);
                    let applied = if apply && merged.conflicts.is_empty() {
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::ReplaceProjectSnapshot {
                                    project: Box::new(merged.merged_project.clone()),
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                        true
                    } else {
                        false
                    };
                    let text = serde_json::to_string_pretty(&json!({
                        "auto_merged_from_theirs": merged.auto_merged_from_theirs,
                        "conflict_count": merged.conflicts.len(),
                        "conflicts": merged.conflicts,
                        "applied": applied,
                    }))
                    .unwrap_or_default();
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                "add_review_comment" => {
                    let t = parse_time_arg(args, "frame", "time_sec", fps, s.playhead);
                    let author = args
                        .get("author")
                        .and_then(Value::as_str)
                        .unwrap_or("Reviewer");
                    let body = args
                        .get("body")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'body'".to_string())?;
                    let mut comments = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .review_comments
                        .clone();
                    let comment = ReviewComment::new(t, author, body, ReviewSource::Internal);
                    let cid = comment.id;
                    comments.push(comment);
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetReviewComments { comments },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "comment_id": cid, "time_sec": t.as_seconds() }).to_string() }] }),
                    )
                }
                "resolve_review_comment" => {
                    let cid = parse_uuid(args, "comment_id")?;
                    let new_status = match args
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("resolved")
                    {
                        "open" => ReviewStatus::Open,
                        "in_progress" => ReviewStatus::InProgress,
                        _ => ReviewStatus::Resolved,
                    };
                    let mut comments = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .review_comments
                        .clone();
                    let mut found = false;
                    for c in &mut comments {
                        if c.id == cid {
                            c.status = new_status;
                            found = true;
                        }
                    }
                    if !found {
                        return Err(format!("review comment not found: {cid}"));
                    }
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetReviewComments { comments },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "comment_id": cid, "status": new_status }).to_string() }] }),
                    )
                }
                "import_reviews" => {
                    let fmt = args
                        .get("format")
                        .and_then(Value::as_str)
                        .unwrap_or("frameio_csv");
                    let content = args
                        .get("content")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'content'".to_string())?;
                    let imported = match fmt {
                        "json" => import_review_comments_json(content, fps)?,
                        "youtube_text" => import_youtube_review_comments(content, fps),
                        _ => import_review_comments_csv(content, fps),
                    };
                    let added = imported.len();
                    let mut comments = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .review_comments
                        .clone();
                    comments.extend(imported);
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetReviewComments { comments },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "imported_count": added, "format": fmt }).to_string() }] }),
                    )
                }
                "export_reviews" => {
                    let fmt = args
                        .get("format")
                        .and_then(Value::as_str)
                        .unwrap_or("frameio_csv");
                    let scene = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?;
                    let text = match fmt {
                        "json" => export_review_comments_json(&scene.review_comments),
                        _ => export_review_comments_csv(&scene.review_comments, scene.fps),
                    };
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                "convert_reviews_to_markers" => {
                    let only_unresolved = args
                        .get("only_unresolved")
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    let cmds = {
                        let scene = s
                            .project
                            .active_scene()
                            .ok_or_else(|| "no active scene".to_string())?;
                        build_review_comments_to_markers_commands(
                            &scene.review_comments,
                            only_unresolved,
                        )
                    };
                    let count = cmds.len();
                    if count > 0 {
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::Batch {
                                    label: format!("レビューコメント→マーカー変換 ({count}件)"),
                                    commands: cmds,
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                    }
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "converted_markers": count }).to_string() }] }),
                    )
                }
                other => Err(format!("unknown collab_diff_and_review action: {other}")),
            }
        }
        "crdt_realtime_collab" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            match action {
                "status" => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": json!({
                            "peer_id": s.crdt_log.local_peer_id,
                            "display_name": s.crdt_log.display_name,
                            "color_hex": s.crdt_log.color_hex,
                            "lamport_clock": s.crdt_log.lamport_clock,
                            "vector_clock": s.crdt_log.vector_clock,
                            "op_count": s.crdt_log.operations.len(),
                            "peers": s.crdt_log.peers
                        }).to_string()
                    }]
                })),
                "set_peer_profile" => {
                    if let Some(pid) = args.get("peer_id").and_then(Value::as_str)
                        && !pid.trim().is_empty()
                    {
                        s.crdt_log.local_peer_id = pid.trim().to_string();
                    }
                    if let Some(dn) = args.get("display_name").and_then(Value::as_str)
                        && !dn.trim().is_empty()
                    {
                        s.crdt_log.display_name = dn.trim().to_string();
                    }
                    if let Some(c) = args.get("color_hex").and_then(Value::as_str)
                        && !c.trim().is_empty()
                    {
                        s.crdt_log.color_hex = c.trim().to_string();
                    }
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "peer_id": s.crdt_log.local_peer_id, "display_name": s.crdt_log.display_name }).to_string() }]
                    }))
                }
                "update_cursor" => {
                    let playhead_sec = finite_clamped(
                        args.get("playhead_sec"),
                        s.playhead.as_seconds(),
                        0.0,
                        86_400.0,
                    );
                    let sel = args
                        .get("selected_item")
                        .and_then(Value::as_str)
                        .and_then(|v| Uuid::parse_str(v).ok());
                    let op = s.crdt_log.update_local_cursor(playhead_sec, sel);
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&op).unwrap_or_default() }]
                    }))
                }
                "record_marker_op" => {
                    let t = finite_clamped(
                        args.get("time_sec"),
                        s.playhead.as_seconds(),
                        0.0,
                        86_400.0,
                    );
                    let label = args
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or("CRDT Marker");
                    let marker = TimelineMarker::new(RationalTime::from_seconds(t), label);
                    let op = s.crdt_log.record_local(CrdtOpKind::AddMarker { marker });
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&op).unwrap_or_default() }]
                    }))
                }
                "record_text_op" => {
                    let item_id = parse_uuid(args, "id")?;
                    let content = args
                        .get("content")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'content'".to_string())?
                        .to_string();
                    let op = s
                        .crdt_log
                        .record_local(CrdtOpKind::SetTextContent { item_id, content });
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&op).unwrap_or_default() }]
                    }))
                }
                "export_delta" => {
                    let delta = s.crdt_log.export_delta_json(None);
                    Ok(json!({ "content": [{ "type": "text", "text": delta }] }))
                }
                "merge_remote_delta" => {
                    let delta_str = args
                        .get("delta_json")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'delta_json'".to_string())?;
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
                    let ops = CrdtSyncLog::parse_delta_json(delta_str)?;
                    let merged_new = s.crdt_log.merge_remote_ops(&ops);
                    let applied = if apply {
                        let converged = s.crdt_log.apply_to_project(&s.project);
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::ReplaceProjectSnapshot {
                                    project: Box::new(converged),
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
                                "merged_new_ops": merged_new,
                                "total_ops": s.crdt_log.operations.len(),
                                "lamport_clock": s.crdt_log.lamport_clock,
                                "peer_count": s.crdt_log.peers.len(),
                                "applied": applied
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown crdt_realtime_collab action: {other}")),
            }
        }
        other => Err(format!("unknown tool: {other}")),
    }
}
