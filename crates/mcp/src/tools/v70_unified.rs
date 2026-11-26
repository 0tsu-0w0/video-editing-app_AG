//! v7.0 #45〜#48: Unified Studio UX & Core Architecture vNext 新規 MCP ツール群（4 ツール）

use super::*;

pub(super) const DOMAIN_ID: &str = "v70_unified";
pub(super) const DOMAIN_LABEL: &str =
    "Unified Studio Hub, MCP Introspection, 4K/8K GPU Pool & 10k/Conform/AB";

pub(super) fn definitions() -> Vec<Value> {
    let mut out = Vec::new();
    if let Value::Array(items) = json!([
        {
            "name": "studio_hub_and_command_palette",
            "description": "v7.0 #45: 役割別 6 スタジオ Hub（Edit / VFX & Motion / Color / Audio / AI & Collab / Deliver）の一覧取得、および Ctrl+K コマンドパレットの決定論ファジー検索を実行する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list_hubs", "search_palette", "get_command"]
                    },
                    "query": { "type": "string" },
                    "hub": {
                        "type": "string",
                        "enum": ["edit", "vfx_motion", "color", "audio", "ai_collab", "deliver"]
                    },
                    "command_id": { "type": "string" },
                    "recent_ids": { "type": "array", "items": { "type": "string" } },
                    "limit": { "type": "integer" }
                }
            }
        },
        {
            "name": "mcp_catalog_introspect",
            "description": "v7.0 #46: ドメイン別分割された内蔵 MCP ツール群（全 100 ツール / 10 ドメイン）のドメイン統計・キーワード横断検索・JSON Schema 整合性検証を実行する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["summary", "search_tools", "verify_schemas"]
                    },
                    "domain": { "type": "string" },
                    "query": { "type": "string" }
                }
            }
        },
        {
            "name": "gpu_v70_frame_pool",
            "description": "v7.0 #47: GPU-Resident 4K/8K 合成フレームプール（VRAM/RAM 再利用・LRU Eviction）のステータス診断、WGSL ブレンド/マスク/ACES 色管理高速パス実行、および 1080p/4K/8K ベンチマークを実行する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["status", "benchmark", "run_fast_blend_aces"]
                    },
                    "preset_resolution": {
                        "type": "string",
                        "enum": ["1080p", "4k", "8k", "custom"]
                    },
                    "width": { "type": "integer" },
                    "height": { "type": "integer" },
                    "iterations": { "type": "integer" },
                    "budget_mib": { "type": "integer" },
                    "blend_mode": {
                        "type": "string",
                        "enum": ["normal", "add", "multiply", "screen"]
                    },
                    "mask_shape": {
                        "type": "string",
                        "enum": ["none", "soft_rect", "soft_ellipse"]
                    },
                    "mask_feather": { "type": "number" },
                    "opacity": { "type": "number" }
                }
            }
        },
        {
            "name": "timeline_index_conform_ab",
            "description": "v7.0 #48: 10,000+ クリップ区間木 O(log N) 検索＆ベンチマーク、プロキシ⇔カメラ原本 SMPTE タイムコード自動コンフォーム、および A/B スプリット比較ビュー設定＆比較 PNG プレビュー出力を実行する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": [
                            "query_interval_index",
                            "benchmark_10k_index",
                            "set_asset_conform_meta",
                            "conform_timeline",
                            "set_ab_compare",
                            "render_ab_preview"
                        ]
                    },
                    "time_sec": { "type": "number" },
                    "end_sec": { "type": "number" },
                    "clip_count": { "type": "integer" },
                    "query_count": { "type": "integer" },
                    "asset_id": { "type": "string" },
                    "reel_name": { "type": "string" },
                    "start_timecode": { "type": "string" },
                    "is_proxy_media": { "type": "boolean" },
                    "linked_counterpart_id": { "type": "string" },
                    "direction": {
                        "type": "string",
                        "enum": ["proxy_to_original", "original_to_proxy"]
                    },
                    "apply": { "type": "boolean" },
                    "ab_mode": {
                        "type": "string",
                        "enum": ["off", "vertical_split", "horizontal_split", "diagonal_split", "difference_amplified"]
                    },
                    "ab_reference": {
                        "type": "string",
                        "enum": ["ungraded_bypass", "frozen_still", "reference_timecode", "alternate_scene"]
                    },
                    "split_ratio": { "type": "number" },
                    "reference_sec": { "type": "number" },
                    "diff_gain": { "type": "number" },
                    "show_divider": { "type": "boolean" },
                    "swap_sides": { "type": "boolean" },
                    "scale": { "type": "number" }
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
    let res = match name {
        "studio_hub_and_command_palette" => handle_studio_hub_palette(args),
        "mcp_catalog_introspect" => handle_mcp_introspect(args),
        "gpu_v70_frame_pool" => handle_gpu_v70_frame_pool(guard, args),
        "timeline_index_conform_ab" => handle_timeline_index_conform_ab(guard, fps, name, args),
        _ => return None,
    };
    Some(res)
}

fn text_ok(payload: Value) -> Result<Value, String> {
    let text = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

fn handle_studio_hub_palette(args: &Value) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("list_hubs");
    match action {
        "list_hubs" => {
            let filter_hub = args
                .get("hub")
                .and_then(Value::as_str)
                .and_then(StudioHubId::from_slug);
            let hubs: Vec<Value> = StudioHubId::ALL
                .iter()
                .copied()
                .filter(|h| filter_hub.is_none_or(|wanted| wanted == *h))
                .map(|h| {
                    let cmds: Vec<Value> = commands_for_hub(h)
                        .into_iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "title_ja": c.title_ja,
                                "title_en": c.title_en,
                                "roadmap_tag": c.roadmap_tag,
                                "shortcut_hint": c.shortcut_hint,
                            })
                        })
                        .collect();
                    json!({
                        "hub": h.slug(),
                        "label_ja": h.button_label(UiLanguage::Japanese),
                        "label_en": h.button_label(UiLanguage::English),
                        "summary_ja": h.summary(UiLanguage::Japanese),
                        "summary_en": h.summary(UiLanguage::English),
                        "command_count": cmds.len(),
                        "commands": cmds,
                    })
                })
                .collect();
            text_ok(json!({
                "hub_count": hubs.len(),
                "total_palette_commands": studio_palette_catalog().len(),
                "preserved_workspaces": ["main_edit", "motion_composite", "node_composite"],
                "hubs": hubs,
            }))
        }
        "search_palette" => {
            let query = args.get("query").and_then(Value::as_str).unwrap_or("");
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(10)
                .clamp(1, 50) as usize;
            let recent: Vec<String> = args
                .get("recent_ids")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            let hits: Vec<Value> = fuzzy_search_commands(query, &recent, limit)
                .into_iter()
                .map(|h| {
                    json!({
                        "id": h.entry.id,
                        "hub": h.entry.hub.map(StudioHubId::slug),
                        "title_ja": h.entry.title_ja,
                        "title_en": h.entry.title_en,
                        "roadmap_tag": h.entry.roadmap_tag,
                        "shortcut_hint": h.entry.shortcut_hint,
                        "score": h.score,
                    })
                })
                .collect();
            text_ok(json!({
                "query": query,
                "hit_count": hits.len(),
                "hits": hits,
            }))
        }
        "get_command" => {
            let cid = args
                .get("command_id")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'command_id'".to_string())?;
            let found = studio_palette_catalog()
                .iter()
                .find(|e| e.id == cid)
                .ok_or_else(|| format!("unknown command_id: {cid}"))?;
            text_ok(json!({
                "id": found.id,
                "hub": found.hub.map(StudioHubId::slug),
                "title_ja": found.title_ja,
                "title_en": found.title_en,
                "roadmap_tag": found.roadmap_tag,
                "shortcut_hint": found.shortcut_hint,
                "keywords": found.keywords,
            }))
        }
        other => Err(format!(
            "unknown studio_hub_and_command_palette action: {other}"
        )),
    }
}

fn handle_mcp_introspect(args: &Value) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("summary");
    let catalog = tool_domain_catalog();
    let total_tools: usize = catalog.iter().map(|d| d.tool_count).sum();

    match action {
        "summary" => text_ok(json!({
            "domain_count": catalog.len(),
            "total_tools": total_tools,
            "domains": catalog,
        })),
        "search_tools" => {
            let domain_filter = args.get("domain").and_then(Value::as_str);
            let q = args
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_lowercase();
            let all_defs = tool_definitions();
            let defs_arr = all_defs.as_array().cloned().unwrap_or_default();

            let mut domain_by_tool = BTreeMap::new();
            for d in &catalog {
                for tn in &d.tool_names {
                    domain_by_tool.insert(tn.clone(), d.domain_id.clone());
                }
            }

            let matched: Vec<Value> = defs_arr
                .into_iter()
                .filter_map(|def| {
                    let name = def.get("name")?.as_str()?;
                    let desc = def.get("description").and_then(Value::as_str).unwrap_or("");
                    let dom = domain_by_tool.get(name).map(String::as_str).unwrap_or("");
                    if let Some(df) = domain_filter
                        && !df.is_empty()
                        && dom != df
                    {
                        return None;
                    }
                    if !q.is_empty()
                        && !name.to_lowercase().contains(&q)
                        && !desc.to_lowercase().contains(&q)
                        && !dom.to_lowercase().contains(&q)
                    {
                        return None;
                    }
                    Some(json!({
                        "name": name,
                        "domain": dom,
                        "description": desc,
                    }))
                })
                .collect();

            text_ok(json!({
                "match_count": matched.len(),
                "matches": matched,
            }))
        }
        "verify_schemas" => {
            let all_defs = tool_definitions();
            let arr = all_defs
                .as_array()
                .ok_or_else(|| "tool_definitions must return JSON array".to_string())?;
            let mut seen = std::collections::BTreeSet::new();
            let mut invalid = Vec::new();
            for def in arr {
                let name = def.get("name").and_then(Value::as_str).unwrap_or("");
                let desc = def.get("description").and_then(Value::as_str).unwrap_or("");
                let schema_ty = def
                    .get("inputSchema")
                    .and_then(|s| s.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if name.is_empty()
                    || desc.is_empty()
                    || schema_ty != "object"
                    || !seen.insert(name.to_string())
                {
                    invalid.push(name.to_string());
                }
            }
            text_ok(json!({
                "valid": invalid.is_empty(),
                "unique_tool_count": seen.len(),
                "invalid_entries": invalid,
            }))
        }
        other => Err(format!("unknown mcp_catalog_introspect action: {other}")),
    }
}

fn handle_gpu_v70_frame_pool(
    guard: &mut crate::session::EditorSession,
    args: &Value,
) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("status");
    match action {
        "status" => {
            let st = detect_gpu_v70_status();
            text_ok(json!({
                "gpu_available": st.gpu_available,
                "adapter_name": st.adapter_name,
                "backend_name": st.backend_name,
                "blend_mask_shader_valid": st.blend_mask_shader_valid,
                "aces_color_shader_valid": st.aces_color_shader_valid,
                "max_stripe_bytes": st.max_stripe_bytes,
            }))
        }
        "benchmark" => {
            let preset = args
                .get("preset_resolution")
                .and_then(Value::as_str)
                .unwrap_or("4k");
            let (def_w, def_h) = match preset {
                "1080p" => (1920u32, 1080u32),
                "8k" => (7680u32, 4320u32),
                "4k" => (3840u32, 2160u32),
                _ => (1280u32, 720u32),
            };
            let w = args
                .get("width")
                .and_then(Value::as_u64)
                .map(|v| v.clamp(16, 8192) as u32)
                .unwrap_or(def_w);
            let h = args
                .get("height")
                .and_then(Value::as_u64)
                .map(|v| v.clamp(16, 8192) as u32)
                .unwrap_or(def_h);
            let iters = args
                .get("iterations")
                .and_then(Value::as_u64)
                .unwrap_or(3)
                .clamp(2, 16) as u32;
            let budget_mib = args
                .get("budget_mib")
                .and_then(Value::as_u64)
                .unwrap_or(768)
                .clamp(64, 4096);
            let rep = benchmark_v70_frame_pool(w, h, iters, budget_mib);
            text_ok(json!({
                "width": rep.width,
                "height": rep.height,
                "iterations": rep.iterations,
                "executed_on_gpu": rep.executed_on_gpu,
                "backend_name": rep.backend_name,
                "elapsed_ms": rep.elapsed_ms,
                "estimated_fps": rep.estimated_fps,
                "pool_hit_rate": rep.pool_metrics.hit_rate(),
                "allocations_created": rep.pool_metrics.allocations_created,
                "pool_reuses": rep.pool_metrics.pool_reuses,
                "evictions": rep.pool_metrics.evictions,
                "resident_bytes": rep.pool_metrics.resident_bytes,
                "gpu_resident_buffers": rep.pool_metrics.gpu_resident_buffers,
            }))
        }
        "run_fast_blend_aces" => {
            let active_id = guard.project.active_scene_id;
            let t = guard.playhead;
            let mut rg = RenderGraph::new();
            rg.active_revision = guard.bus.revision().max(1);
            if let Some(ref f) = guard.folder {
                rg.set_project_folder(Some(f.clone()));
            }
            let base = rg
                .render_project_scene_scaled(&guard.project, active_id, t, 320, 180, false)
                .ok_or_else(|| "failed to render active scene".to_string())?;

            let blend_mode = match args.get("blend_mode").and_then(Value::as_str) {
                Some("add") => BlendMode::Add,
                Some("multiply") => BlendMode::Multiply,
                Some("screen") => BlendMode::Screen,
                _ => BlendMode::Normal,
            };
            let mask_shape = match args.get("mask_shape").and_then(Value::as_str) {
                Some("soft_rect") => FastMaskShape::SoftRect,
                Some("soft_ellipse") => FastMaskShape::SoftEllipse,
                _ => FastMaskShape::None,
            };
            let params = FastBlendMaskParams {
                blend_mode,
                opacity: finite_clamped(args.get("opacity"), 0.85, 0.0, 1.0) as f32,
                matte_mode: None,
                mask_shape,
                mask_center: [0.5, 0.5],
                mask_half_size: [0.36, 0.36],
                mask_feather: finite_clamped(args.get("mask_feather"), 0.06, 0.001, 0.5) as f32,
                invert_mask: false,
            };
            let mut gpu = GpuV70FastPipeline::try_headless();
            let (mut out_img, blend_gpu) = execute_v70_blend_mask_or_cpu(
                gpu.as_mut(),
                &base,
                &base,
                None,
                &params,
                &mut rg.frame_pool,
            );
            let cm_cfg = guard
                .project
                .active_scene()
                .map(|s| s.color_management)
                .unwrap_or(ColorManagementConfig {
                    enabled: true,
                    ..ColorManagementConfig::default()
                });
            let aces_gpu = execute_v70_aces_or_cpu(gpu.as_mut(), &mut out_img, &cm_cfg);
            let metrics = rg.frame_pool.metrics(
                gpu.as_ref()
                    .map_or(0, GpuV70FastPipeline::resident_buffer_count),
            );
            rg.frame_pool.release_linear_canvas(out_img);

            text_ok(json!({
                "width": 320,
                "height": 180,
                "blend_ran_on_gpu": blend_gpu,
                "aces_ran_on_gpu": aces_gpu,
                "pool_allocations": metrics.allocations_created,
                "pool_reuses": metrics.pool_reuses,
            }))
        }
        other => Err(format!("unknown gpu_v70_frame_pool action: {other}")),
    }
}

fn handle_timeline_index_conform_ab(
    guard: &mut crate::session::EditorSession,
    fps: FrameRate,
    _tool_name: &str,
    args: &Value,
) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("query_interval_index");
    match action {
        "query_interval_index" => {
            let scene = guard
                .project
                .active_scene()
                .ok_or_else(|| "no active scene".to_string())?;
            let idx = SceneIntervalIndex::build(scene);
            let t_start = RationalTime::from_seconds(finite_clamped(
                args.get("time_sec"),
                guard.playhead.as_seconds(),
                0.0,
                86_400.0,
            ));
            let (hits, visited) = if let Some(end_s) = args.get("end_sec").and_then(Value::as_f64) {
                let t_end = RationalTime::from_seconds(end_s.max(t_start.as_seconds()));
                idx.query_range_with_stats(t_start, t_end)
            } else {
                idx.query_at_with_stats(t_start)
            };
            text_ok(json!({
                "total_clips": idx.total_clips(),
                "visited_nodes": visited,
                "hit_count": hits.len(),
                "hits": hits,
            }))
        }
        "benchmark_10k_index" => {
            let clips = args
                .get("clip_count")
                .and_then(Value::as_u64)
                .unwrap_or(10_000)
                .clamp(64, 50_000) as usize;
            let queries = args
                .get("query_count")
                .and_then(Value::as_u64)
                .unwrap_or(100)
                .clamp(10, 2_000) as usize;
            let rep = benchmark_interval_index(clips, queries);
            text_ok(serde_json::to_value(rep).map_err(|e| e.to_string())?)
        }
        "set_asset_conform_meta" => {
            let asset_id = parse_uuid(args, "asset_id")?;
            if guard.project.asset(asset_id).is_none() {
                return Err(format!("asset not found: {asset_id}"));
            }
            let mut meta = guard
                .project
                .conform_meta
                .get(&asset_id)
                .cloned()
                .unwrap_or_default();
            if let Some(reel) = args.get("reel_name").and_then(Value::as_str) {
                meta.reel_name = if reel.trim().is_empty() {
                    None
                } else {
                    Some(reel.trim().to_string())
                };
            }
            if let Some(tc) = args.get("start_timecode").and_then(Value::as_str) {
                meta.start_timecode = if tc.trim().is_empty() {
                    None
                } else {
                    Some(tc.trim().to_string())
                };
            }
            if let Some(flag) = args.get("is_proxy_media").and_then(Value::as_bool) {
                meta.is_proxy_media = flag;
            }
            if let Some(link_str) = args.get("linked_counterpart_id").and_then(Value::as_str) {
                meta.linked_counterpart_id = if link_str.trim().is_empty() {
                    None
                } else {
                    Some(
                        Uuid::parse_str(link_str.trim())
                            .map_err(|e| format!("invalid linked_counterpart_id: {e}"))?,
                    )
                };
            }
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetAssetConformMeta {
                        asset_id,
                        meta: meta.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            text_ok(json!({
                "updated": true,
                "asset_id": asset_id,
                "meta": meta,
            }))
        }
        "conform_timeline" => {
            let direction = match args.get("direction").and_then(Value::as_str) {
                Some("original_to_proxy") => ConformDirection::OriginalToProxy,
                _ => ConformDirection::ProxyToOriginal,
            };
            let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
            let scene_id = guard.project.active_scene_id;
            let report = plan_timeline_conform(&guard.project, scene_id, direction);
            let mut applied = false;
            if apply && let Some(cmd) = build_timeline_conform_command(&report) {
                let s = &mut *guard;
                s.bus
                    .execute(&mut s.project, cmd, CommandSource::Mcp, None)
                    .map_err(|e| e.to_string())?;
                applied = true;
            }
            text_ok(json!({
                "applied": applied,
                "action_count": report.actions.len(),
                "issue_count": report.issues.len(),
                "skipped_already_target": report.skipped_already_target,
                "report": report,
            }))
        }
        "set_ab_compare" => {
            let mut cfg = guard
                .project
                .active_scene()
                .map(|s| s.ab_compare.clone())
                .unwrap_or_default();
            if let Some(m) = args.get("ab_mode").and_then(Value::as_str) {
                cfg.mode = match m {
                    "off" => AbCompareMode::Off,
                    "vertical_split" => AbCompareMode::VerticalSplit,
                    "horizontal_split" => AbCompareMode::HorizontalSplit,
                    "diagonal_split" => AbCompareMode::DiagonalSplit,
                    "difference_amplified" => AbCompareMode::DifferenceAmplified,
                    other => return Err(format!("unknown ab_mode: {other}")),
                };
            }
            if let Some(r) = args.get("ab_reference").and_then(Value::as_str) {
                cfg.reference = match r {
                    "ungraded_bypass" => AbReferenceSource::UngradedBypass,
                    "frozen_still" => AbReferenceSource::FrozenStill,
                    "reference_timecode" => AbReferenceSource::ReferenceTimecode,
                    "alternate_scene" => AbReferenceSource::AlternateScene,
                    other => return Err(format!("unknown ab_reference: {other}")),
                };
            }
            if args.get("split_ratio").is_some() {
                cfg.split_ratio = finite_clamped(args.get("split_ratio"), 0.5, 0.05, 0.95) as f32;
            }
            if args.get("reference_sec").is_some() {
                cfg.reference_sec = finite_clamped(args.get("reference_sec"), 0.0, 0.0, 86_400.0);
            }
            if args.get("diff_gain").is_some() {
                cfg.diff_gain = finite_clamped(args.get("diff_gain"), 4.0, 1.0, 16.0) as f32;
            }
            if let Some(d) = args.get("show_divider").and_then(Value::as_bool) {
                cfg.show_divider = d;
            }
            if let Some(sw) = args.get("swap_sides").and_then(Value::as_bool) {
                cfg.swap_sides = sw;
            }
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetAbCompare {
                        ab_compare: cfg.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            text_ok(json!({
                "updated": true,
                "ab_compare": cfg,
            }))
        }
        "render_ab_preview" => {
            let scene = guard
                .project
                .active_scene()
                .ok_or_else(|| "no active scene".to_string())?;
            let scale = finite_clamped(args.get("scale"), 0.5, 0.1, 1.0);
            let tw = ((scene.width as f64 * scale).round() as u32).max(32);
            let th = ((scene.height as f64 * scale).round() as u32).max(18);
            let t = parse_time_arg(args, "frame", "time_sec", fps, guard.playhead);
            let mut rg = RenderGraph::new();
            rg.active_revision = guard.bus.revision().max(1);
            if let Some(ref f) = guard.folder {
                rg.set_project_folder(Some(f.clone()));
            }
            let img = render_ab_compare_scene(
                &mut rg,
                &guard.project,
                guard.project.active_scene_id,
                t,
                tw,
                th,
                None,
            )
            .ok_or_else(|| "failed to render A/B preview".to_string())?;
            let png_bytes = encode_linear_image_png_bytes(&img)?;
            let b64 = base64_encode(&png_bytes);
            Ok(json!({
                "content": [{
                    "type": "image",
                    "mimeType": "image/png",
                    "data": b64
                }]
            }))
        }
        other => Err(format!("unknown timeline_index_conform_ab action: {other}")),
    }
}
