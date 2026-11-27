//! v11.0 #61〜#64 Zero-Copy GPU Engine, Real-Time 8K & Native Studio Performance スタジオパネル
//!
//! 既存の 6 スタジオ Hub 内サブタブとしてトップバー肥大化ゼロ・機能ゼロ欠落で統合される:
//! - VFX & Motion Hub:
//!   - `ui_gpu_render_graph_tab` (#61 完全ゼロコピー GPU レンダリンググラフ＆全 9 WGSL Naga 検証)
//! - Edit Hub:
//!   - `ui_adaptive_prerender_tab` (#62 適応解像度 8K 再生＆複雑度ホットスポット事前レンダリング)
//! - Deliver Hub:
//!   - `ui_memory_fastbin_tab` (#63 FastBin SHA-256 スナップショット・Undo Arc 共有率監査＆コンパクション)
//!   - `ui_profiler_hud_tab` (#64 リアルタイムプロファイラ HUD・ローリング p95/p99 グラフ＆自動ボトルネック診断)

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Stroke, Ui, Vec2};
use ve_core::{
    AdaptiveScaleState, BottleneckRecommendation, BottleneckSeverity, CommandSource, EditCommand,
    ProjectCompactionReport, ProjectMemoryAudit, RollingProfilerHistory, SmartPrerenderPlan,
    audit_project_memory, build_auto_optimize_scene_commands, build_gpu_render_graph_plan,
    compact_project_and_caches, decode_project_fastbin, diagnose_timeline_bottlenecks,
    encode_project_fastbin, load_project_fastbin_from_folder, plan_smart_prerender_schedule,
    save_project_fastbin_to_folder,
};
use ve_mcp::SharedEditorSession;
use ve_render::{
    GpuZeroCopyExecutionStats, RenderGraph, benchmark_v110_zero_copy_graph,
    validate_all_builtin_wgsl_shaders,
};

/// v11.0 #61〜#64 ランタイム状態および Hub パネル状態
pub struct V110PanelsState {
    // --- #61 ゼロコピー GPU レンダリンググラフ ---
    gpu_bench_result: Option<GpuZeroCopyExecutionStats>,
    gpu_status: String,

    // --- #62 適応解像度＆スマート事前レンダリング ---
    pub adaptive_scale: AdaptiveScaleState,
    prerender_plan: Option<SmartPrerenderPlan>,
    adaptive_status: String,

    // --- #63 メモリ監査・FastBin・コンパクション ---
    last_audit: Option<ProjectMemoryAudit>,
    last_compaction: Option<ProjectCompactionReport>,
    memory_status: String,

    // --- #64 プロファイラ HUD ＆ ボトルネック診断 ---
    pub profiler_history: RollingProfilerHistory,
    last_recommendations: Vec<BottleneckRecommendation>,
    profiler_status: String,
}

impl Default for V110PanelsState {
    fn default() -> Self {
        Self {
            gpu_bench_result: None,
            gpu_status: "VRAM 常駐 Ping-Pong WGSL Compute 連結グラフ（全 9 本組み込み WGSL・決定論 CPU フォールバック）".into(),
            adaptive_scale: AdaptiveScaleState::default(),
            prerender_plan: None,
            adaptive_status: "EMA ヒステリシス適応解像度（1/1→1/2→1/4・停止時 1/1 即復帰）＆複雑度ホットスポット事前レンダリング".into(),
            last_audit: None,
            last_compaction: None,
            memory_status: "FIPS 180-4 SHA-256 検証付き FastBin スナップショット・Undo Arc 共有率計測・1-Undo 未使用アセット整理".into(),
            profiler_history: RollingProfilerHistory::new(120),
            last_recommendations: Vec::new(),
            profiler_status: "パス別マイクロ秒計測・ローリング p95/p99 統計・ボトルネック自動診断＆1-Undo 最適化".into(),
        }
    }
}

impl V110PanelsState {
    // =========================================================================
    // #61: 完全ゼロコピー GPU レンダリンググラフ
    // =========================================================================
    pub fn ui_gpu_render_graph_tab(&mut self, ui: &mut Ui, session: &SharedEditorSession) -> bool {
        let mut dirty = false;
        ui.heading("⚡ 完全ゼロコピー GPU レンダリンググラフ (#61)");
        ui.label(
            RichText::new(
                "VRAM 常駐 Ping-Pong バッファ上で合成→カラー→360° パスを 1 コマンドバッファ連結実行（中間 CPU リードバックゼロ）",
            )
            .weak(),
        );
        ui.add_space(4.0);

        let (mut cfg, plan, scene_dims) = {
            let g = session.lock().unwrap();
            let sc = g.project.active_scene();
            let c = sc.map(|s| s.gpu_graph.clone()).unwrap_or_default();
            let dims = sc.map(|s| (s.width, s.height)).unwrap_or((1920, 1080));
            let p = sc.map(|s| build_gpu_render_graph_plan(s, s.width, s.height));
            (c, p, dims)
        };
        let before = cfg.clone();

        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut cfg.enabled, "GPU レンダリンググラフを有効化");
                ui.checkbox(&mut cfg.prefer_zero_copy_vram, "VRAM ゼロコピー連結を優先");
                ui.checkbox(&mut cfg.chain_post_fx_on_gpu, "ポストエフェクト連続連結");
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut cfg.allow_8k_tiling, "8K ストリップタイリング許可");
                ui.checkbox(
                    &mut cfg.show_gpu_direct_badge,
                    "HUD に GPU DIRECT バッジを表示",
                );
            });
            ui.horizontal(|ui| {
                ui.label("VRAM テクスチャ上限:");
                ui.add(egui::Slider::new(&mut cfg.max_vram_textures_mib, 64..=8192).suffix(" MiB"));
            });
        });

        if cfg != before {
            let mut g = session.lock().unwrap();
            let s = &mut *g;
            if s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetGpuRenderGraph { gpu_graph: cfg },
                    CommandSource::Gui,
                    Some("v110_gpu_graph"),
                )
                .is_ok()
            {
                dirty = true;
            }
        }

        if let Some(plan) = plan {
            ui.group(|ui| {
                ui.strong(format!(
                    "📊 現在の実行 DAG 計画 ({}×{} | ゼロコピー連結 {} パス | PCIe 削減 {:.1} MiB/frame)",
                    plan.width,
                    plan.height,
                    plan.zero_copy_chain_len,
                    plan.estimated_pcie_bytes_saved_per_frame as f64 / (1024.0 * 1024.0)
                ));
                ui.label(format!(
                    "推定 VRAM 使用量: {:.1} MiB / 予算 {:.0} MiB | 削減リードバック: {} 回 | 8K タイリング: {}",
                    plan.total_estimated_vram_bytes as f64 / (1024.0 * 1024.0),
                    plan.vram_budget_bytes as f64 / (1024.0 * 1024.0),
                    plan.cpu_readbacks_saved,
                    if plan.requires_8k_tiling { "有効" } else { "不要" }
                ));
                for node in &plan.nodes {
                    let badge = if node.zero_copy_eligible {
                        "⚡ GPU ZERO-COPY"
                    } else {
                        "🖥 CPU FALLBACK"
                    };
                    let col = if node.zero_copy_eligible {
                        Color32::from_rgb(80, 210, 255)
                    } else {
                        Color32::LIGHT_GRAY
                    };
                    ui.horizontal(|ui| {
                        ui.colored_label(col, badge);
                        ui.monospace(format!(
                            "#{} {} ({:.1} MiB)",
                            node.pass_id,
                            node.label,
                            node.estimated_vram_bytes as f64 / (1024.0 * 1024.0)
                        ));
                    });
                }
            });
        }

        static SHADER_CHECKS: std::sync::OnceLock<Vec<ve_render::WgslValidationSummary>> =
            std::sync::OnceLock::new();
        let checks = SHADER_CHECKS.get_or_init(validate_all_builtin_wgsl_shaders);
        let all_ok = checks.iter().all(|c| c.valid);
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                if all_ok {
                    Color32::LIGHT_GREEN
                } else {
                    Color32::LIGHT_RED
                },
                format!(
                    "全 {} 本 組み込み WGSL シェーダー Naga 検証: {}",
                    checks.len(),
                    if all_ok { "ALL PASS" } else { "ERROR" }
                ),
            );
            if ui.button("🚀 ゼロコピー連結ベンチマーク実行").clicked() {
                let bw = scene_dims.0.clamp(320, 1280);
                let bh = scene_dims.1.clamp(180, 720);
                let rep = benchmark_v110_zero_copy_graph(bw, bh);
                self.gpu_status = format!(
                    "完了: {}×{} | {} ({}) | {}パス連結・中間リードバック {}回 | {:.2} ms",
                    rep.width,
                    rep.height,
                    rep.adapter_name,
                    rep.backend_name,
                    rep.chained_passes,
                    rep.intermediate_cpu_readbacks,
                    rep.elapsed_ms
                );
                self.gpu_bench_result = Some(rep);
            }
        });
        if let Some(ref rep) = self.gpu_bench_result {
            ui.monospace(format!(
                "GPU={} | {}×{} | Passes={} | Readbacks={} | Shaders={}/{} OK | {:.2} ms",
                rep.used_gpu,
                rep.width,
                rep.height,
                rep.chained_passes,
                rep.intermediate_cpu_readbacks,
                rep.validated_shader_count,
                rep.validated_shader_count,
                rep.elapsed_ms
            ));
        }
        ui.weak(&self.gpu_status);
        dirty
    }

    // =========================================================================
    // #62: 適応解像度プロキシ自動切替 ＆ スマート事前レンダリングスケジューラ
    // =========================================================================
    pub fn ui_adaptive_prerender_tab(
        &mut self,
        ui: &mut Ui,
        session: &SharedEditorSession,
    ) -> bool {
        let mut dirty = false;
        ui.heading("🎯 適応解像度 8K 再生 ＆ スマート事前レンダリング (#62)");
        ui.label(
            RichText::new(
                "再生中のみ EMA ヒステリシスで 1/1→1/2→1/4 へ自動スケールし停止時に 1/1 即復帰。重負荷区間から優先キャッシュ。",
            )
            .weak(),
        );
        ui.add_space(4.0);

        // ライブ適応解像度ステータスカード
        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                let div = self.adaptive_scale.current_divisor.clamp(1, 4);
                let col = match div {
                    1 => Color32::LIGHT_GREEN,
                    2 => Color32::from_rgb(255, 200, 70),
                    _ => Color32::from_rgb(255, 110, 90),
                };
                ui.colored_label(
                    col,
                    RichText::new(format!("現在スケール: 1/{div}")).strong(),
                );
                ui.monospace(format!(
                    "EMA: {:.2} ms | Downscales: {} | Upscales: {}",
                    self.adaptive_scale.ema_frame_ms,
                    self.adaptive_scale.total_downscales,
                    self.adaptive_scale.total_upscales
                ));
                if ui.small_button("🔄 1/1 リセット").clicked() {
                    self.adaptive_scale = AdaptiveScaleState::default();
                    dirty = true;
                }
            });
        });

        let mut cfg = {
            let g = session.lock().unwrap();
            g.project.adaptive_playback.clone()
        };
        let before = cfg.clone();

        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut cfg.enabled, "適応解像度スケーリングを有効化");
                ui.checkbox(
                    &mut cfg.snap_full_res_on_pause,
                    "停止時に 1/1 フル解像度へ即復帰",
                );
                ui.checkbox(
                    &mut cfg.smart_prerender_enabled,
                    "スマート事前レンダリング優先キュー",
                );
            });
            egui::Grid::new("v110_adaptive_grid")
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label("目標再生 FPS:");
                    ui.add(egui::Slider::new(&mut cfg.target_fps, 24.0..=120.0).suffix(" fps"));
                    ui.end_row();
                    ui.label("最大縮小倍率 (1/N):");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut cfg.max_scale_divisor, 1, "1/1 (固定)");
                        ui.selectable_value(&mut cfg.max_scale_divisor, 2, "最大 1/2");
                        ui.selectable_value(&mut cfg.max_scale_divisor, 4, "最大 1/4");
                    });
                    ui.end_row();
                    ui.label("縮小トリガー閾値倍率:");
                    ui.add(
                        egui::Slider::new(&mut cfg.drop_threshold_ratio, 1.05..=2.0).suffix("x"),
                    );
                    ui.end_row();
                    ui.label("復帰トリガー閾値倍率:");
                    ui.add(
                        egui::Slider::new(&mut cfg.recover_threshold_ratio, 0.35..=0.90)
                            .suffix("x"),
                    );
                    ui.end_row();
                    ui.label("事前レンダリング予算:");
                    let mut budget = cfg.prerender_budget_frames as u32;
                    if ui
                        .add(egui::Slider::new(&mut budget, 16..=600).suffix(" frames"))
                        .changed()
                    {
                        cfg.prerender_budget_frames = budget as usize;
                    }
                    ui.end_row();
                });
        });

        if cfg != before {
            let mut g = session.lock().unwrap();
            let s = &mut *g;
            if s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetAdaptivePlayback {
                        adaptive_playback: cfg.clone(),
                    },
                    CommandSource::Gui,
                    Some("v110_adaptive"),
                )
                .is_ok()
            {
                dirty = true;
            }
        }

        ui.horizontal(|ui| {
            if ui
                .button("🧠 タイムライン複雑度ホットスポット解析")
                .clicked()
            {
                let g = session.lock().unwrap();
                if let Some(sc) = g.project.active_scene() {
                    let plan = plan_smart_prerender_schedule(sc, &g.project.adaptive_playback);
                    self.adaptive_status = format!(
                        "解析完了: 全 {} フレーム中 {} 区間の高負荷ホットスポット・優先キュー {} フレーム",
                        plan.total_frames,
                        plan.hotspots.len(),
                        plan.prioritized_frames.len()
                    );
                    self.prerender_plan = Some(plan);
                }
            }
        });

        if let Some(ref plan) = self.prerender_plan {
            ui.group(|ui| {
                ui.strong(format!(
                    "🔥 高負荷ホットスポット上位（全 {} 区間 / キュー {} フレーム）",
                    plan.hotspots.len(),
                    plan.prioritized_frames.len()
                ));
                if plan.hotspots.is_empty() {
                    ui.weak("高負荷区間は検出されませんでした（リアルタイム余裕あり）");
                } else {
                    for seg in plan.hotspots.iter().take(8) {
                        ui.monospace(format!(
                            "[{:5.1}s..{:5.1}s] score={:4.1} | {}",
                            seg.start_sec,
                            seg.end_sec,
                            seg.complexity_score,
                            seg.reasons.join(", ")
                        ));
                    }
                }
            });
        }
        ui.weak(&self.adaptive_status);
        dirty
    }

    // =========================================================================
    // #63: 大規模プロジェクト超軽量化（FastBin・Undo Arc 共有率・コンパクション）
    // =========================================================================
    pub fn ui_memory_fastbin_tab(
        &mut self,
        ui: &mut Ui,
        session: &SharedEditorSession,
        render_graph: &mut RenderGraph,
    ) -> bool {
        let mut dirty = false;
        ui.heading("💾 超軽量プロジェクト監査 ＆ FastBin スナップショット (#63)");
        ui.label(
            RichText::new(
                "SHA-256 整合性検証付きバイナリスナップショット（cache/project.fastbin）・Undo Arc 共有率監査・未使用アセット 1-Undo 整理",
            )
            .weak(),
        );
        ui.add_space(4.0);

        ui.horizontal_wrapped(|ui| {
            if ui.button("🔍 メモリ＆Undo 監査を実行").clicked() {
                let g = session.lock().unwrap();
                let audit = audit_project_memory(
                    &g.project,
                    g.folder.as_ref(),
                    g.bus.undo_len(),
                    g.bus.unique_snapshot_count(),
                );
                self.memory_status = format!(
                    "監査完了: JSON {:.1} KiB → FastBin {:.1} KiB ({:.1}%) | Undo 共有節約 {:.1}%",
                    audit.json_bytes as f64 / 1024.0,
                    audit.fastbin_bytes as f64 / 1024.0,
                    audit.fastbin_ratio_pct,
                    audit.undo_memory_saved_pct
                );
                self.last_audit = Some(audit);
            }
            if ui.button("💾 FastBin 保存＆SHA-256 検証").clicked() {
                let g = session.lock().unwrap();
                if let Some(ref folder) = g.folder {
                    match save_project_fastbin_to_folder(folder, &g.project) {
                        Ok((path, sz)) => match load_project_fastbin_from_folder(folder) {
                            Ok(restored) if restored == g.project => {
                                self.memory_status = format!(
                                    "✅ FastBin 保存＆SHA-256 復元検証成功: {} ({:.1} KiB)",
                                    path.display(),
                                    sz as f64 / 1024.0
                                );
                            }
                            Ok(_) => {
                                self.memory_status =
                                    "⚠ FastBin 復元後のプロジェクト差分を検出".into();
                            }
                            Err(e) => {
                                self.memory_status = format!("❌ FastBin 復元失敗: {e}");
                            }
                        },
                        Err(e) => {
                            self.memory_status = format!("❌ FastBin 保存失敗: {e}");
                        }
                    }
                } else {
                    match encode_project_fastbin(&g.project)
                        .and_then(|b| decode_project_fastbin(&b).map(|p| (b.len(), p == g.project)))
                    {
                        Ok((sz, eq)) => {
                            self.memory_status = format!(
                                "✅ インメモリ FastBin SHA-256 ラウンドトリップ検証完了 ({:.1} KiB, 一致={eq})",
                                sz as f64 / 1024.0
                            );
                        }
                        Err(e) => {
                            self.memory_status = format!("❌ FastBin 検証エラー: {e}");
                        }
                    }
                }
            }
            if ui
                .button("🧹 未使用アセット整理＆コンパクション (1-Undo)")
                .clicked()
            {
                let mut g = session.lock().unwrap();
                let s = &mut *g;
                let (cmd_opt, rep) =
                    compact_project_and_caches(&s.project, s.folder.as_ref(), true, false, true);
                if let Some(cmd) = cmd_opt
                    && s.bus
                        .execute(&mut s.project, cmd, CommandSource::Gui, None)
                        .is_ok()
                {
                    dirty = true;
                }
                self.memory_status = format!(
                    "コンパクション完了: 未使用アセット {} 件削除 / キャッシュ {} 件削除",
                    rep.removed_unused_assets, rep.purged_cache_files
                );
                self.last_compaction = Some(rep);
                self.last_audit = Some(audit_project_memory(
                    &s.project,
                    s.folder.as_ref(),
                    s.bus.undo_len(),
                    s.bus.unique_snapshot_count(),
                ));
            }
            if ui.button("🗑 レンダーキャッシュ全解放").clicked() {
                render_graph.clear_media_caches();
                dirty = true;
                self.memory_status = "RenderGraph のメディア・テキスト・LUT・フレームプールキャッシュを解放しました".into();
            }
        });

        let pool = render_graph.frame_pool.metrics(0);
        ui.group(|ui| {
            ui.strong("📦 ライブ RenderGraph キャッシュ使用量");
            ui.monospace(format!(
                "Text BBox Cache: {:.1} KiB (hits={} / misses={}) | 3D LUT Arc Cache: {} entries",
                render_graph.text_renderer().text_cache_bytes() as f64 / 1024.0,
                render_graph.text_renderer().text_cache_hits(),
                render_graph.text_renderer().text_cache_misses(),
                render_graph.lut_cache_len()
            ));
            ui.monospace(format!(
                "Frame Pool: {:.1} MiB (reuses={}) | Precomp: {} | VideoFrames: {}",
                pool.resident_bytes as f64 / (1024.0 * 1024.0),
                pool.pool_reuses,
                render_graph.mem_precomp_cache_len(),
                render_graph.video_frame_cache_len()
            ));
        });

        if let Some(ref audit) = self.last_audit {
            ui.group(|ui| {
                ui.strong("🧾 プロジェクト軽量化監査レポート");
                ui.monospace(format!(
                    "Scenes={} | Layers={} | Items={} | Keyframes={}",
                    audit.scene_count,
                    audit.total_layers,
                    audit.total_items,
                    audit.total_keyframes
                ));
                ui.monospace(format!(
                    "Assets={} (未参照: {} 件) | Disk cache: {:.2} MiB",
                    audit.asset_count,
                    audit.unreferenced_asset_ids.len(),
                    audit.disk_cache_bytes as f64 / (1024.0 * 1024.0)
                ));
                ui.monospace(format!(
                    "JSON: {:.1} KiB | FastBin: {:.1} KiB ({:.1}%) | Undo Steps: {} (一意 Arc: {} / 節約 {:.1}%)",
                    audit.json_bytes as f64 / 1024.0,
                    audit.fastbin_bytes as f64 / 1024.0,
                    audit.fastbin_ratio_pct,
                    audit.undo_steps,
                    audit.unique_undo_snapshots,
                    audit.undo_memory_saved_pct
                ));
            });
        }
        if let Some(ref rep) = self.last_compaction {
            ui.monospace(format!(
                "直近コンパクション: 削除アセット={} | キャッシュ削除={}件 ({:.1} KiB) | FastBin={:?} bytes",
                rep.removed_unused_assets,
                rep.purged_cache_files,
                rep.purged_cache_bytes as f64 / 1024.0,
                rep.fastbin_saved_bytes
            ));
        }
        ui.weak(&self.memory_status);
        dirty
    }

    // =========================================================================
    // #64: リアルタイム・パフォーマンスプロファイラ HUD ＆ ボトルネック自動診断
    // =========================================================================
    pub fn ui_profiler_hud_tab(
        &mut self,
        ui: &mut Ui,
        session: &SharedEditorSession,
        render_graph: &mut RenderGraph,
    ) -> bool {
        let mut dirty = false;
        ui.heading("📈 リアルタイムプロファイラ HUD ＆ 自動ボトルネック診断 (#64)");
        ui.label(
            RichText::new(
                "プレビュー左上ダークガラス HUD・直近 120 フレーム p95/p99 グラフ・パス別内訳・自動最適化提案",
            )
            .weak(),
        );
        ui.add_space(4.0);

        let mut cfg = {
            let g = session.lock().unwrap();
            g.project.profiler_hud.clone()
        };
        let before = cfg.clone();

        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut cfg.enabled, "📺 プレビューにプロファイラ HUD を表示");
                ui.checkbox(&mut cfg.show_frame_graph, "パス別フレームグラフバー");
                ui.checkbox(&mut cfg.show_cache_stats, "キャッシュ統計");
            });
            ui.horizontal(|ui| {
                ui.label("警告フレーム予算:");
                ui.add(egui::Slider::new(&mut cfg.warn_frame_ms, 8.0..=66.7).suffix(" ms"));
            });
        });

        if cfg != before {
            let mut g = session.lock().unwrap();
            let s = &mut *g;
            if s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetProfilerHud {
                        profiler_hud: cfg.clone(),
                    },
                    CommandSource::Gui,
                    Some("v110_profiler_hud"),
                )
                .is_ok()
            {
                dirty = true;
            }
        }

        // ローリング p95/p99 サマリー＆ミニフレームタイムグラフ
        let stats = self.profiler_history.summarize(cfg.warn_frame_ms);
        ui.group(|ui| {
            ui.strong(format!(
                "📊 ローリング統計 ({} frames) — Avg {:.2}ms | p95 {:.2}ms | p99 {:.2}ms | Est {:.1} fps | 予算超過 {:.1}%",
                stats.sample_count,
                stats.avg_ms,
                stats.p95_ms,
                stats.p99_ms,
                stats.fps_estimate,
                stats.over_budget_pct
            ));

            let desired = Vec2::new(ui.available_width().max(240.0), 64.0);
            let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 4.0, Color32::from_rgb(16, 20, 28));
            let max_ms = (cfg.warn_frame_ms * 2.0).max(33.4);
            let warn_y = rect.bottom() - (cfg.warn_frame_ms / max_ms).clamp(0.05, 0.95) * rect.height();
            painter.line_segment(
                [Pos2::new(rect.left(), warn_y), Pos2::new(rect.right(), warn_y)],
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 200, 60, 140)),
            );
            let samples = self.profiler_history.samples();
            if !samples.is_empty() {
                let bar_w = (rect.width() / samples.len().max(1) as f32).max(2.0);
                for (i, rep) in samples.iter().enumerate() {
                    let ms = rep.total_ms();
                    let h = ((ms / max_ms).clamp(0.02, 1.0)) * rect.height();
                    let x0 = rect.left() + i as f32 * bar_w;
                    let col = if ms <= cfg.warn_frame_ms {
                        Color32::from_rgb(60, 210, 110)
                    } else if ms <= cfg.warn_frame_ms * 1.5 {
                        Color32::from_rgb(245, 175, 45)
                    } else {
                        Color32::from_rgb(235, 65, 75)
                    };
                    painter.rect_filled(
                        Rect::from_min_max(
                            Pos2::new(x0, rect.bottom() - h),
                            Pos2::new((x0 + bar_w - 1.0).max(x0 + 1.0), rect.bottom()),
                        ),
                        0.0,
                        col,
                    );
                }
            }

            if let Some(latest) = self.profiler_history.latest() {
                ui.monospace(format!(
                    "最新フレーム: {}×{} (1/{}) | {:.2} ms | Layers={} Items={} | TextCache={}/{} ({:.0} KiB)",
                    latest.width,
                    latest.height,
                    latest.adaptive_divisor,
                    latest.total_ms(),
                    latest.active_layer_count,
                    latest.active_item_count,
                    latest.text_cache_hits,
                    latest.text_cache_hits + latest.text_cache_misses,
                    latest.text_cache_bytes as f64 / 1024.0
                ));
                ui.horizontal_wrapped(|ui| {
                    for p in &latest.pass_breakdown {
                        ui.monospace(format!("{}:{:.2}ms", p.pass_name, p.duration_us as f32 / 1000.0));
                    }
                });
            }
        });

        ui.horizontal_wrapped(|ui| {
            if ui.button("⏱ 1フレーム詳細計測").clicked() {
                let (proj, active_id, t) = {
                    let g = session.lock().unwrap();
                    (g.project.clone(), g.project.active_scene_id, g.playhead)
                };
                let (w, h) = proj
                    .active_scene()
                    .map(|s| (s.width.min(640), s.height.min(360)))
                    .unwrap_or((640, 360));
                if let Some((_, rep)) = render_graph.render_project_scene_profiled(
                    &proj,
                    active_id,
                    t,
                    w,
                    h,
                    false,
                    self.adaptive_scale.current_divisor,
                ) {
                    self.profiler_status = format!(
                        "実測完了: {:.2} ms ({} パス)",
                        rep.total_ms(),
                        rep.pass_breakdown.len()
                    );
                    self.profiler_history.push(rep);
                }
            }
            if ui.button("🩺 ボトルネック自動診断").clicked() {
                let g = session.lock().unwrap();
                self.last_recommendations = diagnose_timeline_bottlenecks(
                    &g.project,
                    g.project.active_scene_id,
                    self.profiler_history.latest(),
                );
                self.profiler_status = format!(
                    "診断完了: {} 件の最適化提案を検出",
                    self.last_recommendations.len()
                );
            }
            if ui.button("⚡ 1-Undo 安全自動最適化を適用").clicked() {
                let mut g = session.lock().unwrap();
                let s = &mut *g;
                if let Some(cmd) =
                    build_auto_optimize_scene_commands(&s.project, s.project.active_scene_id)
                {
                    if s.bus
                        .execute(&mut s.project, cmd, CommandSource::Gui, None)
                        .is_ok()
                    {
                        dirty = true;
                        self.profiler_status =
                            "✅ GPU ゼロコピー・適応解像度・プレビュー MB 最適化を 1-Undo で適用しました".into();
                        self.last_recommendations = diagnose_timeline_bottlenecks(
                            &s.project,
                            s.project.active_scene_id,
                            self.profiler_history.latest(),
                        );
                    }
                } else {
                    self.profiler_status = "すでに最適なパフォーマンス設定です ✨".into();
                }
            }
            if ui.small_button("🧹 履歴クリア").clicked() {
                self.profiler_history.clear();
            }
        });

        if !self.last_recommendations.is_empty() {
            ui.group(|ui| {
                ui.strong(format!(
                    "🩺 診断結果 ({} 件)",
                    self.last_recommendations.len()
                ));
                for rec in &self.last_recommendations {
                    let (icon, col) = match rec.severity {
                        BottleneckSeverity::Critical => ("🔴", Color32::LIGHT_RED),
                        BottleneckSeverity::Warning => ("🟡", Color32::from_rgb(255, 200, 70)),
                        BottleneckSeverity::Info => ("🔵", Color32::from_rgb(110, 200, 255)),
                    };
                    ui.colored_label(
                        col,
                        format!(
                            "{icon} {} (推定高速化 +{}%)",
                            rec.title, rec.estimated_speedup_pct
                        ),
                    );
                    ui.weak(&rec.detail);
                }
            });
        }
        ui.weak(&self.profiler_status);
        dirty
    }
}
