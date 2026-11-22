use crate::session::SharedEditorSession;
use serde_json::{Value, json};
use ve_core::{
    ApprovalGateStatus, EditRecipe, RationalTime, RecipeRunState, RecipeStepLog,
    VisualCheckExpectation, VisualCheckReport, builtin_edit_recipes, parse_recipe_json_or_yaml,
    record_recipe_from_commands, resolve_template_vars, serialize_recipe_yaml,
};
use ve_render::{LinearImage, RenderGraph};

const MAX_RECIPE_STEPS: usize = 64;

/// レンダリング結果フレームから輝度・コントラスト・非黒領域カバレッジを算出し、視覚自己検査レポートを生成する
pub fn inspect_preview_frame_metrics(
    image: &LinearImage,
    expect: &VisualCheckExpectation,
) -> VisualCheckReport {
    let total = image.pixels.len().max(1) as f64;
    let mut sum_luma = 0.0f64;
    let mut sum_sq = 0.0f64;
    let mut non_black = 0usize;

    for px in &image.pixels {
        let l = f64::from(px.luma());
        sum_luma += l;
        sum_sq += l * l;
        if l > 0.008 || px.a > 0.05 && (px.r + px.g + px.b) > 0.015 {
            non_black += 1;
        }
    }
    let mean = (sum_luma / total) as f32;
    let var = (sum_sq / total - (sum_luma / total).powi(2)).max(0.0);
    let contrast_std = var.sqrt() as f32;
    let non_black_ratio = (non_black as f64 / total) as f32;

    let mut issues = Vec::new();
    if mean < expect.min_mean_luma {
        issues.push(format!(
            "平均輝度が暗すぎます ({mean:.3} < {:.3}) — 露出・テロップ配置を確認してください",
            expect.min_mean_luma
        ));
    }
    if mean > expect.max_mean_luma {
        issues.push(format!(
            "平均輝度が白飛び気味です ({mean:.3} > {:.3}) — ハイライト/Gainを下げてください",
            expect.max_mean_luma
        ));
    }
    if non_black_ratio < expect.min_non_black_ratio {
        issues.push(format!(
            "有効描画領域が少なすぎます ({:.1}% < {:.1}%) — クリップ時刻・可視状態を確認してください",
            non_black_ratio * 100.0,
            expect.min_non_black_ratio * 100.0
        ));
    }
    let passed = issues.is_empty();
    let advice = if passed {
        format!(
            "視覚自己検査 OK (平均輝度 {mean:.3}, コントラスト {contrast_std:.3}, 描画率 {:.1}%)",
            non_black_ratio * 100.0
        )
    } else {
        issues.join(" / ")
    };

    VisualCheckReport {
        time_sec: expect.time_sec,
        mean_luma: mean,
        contrast_std,
        non_black_ratio,
        passed,
        advice,
    }
}

/// アクティブセッションにレシピをセットして新規実行を開始する
pub fn start_recipe_run(
    session: &SharedEditorSession,
    recipe: EditRecipe,
) -> Result<RecipeRunState, String> {
    if recipe.steps.len() > MAX_RECIPE_STEPS {
        return Err(format!(
            "recipe exceeds max step limit ({MAX_RECIPE_STEPS})"
        ));
    }
    let state = RecipeRunState::start(&recipe);
    let mut g = session
        .lock()
        .map_err(|_| "session lock poisoned".to_string())?;
    g.active_recipe = Some(recipe);
    g.recipe_run = Some(state.clone());
    Ok(state)
}

/// 次の 1 ステップを実行する。`allow_approved_gate` が false で対象ステップが承認ゲート必須なら `AwaitingApproval` で一時停止する。
pub fn advance_recipe_step(
    session: &SharedEditorSession,
    allow_approved_gate: bool,
) -> Result<RecipeRunState, String> {
    // 1. ロックを取得して現在ステップの情報を抜き出し、即ロック解放（call_tool のデッドロック防止）
    let (step_idx, step, vars, undo_before) = {
        let mut g = session
            .lock()
            .map_err(|_| "session lock poisoned".to_string())?;
        let recipe = g
            .active_recipe
            .clone()
            .ok_or_else(|| "no active edit recipe; call start_run first".to_string())?;
        let run = g
            .recipe_run
            .as_mut()
            .ok_or_else(|| "no active recipe run".to_string())?;

        if run.current_step_index >= recipe.steps.len() {
            run.status = ApprovalGateStatus::Completed;
            run.pending_gate_step_id = None;
            run.pending_gate_title = None;
            run.last_message = format!("レシピ「{}」は全ステップ完了しました", recipe.name);
            return Ok(run.clone());
        }

        let idx = run.current_step_index;
        let step = recipe.steps[idx].clone();
        if step.requires_human_approval && !allow_approved_gate {
            run.status = ApprovalGateStatus::AwaitingApproval;
            run.pending_gate_step_id = Some(step.id.clone());
            run.pending_gate_title = Some(step.title.clone());
            run.last_message = format!(
                "人間承認ゲート待機中 [Step {}/{}]: {} ({})",
                idx + 1,
                recipe.steps.len(),
                step.title,
                step.tool_name
            );
            return Ok(run.clone());
        }

        run.status = ApprovalGateStatus::Running;
        run.pending_gate_step_id = None;
        run.pending_gate_title = None;
        (idx, step, recipe.variables, g.bus.undo_len())
    };

    if step.tool_name == "orchestrate_edit_recipe" {
        let mut g = session
            .lock()
            .map_err(|_| "session lock poisoned".to_string())?;
        if let Some(run) = g.recipe_run.as_mut() {
            run.status = ApprovalGateStatus::Failed;
            run.last_message = "再帰的な orchestrate_edit_recipe 呼び出しは禁止されています".into();
            return Ok(run.clone());
        }
        return Err("recursive orchestrate_edit_recipe is forbidden".into());
    }

    // 2. テンプレート変数を展開して対象 MCP ツールを実行
    let resolved_args = resolve_template_vars(&step.args, &vars);
    let call_res = crate::tools::call_tool(session, &step.tool_name, &resolved_args);

    // 3. 任意の視覚自己検査を実行
    let visual_report = if call_res.is_ok()
        && let Some(expect) = &step.visual_check
    {
        let (proj_clone, active_id, folder) = {
            let g = session
                .lock()
                .map_err(|_| "session lock poisoned".to_string())?;
            (
                g.project.clone(),
                g.project.active_scene_id,
                g.folder.clone(),
            )
        };
        let mut rg = RenderGraph::new();
        rg.set_project_folder(folder);
        rg.render_project_scene_scaled(
            &proj_clone,
            active_id,
            RationalTime::from_seconds(expect.time_sec.max(0.0)),
            160,
            90,
            false,
        )
        .map(|img| inspect_preview_frame_metrics(&img, expect))
    } else {
        None
    };

    // 4. ログ追記＆状態更新
    let mut g = session
        .lock()
        .map_err(|_| "session lock poisoned".to_string())?;
    let run = g
        .recipe_run
        .as_mut()
        .ok_or_else(|| "recipe run state missing".to_string())?;

    match call_res {
        Ok(val) => {
            let raw_text = val["content"][0]["text"]
                .as_str()
                .unwrap_or("ok")
                .chars()
                .take(180)
                .collect::<String>();
            let vis_msg = visual_report
                .as_ref()
                .map(|r| format!(" | {}", r.advice))
                .unwrap_or_default();
            run.logs.push(RecipeStepLog {
                step_index: step_idx,
                step_id: step.id.clone(),
                role: step.role,
                title: step.title.clone(),
                tool_name: step.tool_name.clone(),
                succeeded: true,
                output_summary: raw_text,
                undo_depth_before: undo_before,
                visual_report,
            });
            run.current_step_index += 1;
            if run.current_step_index >= run.total_steps {
                run.status = ApprovalGateStatus::Completed;
                run.last_message = format!(
                    "全 {} ステップ完了: {}{}",
                    run.total_steps, step.title, vis_msg
                );
            } else {
                run.status = ApprovalGateStatus::Running;
                run.last_message = format!(
                    "Step {}/{} 完了: {}{}",
                    step_idx + 1,
                    run.total_steps,
                    step.title,
                    vis_msg
                );
            }
        }
        Err(err) => {
            run.logs.push(RecipeStepLog {
                step_index: step_idx,
                step_id: step.id.clone(),
                role: step.role,
                title: step.title.clone(),
                tool_name: step.tool_name.clone(),
                succeeded: false,
                output_summary: err.clone(),
                undo_depth_before: undo_before,
                visual_report: None,
            });
            run.status = ApprovalGateStatus::Failed;
            run.last_message = format!("Step {} 失敗 ({}): {err}", step_idx + 1, step.tool_name);
        }
    }
    Ok(run.clone())
}

/// 承認ゲートまたは完了・失敗まで連続実行する
pub fn run_recipe_until_gate(session: &SharedEditorSession) -> Result<RecipeRunState, String> {
    let mut last_state = None;
    for _ in 0..MAX_RECIPE_STEPS {
        let st = advance_recipe_step(session, false)?;
        let done = matches!(
            st.status,
            ApprovalGateStatus::AwaitingApproval
                | ApprovalGateStatus::Completed
                | ApprovalGateStatus::Failed
                | ApprovalGateStatus::Rejected
        );
        last_state = Some(st);
        if done {
            break;
        }
    }
    last_state.ok_or_else(|| "recipe run produced no state".to_string())
}

/// 現在の承認ゲートを承認してそのステップを実行する（`continue_after` で次のゲート/完了まで続行可能）
pub fn approve_current_gate(
    session: &SharedEditorSession,
    continue_after: bool,
) -> Result<RecipeRunState, String> {
    let st = advance_recipe_step(session, true)?;
    if continue_after && st.status == ApprovalGateStatus::Running {
        run_recipe_until_gate(session)
    } else {
        Ok(st)
    }
}

/// 現在の承認ゲートを却下し、任意で直前ステップの Undo ロールバックまたはスキップを行う
pub fn reject_current_gate(
    session: &SharedEditorSession,
    rollback_last: bool,
    skip_step: bool,
    reason: &str,
) -> Result<RecipeRunState, String> {
    let mut g = session
        .lock()
        .map_err(|_| "session lock poisoned".to_string())?;
    let s = &mut *g;
    if rollback_last && let Some(last_log) = s.recipe_run.as_ref().and_then(|r| r.logs.last()) {
        let target_depth = last_log.undo_depth_before;
        while s.bus.undo_len() > target_depth {
            let _ = s.bus.undo(&mut s.project);
        }
    }
    let run = s
        .recipe_run
        .as_mut()
        .ok_or_else(|| "no active recipe run".to_string())?;
    if skip_step && run.current_step_index < run.total_steps {
        run.current_step_index += 1;
        run.pending_gate_step_id = None;
        run.pending_gate_title = None;
        run.status = if run.current_step_index >= run.total_steps {
            ApprovalGateStatus::Completed
        } else {
            ApprovalGateStatus::Running
        };
        run.last_message = format!("承認ゲートをスキップしました ({reason})");
    } else {
        run.status = ApprovalGateStatus::Rejected;
        run.pending_gate_step_id = None;
        run.pending_gate_title = None;
        run.last_message = format!(
            "承認ゲートで停止しました{} ({})",
            if rollback_last {
                "・直前ステップをUndo済"
            } else {
                ""
            },
            if reason.is_empty() {
                "人間レビュー却下"
            } else {
                reason
            }
        );
    }
    Ok(run.clone())
}

/// `orchestrate_edit_recipe` MCP ツール本体のハンドラ（デッドロック防止のため `session` ロック外で呼ばれる）
pub fn handle_orchestrate_tool(
    session: &SharedEditorSession,
    args: &Value,
) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing string argument 'action'".to_string())?;

    match action {
        "list_recipes" => {
            let mut recipes = builtin_edit_recipes();
            let saved = {
                let g = session
                    .lock()
                    .map_err(|_| "session lock poisoned".to_string())?;
                g.folder
                    .as_ref()
                    .map(|f| f.load_recipes())
                    .unwrap_or_default()
            };
            recipes.extend(saved);
            let list: Vec<Value> = recipes
                .iter()
                .map(|r| {
                    json!({
                        "name": r.name,
                        "description": r.description,
                        "step_count": r.steps.len(),
                        "variables": r.variables,
                    })
                })
                .collect();
            let text =
                serde_json::to_string_pretty(&json!({ "recipes": list })).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "import_recipe" | "save_recipe" => {
            let recipe = extract_recipe_from_args(session, args)?;
            let saved_path = {
                let g = session
                    .lock()
                    .map_err(|_| "session lock poisoned".to_string())?;
                if let Some(folder) = g.folder.as_ref() {
                    Some(
                        folder
                            .save_recipe(&recipe)
                            .map_err(|e| format!("failed to save recipe: {e}"))?,
                    )
                } else {
                    None
                }
            };
            {
                let mut g = session
                    .lock()
                    .map_err(|_| "session lock poisoned".to_string())?;
                g.active_recipe = Some(recipe.clone());
            }
            let text = serde_json::to_string_pretty(&json!({
                "status": "saved",
                "name": recipe.name,
                "step_count": recipe.steps.len(),
                "saved_path": saved_path.map(|p| p.display().to_string()),
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "export_yaml" => {
            let recipe = extract_recipe_from_args(session, args)?;
            let yaml =
                serialize_recipe_yaml(&recipe).map_err(|e| format!("yaml serialize error: {e}"))?;
            Ok(json!({ "content": [{ "type": "text", "text": yaml }] }))
        }
        "record_from_history" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Recorded Macro Recipe");
            let desc = args
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("実行済みコマンド履歴から自動記録された編集レシピ");
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(20)
                .clamp(1, 60) as usize;
            let (recipe, saved_path) = {
                let mut g = session
                    .lock()
                    .map_err(|_| "session lock poisoned".to_string())?;
                let cmds = g.bus.recent_commands(limit);
                let rec = record_recipe_from_commands(name, desc, &cmds);
                let sp = g.folder.as_ref().and_then(|f| f.save_recipe(&rec).ok());
                g.active_recipe = Some(rec.clone());
                (rec, sp)
            };
            let yaml = serialize_recipe_yaml(&recipe).unwrap_or_default();
            let text = serde_json::to_string_pretty(&json!({
                "status": "recorded",
                "name": recipe.name,
                "step_count": recipe.steps.len(),
                "saved_path": saved_path.map(|p| p.display().to_string()),
                "yaml": yaml,
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "start_run" => {
            let mut recipe = extract_recipe_from_args(session, args)?;
            if let Some(vars_obj) = args.get("variables").and_then(Value::as_object) {
                for (k, v) in vars_obj {
                    if let Some(s) = v.as_str() {
                        recipe.variables.insert(k.clone(), s.to_string());
                    }
                }
            }
            let auto_run = args
                .get("auto_run")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let _ = start_recipe_run(session, recipe)?;
            let state = if auto_run {
                run_recipe_until_gate(session)?
            } else {
                let g = session
                    .lock()
                    .map_err(|_| "session lock poisoned".to_string())?;
                g.recipe_run
                    .clone()
                    .ok_or_else(|| "missing run state".to_string())?
            };
            let text = serde_json::to_string_pretty(&state).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "step" => {
            let state = advance_recipe_step(session, false)?;
            let text = serde_json::to_string_pretty(&state).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "run_until_gate" => {
            let state = run_recipe_until_gate(session)?;
            let text = serde_json::to_string_pretty(&state).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "approve_gate" => {
            let cont = args
                .get("continue_after")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let state = approve_current_gate(session, cont)?;
            let text = serde_json::to_string_pretty(&state).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "reject_gate" => {
            let rollback = args
                .get("rollback_last")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let skip = args
                .get("skip_step")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let reason = args.get("reason").and_then(Value::as_str).unwrap_or("");
            let state = reject_current_gate(session, rollback, skip, reason)?;
            let text = serde_json::to_string_pretty(&state).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "get_status" => {
            let g = session
                .lock()
                .map_err(|_| "session lock poisoned".to_string())?;
            let text = serde_json::to_string_pretty(&json!({
                "has_active_recipe": g.active_recipe.is_some(),
                "run": g.recipe_run,
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        other => Err(format!("unknown orchestrate_edit_recipe action: {other}")),
    }
}

fn extract_recipe_from_args(
    session: &SharedEditorSession,
    args: &Value,
) -> Result<EditRecipe, String> {
    if let Some(raw) = args.get("recipe_text").and_then(Value::as_str) {
        return parse_recipe_json_or_yaml(raw).map_err(|e| format!("invalid recipe_text: {e}"));
    }
    if let Some(obj) = args.get("recipe") {
        return serde_json::from_value::<EditRecipe>(obj.clone())
            .map_err(|e| format!("invalid recipe JSON: {e}"));
    }
    if let Some(name) = args.get("recipe_name").and_then(Value::as_str) {
        let mut all = builtin_edit_recipes();
        {
            let g = session
                .lock()
                .map_err(|_| "session lock poisoned".to_string())?;
            if let Some(f) = g.folder.as_ref() {
                all.extend(f.load_recipes());
            }
        }
        if let Some(found) = all.into_iter().find(|r| r.name == name) {
            return Ok(found);
        }
        return Err(format!("recipe not found: {name}"));
    }
    let g = session
        .lock()
        .map_err(|_| "session lock poisoned".to_string())?;
    if let Some(active) = g.active_recipe.clone() {
        return Ok(active);
    }
    Ok(builtin_edit_recipes().remove(0))
}
