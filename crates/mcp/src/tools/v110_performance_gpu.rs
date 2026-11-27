//! v11.0 #61〜#64 Zero-Copy GPU Engine, Real-Time 8K & Native Studio Performance MCP ツール群
//! （4 ツール / 11 ドメイン・全 104 MCP ツール体系化）:
//! - #61 `configure_gpu_render_graph`
//! - #62 `manage_adaptive_playback_and_prerender`
//! - #63 `audit_and_compact_project_memory`
//! - #64 `profile_timeline_bottlenecks`

use super::*;
use crate::session::EditorSession;
use ve_core::{
    AdaptivePlaybackConfig, AdaptiveScaleState, GpuRenderGraphConfig, ProfilerHudConfig,
    audit_project_memory, build_auto_optimize_scene_commands, build_gpu_render_graph_plan,
    compact_project_and_caches, compute_structural_layer_diff, decode_project_fastbin,
    diagnose_timeline_bottlenecks, encode_project_fastbin, load_project_fastbin_from_folder,
    plan_smart_prerender_schedule, save_project_fastbin_to_folder,
};
use ve_render::{benchmark_v110_zero_copy_graph, validate_all_builtin_wgsl_shaders};

pub(crate) const DOMAIN_ID: &str = "v110_performance_gpu";
pub(crate) const DOMAIN_LABEL: &str =
    "v11.0 ゼロコピーGPUグラフ・適応解像度8K再生・FastBin省メモリ・プロファイラHUD (#61〜#64)";

pub(crate) fn definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "configure_gpu_render_graph",
            "description": "v11.0 #61: 完全ゼロコピー GPU レンダリンググラフ（VRAM 常駐 Ping-Pong 連結 WGSL・8K タイリング）の設定・DAG 実行計画・全 9 シェーダー Naga 検証＆ベンチマークを実行する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["status", "configure", "plan", "benchmark"] },
                    "enabled": { "type": "boolean" },
                    "prefer_zero_copy_vram": { "type": "boolean" },
                    "max_vram_textures_mib": { "type": "integer" },
                    "allow_8k_tiling": { "type": "boolean" },
                    "chain_post_fx_on_gpu": { "type": "boolean" },
                    "show_gpu_direct_badge": { "type": "boolean" },
                    "target_width": { "type": "integer" },
                    "target_height": { "type": "integer" }
                }
            }
        }),
        json!({
            "name": "manage_adaptive_playback_and_prerender",
            "description": "v11.0 #62: 再生負荷に応じた適応解像度プロキシ自動切替（EMA ヒステリシス 1/1→1/2→1/4・停止時フル解像度復帰）および複雑度ホットスポット優先スマート事前レンダリング計画を管理する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["status", "configure", "plan_prerender", "simulate_adaptive"] },
                    "enabled": { "type": "boolean" },
                    "target_fps": { "type": "number" },
                    "min_scale_divisor": { "type": "integer" },
                    "max_scale_divisor": { "type": "integer" },
                    "drop_threshold_ratio": { "type": "number" },
                    "recover_threshold_ratio": { "type": "number" },
                    "snap_full_res_on_pause": { "type": "boolean" },
                    "smart_prerender_enabled": { "type": "boolean" },
                    "prerender_budget_frames": { "type": "integer" },
                    "sample_frame_ms_list": { "type": "array", "items": { "type": "number" } }
                }
            }
        }),
        json!({
            "name": "audit_and_compact_project_memory",
            "description": "v11.0 #63: 大規模プロジェクトの参照監査・Undo Arc 共有率計測・SHA-256 整合性検証付き高速バイナリスナップショット（cache/project.fastbin）保存/読込・未使用アセット＆フレームキャッシュ一括コンパクションを実行する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["audit", "save_fastbin", "load_fastbin", "compact", "layer_diff"] },
                    "remove_unused_assets": { "type": "boolean" },
                    "purge_frame_cache": { "type": "boolean" },
                    "write_fastbin": { "type": "boolean" }
                }
            }
        }),
        json!({
            "name": "profile_timeline_bottlenecks",
            "description": "v11.0 #64: パス別マイクロ秒フレーム計測・プロファイラ HUD 設定・自動ボトルネック診断・安全な 1-Undo 非破壊パフォーマンス最適化バッチ適用を実行する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["profile_frame", "configure_hud", "diagnose", "auto_optimize"] },
                    "time_sec": { "type": "number" },
                    "target_width": { "type": "integer" },
                    "target_height": { "type": "integer" },
                    "adaptive_divisor": { "type": "integer" },
                    "hud_enabled": { "type": "boolean" },
                    "show_frame_graph": { "type": "boolean" },
                    "show_cache_stats": { "type": "boolean" },
                    "warn_frame_ms": { "type": "number" }
                }
            }
        }),
    ]
}

pub(crate) fn try_call(
    guard: &mut EditorSession,
    _fps: FrameRate,
    name: &str,
    args: &Value,
) -> Option<Result<Value, String>> {
    match name {
        "configure_gpu_render_graph" => Some(call_gpu_render_graph(guard, args)),
        "manage_adaptive_playback_and_prerender" => Some(call_adaptive_prerender(guard, args)),
        "audit_and_compact_project_memory" => Some(call_memory_compact(guard, args)),
        "profile_timeline_bottlenecks" => Some(call_profile_bottlenecks(guard, args)),
        _ => None,
    }
}

fn call_gpu_render_graph(guard: &mut EditorSession, args: &Value) -> Result<Value, String> {
    let action = args.get("action").and_then(Value::as_str).unwrap_or("plan");
    let mut cfg: GpuRenderGraphConfig = guard
        .project
        .active_scene()
        .ok_or("no active scene")?
        .gpu_graph
        .clone();

    if action == "configure" {
        if let Some(v) = args.get("enabled").and_then(Value::as_bool) {
            cfg.enabled = v;
        }
        if let Some(v) = args.get("prefer_zero_copy_vram").and_then(Value::as_bool) {
            cfg.prefer_zero_copy_vram = v;
        }
        if let Some(v) = args.get("max_vram_textures_mib").and_then(Value::as_u64) {
            cfg.max_vram_textures_mib = (v as u32).clamp(64, 16_384);
        }
        if let Some(v) = args.get("allow_8k_tiling").and_then(Value::as_bool) {
            cfg.allow_8k_tiling = v;
        }
        if let Some(v) = args.get("chain_post_fx_on_gpu").and_then(Value::as_bool) {
            cfg.chain_post_fx_on_gpu = v;
        }
        if let Some(v) = args.get("show_gpu_direct_badge").and_then(Value::as_bool) {
            cfg.show_gpu_direct_badge = v;
        }
        guard
            .bus
            .execute(
                &mut guard.project,
                EditCommand::SetGpuRenderGraph {
                    gpu_graph: cfg.clone(),
                },
                CommandSource::Mcp,
                None,
            )
            .map_err(|e| e.to_string())?;
    }

    let scene = guard.project.active_scene().ok_or("no active scene")?;
    let tw = args
        .get("target_width")
        .and_then(Value::as_u64)
        .map(|v| (v as u32).clamp(64, 7680))
        .unwrap_or(scene.width);
    let th = args
        .get("target_height")
        .and_then(Value::as_u64)
        .map(|v| (v as u32).clamp(64, 4320))
        .unwrap_or(scene.height);
    let plan = build_gpu_render_graph_plan(scene, tw, th);
    let shader_checks = validate_all_builtin_wgsl_shaders();

    let benchmark_val = if action == "benchmark" {
        let bw = tw.min(1920);
        let bh = th.min(1080);
        let stats = benchmark_v110_zero_copy_graph(bw, bh);
        Some(json!({
            "width": stats.width,
            "height": stats.height,
            "used_gpu": stats.used_gpu,
            "adapter_name": stats.adapter_name,
            "backend_name": stats.backend_name,
            "chained_passes": stats.chained_passes,
            "intermediate_cpu_readbacks": stats.intermediate_cpu_readbacks,
            "elapsed_ms": stats.elapsed_ms,
            "validated_shader_count": stats.validated_shader_count,
            "all_shaders_valid": stats.all_shaders_valid
        }))
    } else {
        None
    };

    let text = serde_json::to_string_pretty(&json!({
        "action": action,
        "gpu_graph": scene.gpu_graph,
        "execution_plan": plan,
        "builtin_wgsl_shader_count": shader_checks.len(),
        "all_wgsl_shaders_valid": shader_checks.iter().all(|c| c.valid),
        "benchmark": benchmark_val
    }))
    .unwrap_or_default();
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

fn call_adaptive_prerender(guard: &mut EditorSession, args: &Value) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("plan_prerender");
    let mut cfg: AdaptivePlaybackConfig = guard.project.adaptive_playback.clone();

    if action == "configure" {
        if let Some(v) = args.get("enabled").and_then(Value::as_bool) {
            cfg.enabled = v;
        }
        cfg.target_fps = finite_clamped(
            args.get("target_fps"),
            f64::from(cfg.target_fps),
            15.0,
            240.0,
        ) as f32;
        if let Some(v) = args.get("min_scale_divisor").and_then(Value::as_u64) {
            cfg.min_scale_divisor = (v as u32).clamp(1, 4);
        }
        if let Some(v) = args.get("max_scale_divisor").and_then(Value::as_u64) {
            cfg.max_scale_divisor = (v as u32).clamp(cfg.min_scale_divisor, 4);
        }
        cfg.drop_threshold_ratio = finite_clamped(
            args.get("drop_threshold_ratio"),
            f64::from(cfg.drop_threshold_ratio),
            1.0,
            3.0,
        ) as f32;
        cfg.recover_threshold_ratio = finite_clamped(
            args.get("recover_threshold_ratio"),
            f64::from(cfg.recover_threshold_ratio),
            0.2,
            0.95,
        ) as f32;
        if let Some(v) = args.get("snap_full_res_on_pause").and_then(Value::as_bool) {
            cfg.snap_full_res_on_pause = v;
        }
        if let Some(v) = args.get("smart_prerender_enabled").and_then(Value::as_bool) {
            cfg.smart_prerender_enabled = v;
        }
        if let Some(v) = args.get("prerender_budget_frames").and_then(Value::as_u64) {
            cfg.prerender_budget_frames = (v as usize).clamp(1, 2000);
        }
        guard
            .bus
            .execute(
                &mut guard.project,
                EditCommand::SetAdaptivePlayback {
                    adaptive_playback: cfg.clone(),
                },
                CommandSource::Mcp,
                None,
            )
            .map_err(|e| e.to_string())?;
    }

    let scene = guard.project.active_scene().ok_or("no active scene")?;
    let plan = plan_smart_prerender_schedule(scene, &guard.project.adaptive_playback);

    let simulation = if action == "simulate_adaptive" {
        let samples: Vec<f32> = args
            .get("sample_frame_ms_list")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_f64)
                    .filter(|v| v.is_finite())
                    .map(|v| v.clamp(0.0, 2000.0) as f32)
                    .collect()
            })
            .filter(|v: &Vec<f32>| !v.is_empty())
            .unwrap_or_else(|| vec![28.0, 30.0, 32.0, 30.0, 8.0, 8.0, 7.0, 7.0, 6.0, 6.0]);
        let mut st = AdaptiveScaleState::default();
        let mut divisors = Vec::with_capacity(samples.len());
        for ms in &samples {
            divisors.push(st.record_frame_time(*ms, true, &guard.project.adaptive_playback));
        }
        let pause_divisor = st.record_frame_time(25.0, false, &guard.project.adaptive_playback);
        Some(json!({
            "input_frame_ms": samples,
            "playing_divisors": divisors,
            "pause_restored_divisor": pause_divisor,
            "total_downscales": st.total_downscales,
            "total_upscales": st.total_upscales
        }))
    } else {
        None
    };

    let text = serde_json::to_string_pretty(&json!({
        "action": action,
        "adaptive_playback": guard.project.adaptive_playback,
        "prerender_plan": plan,
        "simulation": simulation
    }))
    .unwrap_or_default();
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

fn call_memory_compact(guard: &mut EditorSession, args: &Value) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("audit");

    let text = match action {
        "save_fastbin" => {
            if let Some(folder) = guard.folder.as_ref() {
                let (path, sz) = save_project_fastbin_to_folder(folder, &guard.project)?;
                serde_json::to_string_pretty(&json!({
                    "action": "save_fastbin",
                    "saved_to_disk": true,
                    "path": path,
                    "fastbin_bytes": sz,
                    "sha256_verified": true
                }))
                .unwrap_or_default()
            } else {
                let bytes = encode_project_fastbin(&guard.project)?;
                let verified = decode_project_fastbin(&bytes)? == guard.project;
                serde_json::to_string_pretty(&json!({
                    "action": "save_fastbin",
                    "saved_to_disk": false,
                    "fastbin_bytes": bytes.len(),
                    "sha256_verified": verified
                }))
                .unwrap_or_default()
            }
        }
        "load_fastbin" => {
            let (loaded, from_disk) = if let Some(folder) = guard.folder.as_ref() {
                match load_project_fastbin_from_folder(folder) {
                    Ok(p) => (p, true),
                    Err(_) => {
                        let bytes = encode_project_fastbin(&guard.project)?;
                        (decode_project_fastbin(&bytes)?, false)
                    }
                }
            } else {
                let bytes = encode_project_fastbin(&guard.project)?;
                (decode_project_fastbin(&bytes)?, false)
            };
            serde_json::to_string_pretty(&json!({
                "action": "load_fastbin",
                "from_disk": from_disk,
                "sha256_verified": true,
                "matches_active_project": loaded == guard.project,
                "scene_count": loaded.scenes.len()
            }))
            .unwrap_or_default()
        }
        "compact" => {
            let remove_unused = args
                .get("remove_unused_assets")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let purge_cache = args
                .get("purge_frame_cache")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let write_fastbin = args
                .get("write_fastbin")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let (cmd_opt, report) = compact_project_and_caches(
                &guard.project,
                guard.folder.as_ref(),
                remove_unused,
                purge_cache,
                write_fastbin,
            );
            let mut applied_undoable = false;
            if let Some(cmd) = cmd_opt {
                guard
                    .bus
                    .execute(&mut guard.project, cmd, CommandSource::Mcp, None)
                    .map_err(|e| e.to_string())?;
                applied_undoable = true;
            }
            let after_audit = audit_project_memory(
                &guard.project,
                guard.folder.as_ref(),
                guard.bus.undo_len(),
                guard.bus.unique_snapshot_count(),
            );
            serde_json::to_string_pretty(&json!({
                "action": "compact",
                "applied_undoable_command": applied_undoable,
                "compaction_report": report,
                "audit_after": after_audit
            }))
            .unwrap_or_default()
        }
        "layer_diff" => {
            let scene = guard.project.active_scene().ok_or("no active scene")?;
            let diff = compute_structural_layer_diff(scene, scene);
            serde_json::to_string_pretty(&json!({
                "action": "layer_diff",
                "scene_id": scene.id,
                "structural_diff": diff
            }))
            .unwrap_or_default()
        }
        _ => {
            let audit = audit_project_memory(
                &guard.project,
                guard.folder.as_ref(),
                guard.bus.undo_len(),
                guard.bus.unique_snapshot_count(),
            );
            serde_json::to_string_pretty(&json!({
                "action": "audit",
                "audit": audit
            }))
            .unwrap_or_default()
        }
    };

    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

fn call_profile_bottlenecks(guard: &mut EditorSession, args: &Value) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("diagnose");

    if action == "configure_hud" {
        let mut cfg: ProfilerHudConfig = guard.project.profiler_hud.clone();
        if let Some(v) = args.get("hud_enabled").and_then(Value::as_bool) {
            cfg.enabled = v;
        }
        if let Some(v) = args.get("show_frame_graph").and_then(Value::as_bool) {
            cfg.show_frame_graph = v;
        }
        if let Some(v) = args.get("show_cache_stats").and_then(Value::as_bool) {
            cfg.show_cache_stats = v;
        }
        cfg.warn_frame_ms = finite_clamped(
            args.get("warn_frame_ms"),
            f64::from(cfg.warn_frame_ms),
            4.0,
            250.0,
        ) as f32;
        guard
            .bus
            .execute(
                &mut guard.project,
                EditCommand::SetProfilerHud { profiler_hud: cfg },
                CommandSource::Mcp,
                None,
            )
            .map_err(|e| e.to_string())?;
    }

    let mut auto_optimized = false;
    if action == "auto_optimize"
        && let Some(cmd) =
            build_auto_optimize_scene_commands(&guard.project, guard.project.active_scene_id)
    {
        guard
            .bus
            .execute(&mut guard.project, cmd, CommandSource::Mcp, None)
            .map_err(|e| e.to_string())?;
        auto_optimized = true;
    }

    let scene = guard.project.active_scene().ok_or("no active scene")?;
    let tw = args
        .get("target_width")
        .and_then(Value::as_u64)
        .map(|v| (v as u32).clamp(64, 1920))
        .unwrap_or_else(|| scene.width.min(640));
    let th = args
        .get("target_height")
        .and_then(Value::as_u64)
        .map(|v| (v as u32).clamp(64, 1080))
        .unwrap_or_else(|| scene.height.min(360));
    let div = args
        .get("adaptive_divisor")
        .and_then(Value::as_u64)
        .map(|v| (v as u32).clamp(1, 4))
        .unwrap_or(1);
    let sample_time = args
        .get("time_sec")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(RationalTime::from_seconds)
        .unwrap_or(guard.playhead);

    let mut rg = RenderGraph::new();
    rg.set_project_folder(guard.folder.clone());
    rg.active_revision = guard.bus.revision();
    let profile_report = rg
        .render_project_scene_profiled(
            &guard.project,
            guard.project.active_scene_id,
            sample_time,
            tw,
            th,
            false,
            div,
        )
        .map(|(_, rep)| rep);

    let recommendations = diagnose_timeline_bottlenecks(
        &guard.project,
        guard.project.active_scene_id,
        profile_report.as_ref(),
    );
    let has_auto_optimize =
        build_auto_optimize_scene_commands(&guard.project, guard.project.active_scene_id).is_some();

    let text = serde_json::to_string_pretty(&json!({
        "action": action,
        "profiler_hud": guard.project.profiler_hud,
        "auto_optimized": auto_optimized,
        "has_pending_auto_optimize": has_auto_optimize,
        "frame_profile": profile_report,
        "recommendation_count": recommendations.len(),
        "recommendations": recommendations
    }))
    .unwrap_or_default();
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}
