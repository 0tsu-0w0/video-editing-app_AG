# Changelog

## [10.1.0] - 2026-11-27

v10.0.0 公開後の**パフォーマンス最適化 ＆ メモリ軽量化ハードニング**を実施し、あわせて次期ロードマップ **v11.0〜v14.0（#61〜#76）** を `docs/future_features.md` へ策定しました（Pure Rust / ゼロ `unsafe` / `Cargo.toml`・`Cargo.lock` 無変更）。

### パフォーマンス最適化 ＆ メモリ軽量化
* **Undo/Redo メモリ約 50% 削減（`ve-core`）:** `CommandBus` の `UndoRecord` を `Arc<Project>` 化し、連続編集時の隣接スナップショット（`prev.after == cur.before`）を自動共有（200 段 Undo 時の保持スナップショット数を 400 → 201 へ約半減・`unique_snapshot_count()` 計測 API 追加）。
* **RAM プレビュー 50% 省メモリ ＆ 60fps 再生時ゼロクローン化（`ve-app` / `ve-render`）:**
  * `RamPreviewManager` のキャッシュ格納形式を 16 B/px `LinearImage` から 8 B/px `PackedRamFrame`（`Rgba16Float`）へ移行し、RAM 消費量を厳密に **50% 削減**（640×360×96 枚: 340 MiB → 170 MiB、1080p×96 枚: 3.1 GiB → 1.55 GiB）。
  * `WgpuPreviewPipeline::upload_rgba16f_process_and_present` を追加し、RAM キャッシュヒット時は f32↔f16 再変換ゼロで直接 GPU テクスチャへ転送。
  * `EditorApp::update` に `needs_target_update` / `try_update_center_frame_fast` を導入し、待機中・通常再生中の毎フレーム `Project` / `proxy_paths` / `WaveformPeaks` ディープクローンを完全排除。
  * ワーカーの `RenderGraph` に `active_revision` 同期（区間木 $O(\log N)$ 有効化）を追加し、`set_project_folder` / `register_proxy` の同一値再登録ガードにより毎フレーム `lut_cache` と常駐 `VideoStreamDecoder` が破棄されるボトルネックを解消。
* **テキスト描画 省メモリ BBox LRU キャッシュ ＆ 外接矩形縁取り最適化（`ve-render`）:**
  * `TextRenderer` に透過領域を切り落とした省メモリ BBox LRU キャッシュ（上限 64 エントリ / 16 MiB）を追加し、静的字幕および文字ごとアニメーション完了後の定常フレームを O(1) 再利用（ビット完全一致）。
  * `apply_text_style` の縁取り膨張およびドロップシャドウ抽出を `alpha_bounds` 外接矩形範囲のみへ限定し、キャッシュミス時も透過領域走査を完全スキップ。
* **RenderGraph・ピクセル合成・ブラー・回転・色調/スコープ高速化（`ve-render`）:**
  * `lut_cache` を `Arc<CubeLut3D>` 化し毎フレーム ~431 KB の 3D LUT クローンを排除、`ItemKind::VtuberAvatar` カスタム PNG を `png_cache` 経由へキャッシュ化、`render_project_scene_cached_rgba8` にリビジョン別シーン署名キャッシュを追加、`render_frame_batch_parallel` で区間木 $O(\log N)$ を有効化。
  * `LinearImage::blend_with_mode` / `apply_track_matte` のモード・不透明度分岐をループ外へホイスト、`to_rgba8_srgb` / `write_rgba16f_bytes` の `chunks_exact_mut` 直接書き込み化、`box_blur_separable` 中央領域のクランプ分岐排除、`rotate_linear_image` の行定数ホイスト＋境界内直接参照化、`apply_scene_color_grade` の中立ガンマ `powf(1.0)` スキップ、および `compute_video_scopes` の 8193 段 LUT 化を実施。
* **回帰テスト拡充:** `crates/render/tests/perf_lightweight_regression.rs` および `CommandBus` `Arc` 共有テスト・`RamPreviewManager` 8 B/px 検証を追加。

### 次期ロードマップ策定（`docs/future_features.md`）
* **v11.0〜v14.0（#61〜#76）** を追加（v11.0: ゼロコピー GPU グラフ＆適応 8K エンジン / v12.0: HDR 4000nit＆パワーウィンドウ＆Phase Vocoder DAW＆縦書き・ルビ / v13.0: glTF・VRM スキニング＆ビジュアル WGSL シェーダー＆Voronoi 破壊＆EXR 16-bit / v14.0: ブランドデザインシステム＆スタイル学習 120+ MCP＆WASM v2＆長期運用 WCAG AAA）。

## [10.0.0] - 2026-11-25

Spatial Computing, Physics Fluids & Autonomous Cinema OS 最終マイルストーン v10.0（#57〜#60）をすべて実装し、**全 60 ロードマップ項目を完全達成**しました（Pure Rust / ゼロ `unsafe` / ゼロ追加依存・**全 100 MCP ツール / 10 ドメイン**）。

### #57 360° 全天球動画（OverCapture / Tiny Planet / 水平ロック）＆ ステレオスコピック空間 3D ビデオ編集
* `ve-core` / `ve-render`: `spatial_video` / `v100_render` モジュール — Equirectangular ⇔ 透視投影リフレーム（`OverCapture`）・逆ステレオ投影（`TinyPlanet`）・全天球レビュー（`EquirectangularReview`）の厳密逆射影関数 `project_screen_to_equirect_uv`、ジャイロ時系列サンプルからの指数平滑化水平ロック `evaluate_horizon_locked_euler`、および Side-by-Side / Top-and-Bottom / Red-Cyan アナグリフ / 単眼 L・R ステレオスコピック空間 3D パス `render_spatial_360_and_stereo_pass`（`Scene::spatial_video` / `EditCommand::SetSpatialVideo`）を `RenderGraph` へ統合
* `ve-app` / `ve-mcp`: VFX & Motion Hub 内「🌐 360° 全天球動画＆空間 3D (#57)」タブ、MCP `configure_360_overcapture_and_horizon_lock` / `configure_stereoscopic_spatial_3d` を追加

### #58 決定論 2D Navier-Stokes 流体（煙・炎・インク）＆ クロス物理 ＋ 3D シャドウマップ / HDRI IBL
* `ve-core` / `ve-render`: `fluid_cloth` / `v100_render` / `mesh3d_render` モジュール — Jos Stam 安定法 2D オイラー格子流体ソルバー `simulate_fluid_2d_at_time`（移流・暗黙拡散・渦度閉じ込め・圧力ポアソン反復・`ItemKind::FluidSim` / `EditCommand::SetFluidSim`）、構造・せん断・曲げ距離拘束付き決定論 PBD クロス物理ソルバー `simulate_cloth_pbd_at_time`（`ItemKind::ClothSim` / `EditCommand::SetClothSim`）、および 2 パス直交デプスシャドウマップ（3×3 / 5×5 PCF ソフトシャドウ＋地面シャドウキャッチャー）＆ Equirectangular プロシージャル HDRI 拡散/鏡面 IBL `sample_hdri_ibl` / `render_mesh_data_pbr_with_adv`（`Scene::lighting_3d_adv` / `EditCommand::SetLighting3DAdv`）を `RenderGraph` へ統合
* `ve-app` / `ve-mcp`: VFX & Motion Hub 内「🔥 2D 流体・クロス物理＆3D シャドウ/HDRI IBL (#58)」タブ、MCP `add_or_update_fluid_simulation` / `add_or_update_cloth_simulation` / `configure_3d_shadows_and_hdri_ibl` を追加

### #59 BVH / 動画シルエット モーションキャプチャ → 2D / 3D リグ自動リターゲット（足接地ロック付）
* `ve-core`: `mocap_retarget` モジュール — Pure-Rust 標準 `.bvh`（`HIERARCHY` / `MOTION`）パーサ `parse_bvh`＆3D 順運動学 `evaluate_bvh_world_joints`、前景シルエット RGBA8 からの 7 関節 2D 骨格姿勢推定 `extract_silhouette_pose_from_rgba8`、ボーン長比率スケーリング＋足滑り防止 Foot Skate Locking IK リターゲット `retarget_bvh_to_rig2d` / `retarget_silhouette_sequence_to_rig2d`、および 1-Undo キーフレームベイク `build_mocap_retarget_bake_commands` を実装
* `ve-app` / `ve-mcp`: VFX & Motion Hub 内「🏃 BVH / シルエットモキャップ＆足接地リターゲット (#59)」タブ、MCP `import_bvh_mocap_and_retarget` / `extract_silhouette_mocap_pose` を追加

### #60 LAN 分散レンダーファーム ＆ 自律 AI シネマプロデューサー（台本→絵コンテ・A/B 自動編集・視聴維持率予測・全 100 MCP ツール）
* `ve-core`: `cinema_os` モジュール — Pure-Rust RFC 2104 `hmac_sha256_hex`（RFC 4231 公式テストベクター検証済）、重み比例チャンク分割＆署名検証 `plan_render_farm_chunks` / `verify_chunk_hmac`、非ブロッキングローカル HTTP/1.1 コーディネータ `RenderFarmCoordinatorServer`、FFmpeg 無劣化 concat マニフェスト `build_farm_concat_manifest`（`Project::render_farm` / `EditCommand::SetRenderFarm`）、さらに台本→絵コンテ構造解析＆プレイアブルアニマティック 1-Undo 自動構築 `parse_script_to_storyboard` / `build_storyboard_animatic_commands`、秒別視聴維持率カーブ予測＆離脱診断 `analyze_viewer_retention`、戦略別 A/B 自動編集バリアント生成 `build_ab_edit_variant_command` を実装
* `ve-app` / `ve-mcp`: AI & Collab Hub 内「🎬 自律 AI シネマプロデューサー (#60)」タブ（維持率グラフ可視化付）・Deliver Hub 内「🖥 LAN 分散レンダーファーム (#60)」タブ、MCP `manage_lan_render_farm` / `produce_storyboard_animatic_from_script` / `predict_retention_and_generate_ab_variants`（第 10 ドメイン `v100_spatial_cinema_os`・**全 100 MCP ツール完全体系化**）、およびゴールデン視覚回帰テスト `golden_v100_spatial_360_fluid_cloth_ibl_and_mocap_determinism` を追加

### v10.0.0 品質・性能総仕上げ ＆ リリース準備ハードニング
* **大規模ファイル分割リファクタリング（公開 API 互換 100%）:** `crates/core/src/command/`（`mod.rs` / `apply.rs` / `tests.rs`）、`crates/render/src/graph/`（`mod.rs` / `effects.rs` / `tests.rs`）、`crates/app/src/timeline/`（`mod.rs` / `actions.rs` / `tests.rs`）へ責務別分割整理
* **GPU / WGSL コンピュート高速化 ＆ CPU ホットパス最適化:** `crates/render/src/gpu_v100.rs` に `SPATIAL_360_EQUIRECT_WGSL`・`FLUID_2D_ADVECTION_WGSL` コンピュートシェーダーと `GpuV100ComputePipeline`・全 8 本組み込み WGSL シェーダー Naga 一括検証器 `validate_all_builtin_wgsl_shaders` を追加（VFX Hub GPU パネルへ統合）。さらに 360° CPU パスへフレーム単位事前計算 `RotationYprMatrix` ＆ ゼロクローン借用を導入し毎ピクセルの三角関数再計算を排除
* **全機能統合 E2E ＆ 100 MCP ツール耐障害性監査:** `crates/render/tests/e2e_v100_full_suite.rs`（v7.0〜v10.0 全コマンド Undo/Redo＆`history.jsonl` クラッシュ完全復旧・レガシー v0.1 JSON 後方互換ロード・全 8 WGSL シェーダー検証・全載せシーン 4 スレッド並列描画ビット完全一致）および `v100_mcp_all_100_tools_schema_audit_and_error_resilience` を追加
* **マルチ OS リリース整備:** `.github/workflows/release.yml` の macOS `.app` `Info.plist` バージョンをリリースタグ連動（既定 `10.0.0`）＆ `CHANGELOG.md` 同梱対応へ更新し、`docs/architecture.md` を v10.0.0 最新構成へ刷新

## [9.0.0] - 2026-11-25

Live Broadcast, VTuber & Hardware Ecosystem マイルストーン v9.0（#53〜#56）をすべて実装し、全 56 ロードマップ項目を完全達成しました（Pure Rust / ゼロ `unsafe` / ゼロ追加依存・全 **90 MCP ツール** / 9 ドメイン）。

### #53 ライブスイッチャー ＆ NLE 同時収録（RTMP / SRT / スティンガー遷移 / `.veproj` 同時生成）
* `ve-core` / `ve-media` / `ve-render`: `live_switcher` / `ffmpeg` / `v90_render` モジュール — PGM/PVW 2 バス放送ライブスイッチャー `LiveSwitcherConfig`（`EditCommand::SetLiveSwitcher`）、6 種アルファ付きスティンガー遷移コンポジタ `render_live_stinger_transition`＆放送タリー HUD `draw_live_tally_hud`、URL スキーム厳格検証付き FFmpeg サイドカー RTMP/RTMPS/SRT＋ローカル MKV ISO tee 引数ビルダー `build_live_stream_ffmpeg_args`、およびスイッチング履歴を 1-Undo NLE タイムライン＆編集用 `.veproj` JSON へ同時生成する `build_live_switcher_veproj_commands` / `export_live_session_veproj_json` を実装
* `ve-app` / `ve-mcp`: Deliver Hub 内「📡 ライブスイッチャー＆NLE 同時収録 (#53)」タブおよび MCP `live_switcher_and_broadcast` ツールを追加

### #54 VTuber 2D 三角メッシュ変形（ブレンドシェイプ）・Verlet 物理揺れ・UDP OSC / VMC 顔トラッキング連携
* `ve-core` / `ve-render`: `vtuber_rig` / `v90_render` モジュール — Pure-Rust OSC 1.0 バイナリコーデック `encode_osc_message` / `decode_osc_packet`（`#bundle` 対応）、VMC プロトコル変換 `apply_vmc_osc_message_to_blendshapes`（クォータニオン→オイラー角）、非ブロッキングローカル UDP 受信機 `VmcUdpReceiver`、2D 三角グリッドメッシュ生成＆擬似 3D ブレンドシェイプ変形 `deform_vtuber_mesh`、距離拘束付き 60Hz 決定論 Verlet 物理チェイン `simulate_verlet_hair_chain`、重心座標アフィンテクスチャラスタライザ `render_vtuber_avatar`（`ItemKind::VtuberAvatar` / `EditCommand::SetVtuberAvatar`・4 内蔵プリセット＋カスタム PNG 対応・`vt_yaw`〜`vt_blink_r` 1-Undo キーフレームベイク `build_vtuber_vmc_bake_command`）を `RenderGraph` へ統合
* `ve-app` / `ve-mcp`: VFX & Motion Hub 内「🦊 VTuber 2D メッシュ＆Verlet＆VMC (#54)」タブおよび MCP `vtuber_mesh_physics_and_vmc` ツールを追加

### #55 OSC / MIDI-over-UDP / Stream Deck WebSocket ハードウェアコントローラー連携
* `ve-core`: `hardware_ctrl` モジュール — Pure-Rust FIPS 180-4 SHA-1 `sha1_digest` ＆ RFC 6455 WebSocket ハンドシェイク／マスク付きテキストフレームコーデック（公式テストベクター検証済）、生 MIDI / RTP-MIDI-over-UDP パーサ `parse_midi_udp_packet`、3 プロトコル統合非ブロッキングローカルサーバー `HardwareBridgeServer`（OSC UDP + MIDI UDP + Stream Deck WebSocket）、4 種プリセット `HardwareControllerPreset`、および `CommandBus` 統合ディスパッチャ `map_hardware_event_to_outcome`（`EditCommand::SetHardwareController`）を実装
* `ve-app` / `ve-mcp`: AI & Collab Hub 内「🎛 OSC / MIDI / Stream Deck ハードウェア連携 (#55)」タブおよび MCP `hardware_controller_osc_midi_deck` ツールを追加

### #56 ローカル スマート DAM（代表色/BPM/LUFS 索引・類似ショット検索・スマートビン・一括自動再リンク）
* `ve-core`: `smart_dam` モジュール — 先頭 64KiB＋サイズによる FIPS 180-4 SHA-256 指紋算出 `compute_file_head_fingerprint`、5 色ドミナントパレット・暖色度・Sobel エッジ密度・16 次元 L2 正規化特徴ベクトル抽出 `extract_visual_features_from_rgba8`、コサイン類似度検索 `find_similar_shots`、条件式スマートビン `evaluate_smart_bin`（4 標準スマートビン内蔵）、およびドライブ移動・リネーム後の行方不明素材を SHA-256 指紋優先で 1-Undo 一括自動再リンクする `relink_missing_assets_from_dir` / `build_relink_assets_command`（`EditCommand::SetSmartDamIndex` / `RelinkAssets`）を実装
* `ve-app` / `ve-mcp`: Edit Hub 内「🗂 ローカル スマート DAM (#56)」タブ（5 色スウォッチ表示付）、MCP `smart_dam_search_and_relink`（`v90_broadcast_ecosystem` ドメイン追加・全 **90 MCP ツール**）、およびゴールデン視覚回帰テストを追加

## [8.0.0] - 2026-11-25

Pro Cinema VFX, Motion Foundry & Audio Restoration マイルストーン v8.0（#49〜#52）をすべて実装し、全 52 ロードマップ項目を完全達成しました（Pure Rust / ゼロ `unsafe` / ゼロ追加依存・全 **86 MCP ツール** / 8 ドメイン）。

### #49 4 点平面トラッカー（Planar Homography Tracker）＆ エッジ吸着キーフレームロトブラシ
* `ve-core` / `ve-render`: `v80_vfx` / `v80_render` モジュール — 8×8 ガウス消去法による一般四辺形 $3\times 3$ 射影ホモグラフィソルバー `solve_quad_to_quad_homography`、4×4 グリッドパッチ ZNCC＋サブピクセル放物線補間＋RANSAC 外れ値除去による `estimate_homography_ransac` / `track_planar_quad_in_scene`、および `pin_tl_x`〜`pin_bl_y` キーフレームを 1-Undo `EditCommand::Batch` へ焼き込む `build_planar_track_corner_pin_command`（`TimelineItem::eval_corner_pin` で `RenderGraph` 毎フレーム評価）を実装
* `ve-core` / `ve-render`: Sobel 勾配強度マップ `compute_sobel_edge_magnitude`・輪郭頂点エッジ吸着 `snap_roto_vertices_to_edges`・オプティカルフロー時間軸伝播 `propagate_roto_brush_in_scene`・非破壊キーフレームロト `RotoBrushConfig`（`EditCommand::SetRotoBrush`）を `RenderGraph` マスクパスへ統合
* `ve-app` / `ve-mcp`: VFX & Motion Hub 内「🎯 4 点平面トラッカー＆ロトブラシ (#49)」タブ（4 隅直接ドラッグ可能キャンバス付）および MCP `planar_track_and_roto_brush` ツールを追加

### #50 SVG `<path>` インポート＆パスモーフィング・Text-on-Path・ディスプレイスメント/グリッドワープ
* `ve-core` / `ve-render`: `motion_foundry` / `v80_render` モジュール — `M/m/L/l/H/h/V/v/C/c/S/s/Q/q/T/t/A/a/Z/z` 全コマンドおよび SVG F.6 楕円弧→3 次ベジェ厳密変換に対応する Pure-Rust SVG `<path>` パーサ `parse_svg_path_to_vertices`（6 標準プリセット `SvgShapePreset`）、等弧長リサンプリング＆頂点数正規化モーフィング `morph_paths_equal_arclength`（`ItemKind::SvgMorph` / `EditCommand::SetSvgMorph` / `render_svg_morph`）、3 次曲線接線回転追従タイポグラフィ `TextOnPathConfig`（`EditCommand::SetTextOnPath` / `render_text_on_path`）、および Catmull-Rom 4×4 双三次グリッドワープ＋5 プリセット＋自己輝度ディスプレイスメント `WarpDisplaceConfig`（`EditCommand::SetWarpDisplace` / `apply_warp_and_displacement`）を `RenderGraph` へ統合
* `ve-app` / `ve-mcp`: VFX & Motion Hub 内「✒ SVG モーフィング＆Text-on-Path＆ワープ (#50)」タブおよび MCP `svg_morph_text_path_and_warp` ツールを追加

### #51 シネマ光学 VFX（擬似深度ボケ Rack Focus・ゴッドレイ・Brown-Conrady レンズ歪み補正＆物理色収差）
* `ve-core` / `ve-render`: `v80_vfx` / `v80_render` モジュール — 3 種擬似深度モード（チルトシフト／中央放射／輝度顕著性）＋Fibonacci 多角形絞り羽根（0=真円 / 5〜9 角）玉ボケの Rack Focus、放射状オクルージョンサンプリングによる体積光ゴッドレイ、および Brown-Conrady 放射/接線歪みモデル（$k_1, k_2, p_1, p_2$・自動ズーム補償）＆RGB 波長分散色収差を統合した `OpticalVfxConfig`（4 プリセット `OpticalVfxPreset` / `EditCommand::SetOpticalVfx` / `apply_cinema_optical_vfx`）を `RenderGraph` ポストプロセス段へ統合（A/B Bypass 連動対応）
* `ve-app` / `ve-mcp`: VFX & Motion Hub 内「🎥 シネマ光学 VFX (#51)」タブおよび MCP `cinema_optical_vfx` ツールを追加

### #52 オーディオ修復（ハム除去/デクリック/適応 De-Reverb）＆ ADR カウント収録 ＆ 5.1 / FOA Ambisonics バス
* `ve-audio` / `ve-core` / `ve-render`: `restoration` / `v80_vfx` / `v80_render` モジュール — 2 段縦続 RBJ ノッチ IIR 50/60Hz 高調波電源ハム除去 `apply_hum_removal`（`hum_remove`・20dB 超減衰検証済）・2 次差分 MAD 外れ値検出＋3 次エルミート補間デクリック `apply_declick`（`declick`）・2 バンド適応後期残響エンベロープ減算 `apply_adaptive_dereverb`（`dereverb`）を `apply_audio_effect` へ統合。さらに 3-2-1 1kHz 同期ビープ合成 `synthesize_adr_preroll_beeps`・タイムラインキュー配置 `build_adr_cue_timeline_commands`・プレビュー 3-2-1 映像ストリーマ HUD `draw_adr_countdown_streamer`（`AdrSessionConfig` / `EditCommand::SetAdrSession`）、および ITU-R BS.775 5.1ch サラウンド（`encode_stereo_to_51_surround` / `downmix_51_to_stereo_itu`）＆1 次 Ambisonics FOA B-Format AmbiX（`W,Y,Z,X` SN3D/ACN 回転＆バイノーラル復号 `encode_stereo_to_foa_ambix` / `rotate_foa_ambix` / `decode_foa_ambix_to_binaural_stereo`）＋マルチチャンネル PCM16 WAV 出力 `write_multichannel_wav_file`（`SpatialBusConfig` / `EditCommand::SetSpatialBus`）を実装
* `ve-app` / `ve-mcp`: Audio Hub 内「🎙 オーディオ修復・ADR・5.1/FOA 空間バス (#52)」タブ（360° 空間音場レーダー付）、MCP `audio_restoration_adr_spatial_bus`（`v80_cinema_audio` ドメイン追加・全 **86 MCP ツール**）、およびゴールデン視覚回帰テストを追加

## [7.0.0] - 2026-11-25

Unified Studio UX & Core Architecture vNext マイルストーン v7.0（#45〜#48）をすべて実装し、全 48 ロードマップ項目を完全達成しました（Pure Rust / ゼロ `unsafe` / ゼロ追加依存・全 **82 MCP ツール**）。また、`docs/future_features.md` を次世代ロードマップ **v8.0〜v10.0（#49〜#60・全 60 項目）** へ拡張しました。

### #45 役割別 6 スタジオ Hub UX 統合 ＆ `Ctrl+K` ファジー検索コマンドパレット
* `ve-core`: `studio_hub` モジュール — 制作ロール別の 6 スタジオ Hub（`StudioHubId`: `Edit` / `VfxMotion` / `Color` / `Audio` / `AiCollab` / `Deliver`）、日英バイリンガル＋ロードマップ番号 `#1`〜`#48`＋ショートカットヒント付き全コマンドカタログ `STUDIO_PALETTE_CATALOG`、MRU 優先度付き決定論ファジー検索 `fuzzy_search_commands`、および `EditorAction::OpenCommandPalette`（`Ctrl+K` / `Ctrl+Shift+P`）を追加
* `ve-app` / `ve-mcp`: 現行 3 空間（メイン編集・モーション合成・ノード合成）を完全維持したまま、トップバーの `v0.9`〜`v6.0` 個別ボタン群を 6 スタジオ Hub 統合ウィンドウ＋`Ctrl+K` モーダルコマンドパレット `V70StudioHubState` へ機能ゼロ欠落で再編成し、MCP `studio_hub_and_command_palette` ツールを追加

### #46 内蔵 MCP モジュール分割リファクタリング（ドメイン別サブモジュール整理 ＆ 全 82 ツール）
* `ve-mcp`: 12,000 行超に肥大化した単一 `crates/mcp/src/tools.rs` を 6 スタジオ Hub 対応のドメイン別サブモジュール（`crates/mcp/src/tools/{mod,edit_nle,vfx_motion,color_finishing,audio_voice,ai_collab,deliver_system,v70_unified,tests}.rs`）へ完全分割。既存 78 ツールの JSON スキーマ＆動作互換を 100% 維持したまま v7.0 新規 4 ツールを追加し、ドメイン別カタログ統計＆検索 API `tool_domain_catalog` および MCP `mcp_catalog_introspect` を実装

### #47 GPU-Resident 4K/8K 合成フレームプール ＆ WGSL ブレンド/マスク/色管理高速パス
* `ve-render`: `gpu_v70` モジュール — 解像度＆フォーマット別バケット・VRAM/RAM 予算上限・LRU 自動 Eviction・再利用ヒット率計測を備えた `ResidentFramePool` を `RenderGraph` のレイヤー合成バッファ管理へ統合。さらに IEEE-754 `f16` パック＆64 MiB ストライプ分割で標準 `wgpu` ストレージ上限下でも 4K/8K 動作する `@compute @workgroup_size(8, 8)` シェーダー `FAST_BLEND_MASK_WGSL`＆`FAST_ACES_COLOR_MGMT_WGSL`（`naga` 静的検証済）、ヘッドレス GPU パイプライン `GpuV70FastPipeline`、および決定論的 CPU フォールバック（`execute_v70_blend_mask_or_cpu` / `execute_v70_aces_or_cpu`）を実装
* `ve-app` / `ve-mcp`: VFX & Motion Hub 内「⚡ #47 GPU 4K/8K プール＆高速パス」パネルおよび MCP `gpu_v70_frame_pool` を追加

### #48 大規模タイムライン最適化（10,000+ クリップ区間木 $O(\log N)$）＆ プロキシ⇔原本 TC 自動コンフォーム ＆ A/B スプリット比較ビュー
* `ve-core` / `ve-render` / `ve-app` / `ve-mcp`: フラット配列型 Augmented Interval Tree `SceneIntervalIndex` / `CachedSceneIntervalIndex`（構築 $O(N \log N)$・時刻刺し/範囲検索 $O(\log N + K)$、12,000 クリップ上で線形走査と 100% 順序一致検証済）を `RenderGraph`・`TimelineState`（ビューポートカリング＆スナップ最適化）・MCP `get_timeline_range` へ統合
* `ve-core` / `ve-media` / `ve-app`: `ffprobe` からの SMPTE 開始タイムコード＆リール名自動抽出（`MediaProbe.start_timecode` / `reel_name`）、インポート時 `Project::conform_meta` 自動記録、およびリール名正規化＋開始タイムコード差分からクリップの `asset_id`＆`source_offset` を 1-Undo 自動変換する `plan_timeline_conform` / `build_timeline_conform_command`（`EditCommand::SetAssetConformMeta` / `ConformClipSource`）を実装
* `ve-core` / `ve-render` / `ve-app` / `ve-mcp`: 垂直・水平・斜めワイプ＆差分強調（未グレーディング Bypass / 静止画フリーズ A / 参照タイムコード / 別シーン比較）の `AbCompareConfig`（`EditCommand::SetAbCompare`）・`compose_ab_split_linear`・`render_ab_compare_scene`・プレビューキャンバス上での分割線直接ドラッグ操作（`Alt+W` トグル対応）、および MCP `timeline_index_conform_ab`（全 **82 MCP ツール**）＋ゴールデン視覚回帰テストを追加

## [6.0.0] - 2026-11-22

クリエイター Pro スタジオマイルストーン v6.0（#41〜#44）をすべて実装し、全 44 ロードマップ項目を完全達成しました（Pure Rust / ゼロ `unsafe` / ゼロ追加依存・全 **78 MCP ツール**）。

### #41 データ駆動モーショングラフィックス＆インフォグラフィック・チャートエンジン
* `ve-core` / `ve-render`: `data_viz` / `v60_render` モジュール — CSV/JSON データ（`parse_chart_csv`・オプション HEX 色指定）から 4 種の決定論アニメーションチャート（棒グラフ `BarChart`・トレンド折れ線＆エリア `LineChart`・ドーナツ割合 `DonutChart`・KPI ロールアップカウンター `KpiCounter`）を Cubic Ease-Out 演出＋5×7 ビットマップ数値・ラベル付きで描画する `render_data_chart`・`ItemKind::DataChart`・`EditCommand::SetDataChart` を `RenderGraph`（スピードランプ＆プレビューギズモ対応）へ統合
* `ve-app` / `ve-mcp`: v6.0 クリエイター Pro「📊 #41 チャート」タブ（CSV エディタ＆ミニプレビュー付）および MCP `data_chart_motion_graphics` を追加

### #42 非破壊 VFX ペイント ＆ クローンスタンプ / ワイヤー・不要物消し（Inpainting）＆ プライバシー秘匿ぼかし
* `ve-core` / `ve-render`: `vfx_paint` / `v60_render` モジュール — クリップ単位の非破壊ベクターペイント設定 `TimelineItem::vfx_paint`（`EditCommand::SetVfxPaint`）と 4 種ツール（Write-On アニメーション対応 `BrushGlow`・バイリニアフェザー複製 `CloneStamp`・ストローク法線＆周囲 8 方向クリーン境界補間による `WireEraseInpaint`・局所モザイク平均化 `BlurRedact`）を `RenderGraph` のアイテム後段パス `apply_vfx_paint` へ統合
* `ve-app` / `ve-mcp`: 直接ドラッグ描画可能な正規化スケッチパッド付き「🖌 #42 VFXペイント」タブおよび MCP `vfx_paint_and_inpaint` を追加

### #43 マルチトラック・ポッドキャスト＆コラボ実況オート編集エンジン
* `ve-audio`: `podcast_studio` モジュール — 複数話者ステレオ PCM を 50ms ブロック RMS＋無音ゲート＋ギャップ結合で解析し、アクティブ話者ターン区間および `-16 LUFS` 推奨ゲインを算出する `analyze_podcast_speakers`、滑らかな短期 AGC＋True-Peak `-1.0 dBTP` リミッターで話者ごとの声量ムラを揃える `apply_podcast_voice_leveler`、さらに話者マーカー・カメラ自動切替 Hold キーフレーム・音声フェーダー均一化を 1-Undo `EditCommand::Batch` へ組み立てる `build_podcast_auto_switch_commands` を実装
* `ve-app` / `ve-mcp`: 「🎙 #43 ポッドキャスト」タブおよび MCP `podcast_auto_director` を追加

### #44 プロ・マルチプラットフォーム自動リフォーマット＆1 クリック納品バンドルパッケージャー
* `ve-core` / `ve-mcp` / `ve-app`: `multi_deliver` モジュール — 1 本のマスターシーンから 4 プラットフォーム（YouTube 16:9・Shorts/Reels 9:16・Square 1:1・Cinema 4K）向けに中央リフレーム・縦動画セーフゾーン字幕折り返し・LUFS 目標を自動最適化した派生シーンを生成する `build_reformatted_scene_command` と、全派生シーン JSON・媒体別 SRT・YouTube チャプター・`1280×720` 高 CTR サムネイル PNG・CMX 3600 EDL・FCPXML v1.10・放送 QC レポート JSON・`manifest.json` を 1 クリックで束ねて出力する納品パッケージャー（GUI「📦 #44 マルチ納品」タブ＆MCP `multi_platform_packager`・全 **78 MCP ツール**）を実装

## [5.0.0] - 2026-11-21

プロフィニッシング＆モーションリギング＆スマート音響マイルストーン v5.0（#37〜#40）をすべて実装し、全 40 ロードマップ項目を完全達成しました（Pure Rust / ゼロ `unsafe` / ゼロ追加依存・全 **74 MCP ツール**）。

### #37 2D スケルトンボーン ＆ 2-Bone IK パペットリギングエンジン
* `ve-core` / `ve-render`: `rigging` / `rig_render` モジュール — 10 ボーン標準ヒューマノイド階層 FK・余弦定理による解析的 2-Bone 逆運動学 `solve_two_bone_ik`・髪/リボン二次物理スウェイ・4 種モーションプリセット（`IdleBreathe` / `WaveHello` / `NodTalk` / `DanceBounce`）・`ItemKind::RiggedPuppet`＆`EditCommand::SetRig2D` を `RenderGraph`（アンチエイリアスカプセル＋ボーン/IK オーバーレイ）へ統合
* `ve-app` / `ve-mcp`: ライブスケルトン可視化付き「🦴 2D ボーン＆IK」タブおよび MCP `rig2d_puppet_control` を追加

### #38 オフライン自動プロシージャル BGM コンポーザー（音楽理論・マルチトラック 48kHz WAV ＆ 小節マーカー同期）
* `ve-audio`: `bgm_composer` モジュール — 著作権フリー完全オフラインで 6 ジャンル（Lo-Fi チルホップ・アップビートポップ・シネマティック壮大・サイバーシンセウェーブ・爽やかアコースティック・サスペンスパルス）のドラム・ベース・デチューンステレオコードパッド・メロディ＆エコーを 48kHz 放送安全リミット付きステレオ合成し、正確な拍／小節マーカー時刻を同時生成する `compose_bgm_track` を実装
* `ve-app` / `ve-mcp`: 「🎼 自動BGM作曲」タブ（小節マーカー 1-Undo 同時配置）および MCP `offline_bgm_composer` を追加

### #39 スピードランプ（Time-Remapping 解析積分曲線）＆ 自動シーンカット検出（Scene Edit Detection）
* `ve-core` / `ve-render`: `speed_ramp` / `rig_render` モジュール — 5 プリセット（`BulletTimeDip` / `RampUpHero` / `EaseInSlowMo` / `FlashBurst` 等）＆カスタムノットの厳密解析積分タイムリマップ `evaluate_speed_ramp_source_time`（`TimelineItem::speed_ramp`・`EditCommand::SetSpeedRamp`）を `TimelineItem::evaluate_source_time_sec`＆`RenderGraph` へ統合。さらに 64 ビン RGB ヒストグラム χ² 距離＋輝度変化によるショット境界自動検出 `detect_scene_cuts_in_scene` および 1-Undo マーカー＆自動分割コマンドビルダー `build_scene_cut_commands` を実装
* `ve-app` / `ve-mcp`: 速度曲線グラフ＆検出カット一覧付き「⚡ 速度曲線＆カット検出」タブおよび MCP `speed_ramp_and_scene_detect` を追加

### #40 プロ・シネマフィニッシング（35mm ハレーション・アナモルフィックフレア・グレイン・バーンイン）＆ CMX 3600 EDL / FCPXML v1.10 書き出し
* `ve-core` / `ve-render`: `finishing` / `finishing_render` モジュール — 35mm フィルムハレーション（高輝度赤橙光学散乱）・双方向指数ストリーク型アナモルフィック水平レンズフレア・オーガニック 35mm グレイン・周辺ビネット・2.39:1/1.85:1/4:3 レターボックス・SMPTE タイムコード＆透かしバーンイン HUD（`apply_cinema_finishing` / `EditCommand::SetCinemaFinishing`）を `RenderGraph` 最終段へ統合。さらに業界標準 **CMX 3600 EDL**（`export_cmx3600_edl`）および **Final Cut Pro XML v1.10**（`export_fcpxml_v10`）書き出しを実装
* `ve-app` / `ve-mcp`: 「🎬 シネマ仕上げ＆EDL」タブおよび MCP `cinema_finishing_and_edl`（全 **74 MCP ツール**）を追加

## [4.0.0] - 2026-11-21

マスタースタジオ＆クリエイティブ物理マイルストーン v4.0（#33〜#36）をすべて実装し、全 36 ロードマップ項目を完全達成しました（Pure Rust / ゼロ `unsafe` / ゼロ追加依存・全 **70 MCP ツール**）。

### #33 プロ・シネマカラー管理（ACEScg / OpenColorIO v2 パイプライン＆カメラ Log IDT）
* `ve-core` / `ve-render`: `color_mgmt` モジュール — Rec.709 / ACEScg (AP1) / Display P3 / Rec.2020 の厳密 3×3 色空間変換行列、カメラ Log EOTF（Sony S-Log3・Panasonic V-Log・ARRI LogC3）、ACES 1.3 Reference Gamut Compression（ネオン色相破綻防止）、ACES Fitted / AgX Punchy / Reinhard Extended トーンマッパー、`.ocio` v2 設定 YAML 書き出し（`export_ocio_config_yaml`）、および `Scene::color_management` / `EditCommand::SetColorManagement` を `RenderGraph` 最終段へ統合
* `ve-app` / `ve-mcp`: v4.0 マスタースタジオ「🎨 ACEScg / OCIO」タブおよび MCP `aces_color_management` を追加

### #34 決定論的 2D リジッドボディ物理演算エンジン（自動キーフレームベイク）
* `ve-core`: `physics` モジュール — 60Hz 固定タイムステップの 2D 剛体物理エンジン（重力・床/壁バウンス・摩擦回転・剛体同士の弾性衝突インパルス・中心ボテックス引力）と 4 種プリセット（`BouncingTitle` / `DominoCascade` / `ZeroGravityFloat` / `OrbitalVortex`）、および `x`/`y`/`rotation_deg` への 1-Undo キーフレーム自動ベイク `build_physics_bake_commands` を実装
* `ve-app` / `ve-mcp`: 2D 軌跡リアルタイム可視化キャンバス付き「🏀 物理演算エンジン」タブおよび MCP `physics_simulate_and_bake` を追加

### #35 オフライン・プロシージャル生成スタジオ（動画背景ジェネレーター ＆ Foley/SFX シンセサイザー）
* `ve-core` / `ve-render`: `generative` / `generative_render` モジュール — `ItemKind::GenerativeBg`＆`EditCommand::SetGenerativeBg` による 6 種の決定論アニメーション背景（オーロラ・シネマ玉ボケ・サイバー 3D グリッド・リキッドマーブル・スターフィールドワープ・アブストラクト波）を `RenderGraph` へ統合
* `ve-audio`: `foley` モジュール — 6 種プロシージャル効果音（ウーッシュ遷移・重低音インパクト・UI ポップ・グリッチ電子音・環境雨音・テンションライザー）の 48kHz ステレオ PCM/WAV 合成（`synthesize_foley_sfx_pcm` / `write_stereo_wav_file`）を実装
* `ve-app` / `ve-mcp`: 「🌌 背景＆Foley SFX」タブおよび MCP `generative_bg_and_foley` を追加

### #36 放送コンプライアンス QC 監査 ＆ 高 CTR サムネイル抽出 ＆ 自己完結型 HTML5 レビュープレイヤー
* `ve-core` / `ve-render`: `broadcast_qc` モジュール — 4 規格プロファイル（YouTube 1080p / 放送 EBU R128 / Shorts 9:16 / シネマ 4K）での ITU-R BS.1770 True-Peak/LUFS・ITU-R BT.1702 光過敏フラッシュ検査・輝度ガマット QC 監査 `run_broadcast_qc_audit`、エッジ鮮鋭度＋彩度＋コントラスト評価による `1280×720` 高 CTR ヒーローサムネイル自動抽出 `select_best_thumbnail_frame`、およびサーバー不要・フレーム PNG Base64 埋込の単一ファイル HTML5 インタラクティブレビュープレイヤー書き出し `export_portable_web_review_html` を実装
* `ve-app` / `ve-mcp`: 「📡 放送QC＆HTML5レビュー」タブおよび MCP `broadcast_qc_and_web_export`（全 **70 MCP ツール**）を追加

## [3.0.0] - 2026-11-21

次世代ワールド＆GPU マイルストーン v3.0（#29〜#32）をすべて実装し、全 32 ロードマップ項目を完全達成しました（Pure Rust / ゼロ `unsafe` / ゼロ追加依存・全 **66 MCP ツール**）。

### #29 フル GPU コンピュートパイプライン（Vulkan / Metal / DX12 WGSL ＆ 4K スコープ）
* `ve-render`: `gpu_v30` モジュール — `@compute @workgroup_size(8, 8)` WGSL コンピュートシェーダー `COLOR_GRADE_COMPUTE_WGSL` および原子ヒストグラム／スコープ用 `VIDEO_SCOPES_COMPUTE_WGSL`（`atomicAdd`）を追加し、`naga` 静的検証・`wgpu` ヘッドレス GPU ディスパッチ（`GpuV30Pipeline`）・決定論的 CPU フォールバック（`run_gpu_or_cpu_grade_and_scopes`）・1080p/4K 実測ベンチマーク（`benchmark_v30_gpu_pipeline`）を実装
* `ve-app` / `ve-mcp`: v3.0 スタジオ「⚡ GPU コンピュート＆4K」タブおよび MCP `gpu_v30_pipeline` を追加

### #30 3D メッシュ / glTF 2.0 シーン合成 ＆ 3D カメラトラッカー
* `ve-core`: `scene3d` モジュール — Pure-Rust RFC 4648 `base64_decode`・glTF 2.0（`.gltf` JSON + embedded Base64 data URI / 外部 `.bin` ＆ `.glb` バイナリコンテナ）パーサ `parse_gltf_or_glb`・GLB エンコーダ `encode_mesh_to_glb`・プロシージャル 3D キューブ／球体／トーラス／**5×7 押し出し 3D 立体テキスト**・`ItemKind::Mesh3D`＆`EditCommand::SetMesh3D` を実装
* `ve-render`: `mesh3d_render` モジュール — Z バッファ付き PBR 3D ラスタライザ（GGX 鏡面・Schlick Fresnel・スタジオ環境反射・`Light3D` 連動・ワイヤーフレーム）を `RenderGraph` に統合し、特徴点視差＋Z ドリー復元の 3D カメラトラッカー `analyze_3d_camera_track`＆`build_3d_camera_track_command` を追加
* `ve-app` / `ve-mcp`: プレビュー直接操作ギズモ対応、「🧊 3D メッシュ/glTF ＆ カメラ追跡」タブ、および MCP `scene_3d_and_camera_track` を追加

### #31 オフライン多言語 AI 吹替 ＆ 音声リップシンク自動生成
* `ve-audio` / `ve-core`: `dubbing` モジュール — 完全オフライン（クラウド送信ゼロ）の日英テキスト→母音音素アライメント `align_text_to_phoneme_segments`・5 話者プリセットの 3 フォルマント共振 48kHz ステレオ音声合成 `synthesize_offline_dubbing_pcm`・字幕切替＋吹替 WAV 配置＋全立ち絵（`CharacterSprite`）への音素リップシンク一括同期 `build_dubbing_timeline_commands`（`EditCommand::SetItemPhonemes` 1-Undo 対応）を実装
* `ve-app` / `ve-mcp`: 「🎙 オフライン吹替＆リップシンク」タブ（母音タイムラインプレビュー付）および MCP `offline_ai_dubbing_lipsync` を追加

### #32 リアルタイム P2P / LAN ＆ CRDT 同時共同編集（クラウド不要ローカルチーム編集）
* `ve-core`: `crdt_sync` モジュール — Lamport 論理時計＋Vector Clock＋決定論的全順序 `(lamport, peer_id, op_id)` による可換・結合・冪等な Op-Based CRDT（`CrdtSyncLog`）・ライブピアカーソル在席管理（`PeerCursorState`）・トークン認証付き Pure-Rust LAN TCP JSON-Lines ピア同期サーバー＆クライアント（`LanCollabHost` / `sync_crdt_with_lan_peer`）・Delta JSON 入出力を実装
* `ve-mcp` / `ve-app`: `EditorSession::crdt_log` 共有統合、「🌐 P2P/CRDT 同時共同編集」タブ、および MCP `crdt_realtime_collab`（全 **66 MCP ツール**）を追加

## [2.1.0] - 2026-11-21

シネマ＆クリエイタースタジオ v2.1 マイルストーン（#25〜#28）をすべて実装し、全 28 ロードマップ項目を達成しました（Pure Rust / ゼロ `unsafe` / ゼロ追加依存・全 **62 MCP ツール**）。

### #25 プロ・カラーグレーディング完成（RGB/マスタートーンカーブ・HSL 二次色補正・ショット間自動カラーマッチ）
* `ve-core` / `ve-render`: `ColorCurves`（Master/R/G/B 制御点＋プリセット 3 種）および Fritsch-Carlson 単調 3 次エルミートスプライン補間 `evaluate_tone_curve_lut`（オーバーシュート無し）、`HslQualifier`＆`apply_hsl_qualifier_rgb`（特定色相・彩度・輝度のソフト抽出＋色相シフト/彩度/輝度補正＋非選択モノクロ暗転マット可視化）、および Reinhard 方式統計カラーマッチ `compute_auto_color_match` を `apply_scene_color_grade` へ統合
* `ve-app` / `ve-mcp`: シネマ＆クリエイタースタジオ「🎨 トーンカーブ / HSL / カラーマッチ」タブ（ドラッグ制御点＆スプライン描画キャンバス付）および MCP `pro_color_match_and_grade` を追加

### #26 自動 2D 手ぶれ補正 ＆ 平面 4 点コーナーピン合成（VFX トラッキング拡張）
* `ve-core` / `ve-render`: 8×8 部分ピボット付きガウス消去法による厳密 $3\times 3$ 射影ホモグラフィ `solve_unit_square_to_quad_homography`＆`apply_corner_pin`（エッジアンチエイリアスフェザー付・`EditCommand::SetCornerPin`）を `RenderGraph` に統合。さらに 4 象限マルチパッチカメラ運動推定 `estimate_global_frame_motion`・ゼロ位相平滑化手ぶれ補正 `analyze_camera_shake`・自動クロップ拡大＋1-Undo キーフレーム生成 `build_stabilize_commands` を実装
* `ve-app` / `ve-mcp`: 「📹 手ぶれ補正 / 4点コーナーピン」タブ（ブレ軌跡スパークライン＋ドラッグ操作可能な 4 隅射影クワッドキャンバス）および MCP `vfx_stabilize_and_corner_pin` を追加

### #27 3D 空間オーディオ（バイノーラルパンナー）＆ マスタリング DSP スイート
* `ve-audio` / `ve-core`: Woodworth 球面頭部モデルの ITD 両耳間時間差（端数サンプル補間）・ILD 両耳間レベル差・頭部回折シャドウ LPF・距離減衰・空間初期反射を備える `apply_binaural_spatial_3d`（`"spatial_3d"`）、位相滑らかな 3 バンド独立ダイナミクス `apply_multiband_compressor_3band`（`"multiband_comp"`）、および ITU-R BS.1770 準拠の 4× Catmull-Rom オーバーサンプリング True-Peak 計測 `measure_true_peak_dbtp`＆ブリックウォールリミッター `apply_true_peak_limiter`（`"true_peak_limiter"`、既定 `-1.0 dBTP` 厳守）を `apply_audio_effect_chain` に統合
* `ve-app` / `ve-mcp`: 「🎧 3D空間オーディオ / マスタリング」タブ（360° インタラクティブトップダウンレーダー＋True-Peak/LUFS 放送安全判定）および MCP `spatial_and_master_audio` を追加

### #28 ダイナミック・キネティック字幕 ＆ 多言語字幕ローカライズ切替
* `ve-core` / `ve-render`: `kinetic_sub` モジュール — 日本語・英語ハイブリッドのワード／文節トークナイザー `tokenize_kinetic_words`、4 種ハイライトモード（`WordPopColor` / `KaraokeSweep` / `ProgressiveReveal` / `KeywordPulse`）による毎フレーム動的 AviUtl2 `<#RRGGBB><sNN>` マークアップ生成 `generate_kinetic_inline_markup`（`RenderGraph` 統合・`EditCommand::SetKineticSubtitle`）、および多言語字幕テーブル抽出・スプレッドシート互換 CSV 入出力・1-Undo 言語一括切替（`extract_localization_table` / `export_localization_csv` / `import_localization_csv` / `build_apply_locale_commands`）を実装
* `ve-app` / `ve-mcp`: 「💬 キネティック字幕 / 多言語切替」タブ（ライブプレビューバー＆CSV エディタ付）および MCP `kinetic_subtitles_and_localize`（全 **62 MCP ツール**）を追加

## [2.0.0] - 2026-11-21

次世代 AI ＆ ノード合成・共同編集フロンティア v2.0 マイルストーン（#21〜#24）をすべて実装し、全 24 ロードマップ項目を完全達成しました（Pure Rust / ゼロ `unsafe` / ゼロ追加依存・3 ワークスペース・全 **58 MCP ツール**）。

### #21 ノードベース合成グラフ（第 3 ワークスペース：Fusion / Nuke スタイル）
* `ve-core`: `node_graph` モジュール（`NodeCompositeGraph`・11 種ノード：`LayerInput` / `SolidColor` / `Effect` / `ChannelSplit`［RGB/Alpha/Luma/3D Depth］/ `ChannelMerge` / `Transform` / `Merge`［`mask` 入力対応］/ `MaskCombine` / `ColorCorrect` / `DepthFog` / `Output`、Kahn トポロジカルソート＆サイクル検出、自動整列、双方向変換 `from_scene_layers` ＆ `sync_node_graph_to_scene_layers`、`EditCommand::SetNodeGraph`）を追加
* `ve-render`: `node_eval::evaluate_node_composite_graph` を `RenderGraph` に統合し、ノードグラフ有効時に 3D 深度パケット付きトポロジカル評価を実行
* `ve-app` / `ve-mcp`: 第 3 ワークスペース `WorkspaceMode::NodeComposite`＆インタラクティブノードキャンバス `Pane::NodeGraph`（ドラッグ配置・ベジェ配線・バイパス・ズーム/パン・双方向同期）および MCP `manage_node_graph` を追加

### #22 ローカル AI セグメンテーション ＆ インテリジェント自動リフレーム
* `ve-render`: 完全オフライン（クラウド送信ゼロ）の `ai_vision` モジュールを実装 — ローカル推論ステータス検出（Pure-Rust 内蔵＋オプショナル `models/*.onnx` / `VE_LOCAL_ONNX_BIN`）、グリーンバック不要人物切り抜き `apply_ai_segmentation`（`ai_segment` エフェクト統合）、顔・肌色＋動き＋エッジ顕著性＋ゼロ位相平滑化の `analyze_scene_auto_reframe`＆`build_auto_reframe_commands`、Lanczos-3＋エッジ誘導ローカル超解像（`upscale_super_resolution` / `ai_super_res`）、双方向オクルージョン保護オプティカルフロー補間（`estimate_optical_flow` / `interpolate_frame_optical_flow`）を `RenderGraph` の `PlaybackSettings::optical_flow_interp` へ統合
* `ve-app` / `ve-mcp`: フロンティアスタジオ「🧠 ローカル AI ビジョン」タブ（被写体軌跡グラフ付き）および MCP `run_local_ai_vision` を追加

### #23 マルチエージェント AI 編集ディレクター（MCP オーケストレーション ＆ 編集レシピ）
* `ve-core`: `recipe` モジュール（`EditRecipe`・5 専門ロール `AgentRole`・`VisualCheckExpectation`・Pure-Rust YAML/JSON シリアライザ＆パーサ・`{{var}}` 変数展開・履歴マクロ自動記録 `record_recipe_from_commands`・標準ディレクターレシピ 3 種・`ProjectFolder::recipes_dir()` CRUD）を追加
* `ve-mcp` / `ve-app`: `director` オーケストレータ（人間承認ゲート `AwaitingApproval`・`render_frame_preview` 相当の自動視覚自己検査 `inspect_preview_frame_metrics`・Undo ロールバック対応）、MCP `orchestrate_edit_recipe`（全 **58 MCP ツール**）、およびフロンティアスタジオ「🤖 AI 編集ディレクター」UI を追加

### #24 共同編集・差分マージ ＆ タイムコードレビュー（Git-Friendly NLE）
* `ve-core`: `collab` モジュール（意味的タイムライン差分 `diff_projects`・3-Way 意味的ブランチマージ `merge_projects_three_way`［非競合自動統合＋競合解決 `KeepOurs`/`KeepTheirs`/`KeepBothDuplicate`＋1 ステップ Undo 可能な `ReplaceProjectSnapshot`］・Frame.io CSV/JSON＆YouTube タイムスタンプレビューコメント入出力・ステータス別カラー付きマーカー自動変換・`ProjectFolder::snapshots_dir()` CRUD）を追加
* `ve-app` / `ve-mcp`: フロンティアスタジオ「🤝 共同編集・Diff＆レビュー」UI（カラー付き Diff・クリックシーク・3-Way 競合解決・Frame.io/YouTube 入出力）および MCP `collab_diff_and_review` を追加

## [1.0.0] - 2026-11-21

長尺・大規模・プロ品質のプロダクション完成版マイルストーン v1.0（#17〜#20）をすべて実装しました（Pure Rust / ゼロ `unsafe` / ゼロ追加依存・全 **54 MCP ツール**）。

### #17 マルチカメラ編集 ＆ 音声相互相関による自動同期
* `ve-audio`: 2 段階正規化相互相関（500Hz RMS エンベロープ粗探索 → 8kHz 波形精密探索、`estimate_audio_offset_xcorr`）によるサブフレーム精度（0.125ms）の自動同期オフセット算出を実装
* `ve-core` / `ve-app` / `ve-mcp`: マルチカム自動整列（`build_multicam_sync_align_commands`）・2×2 / 3×3 同時プレビューグリッド＆数字キー `1`〜`9` ライブスイッチング記録・1 ステップ Undo 切替カット確定（`build_multicam_switch_cut_commands`）および MCP `sync_multicam` を追加

### #18 パフォーマンス革命：HW デコード ＆ 並列合成 ＆ 永続ディスクキャッシュ
* `ve-media`: FFmpeg ハードウェアデコード（`d3d11va` / `dxva2` / `vaapi` / `videotoolbox`）自動検出＆安全 Software フォールバック（`open_video_stream_with_hwaccel`）および解像度・コーデック可変プロキシ生成（`ProxyProfile` / `H264Fast` / `MjpegIntra`）を実装
* `ve-render`: マルチスレッド並列フレームバッチレンダリング（`render_frame_batch_parallel`）と `@compute @workgroup_size(8, 8)` `wgpu` コンピュートシェーダーコンポジタ（`WgpuComputeCompositor` / `composite_compute.wgsl`）を実装
* `ve-core` / `ve-app`: アプリ再起動後も再利用できる SSD 永続フレームキャッシュ（`PersistentFrameCache` / `cache/frames/` コンテンツハッシュ＋LRU 管理）を `RenderGraph` / `RamPreviewManager` へ統合し、バックグラウンド書き出しも並列化

### #19 ノンリニア編集プロ機能 ＆ 音声ポストプロダクション完成
* `ve-core` / `ve-app`: スリップ／ロール／スライドトリム（`SlipTrim` / `RollTrim` / `SlideTrim`）、クリップグルーピング＆A/V リンク解除（`Ctrl+G` / `Ctrl+Shift+G` / `SetGroupId`）、タイムライン上のボリューム（黄）・パン（シアン）オートメーションカーブ直接描画、トラックミキサー（`SetLayerMixer`：フェーダー/パン/ソロ/ミュート/バス/Aux リバーブセンド）、マイク直接アフレコ録音（`VoiceoverRecorder`）を実装
* `ve-audio`: Pure-Rust Hann 窓 STFT スペクトル減算ノイズリダクション（`noise_reduction`）、ダイナミック LPF ディエッサー（`de_esser`）、オフライン Mid/Side＋帯域ボーカル／BGM 分離（`separate_vocal_bgm_stems`）を追加
* `ve-core` / `ve-media` / `ve-mcp`: 書き出し前検証チェックリスト（`validate_project_delivery`）、Pure-Rust IEEE CRC-32 ZIP アーカイブパッケージ化（`archive_project_to_zip`）、Rec.709 SDR カラータグ＆HDR10 PQ 10-bit（BT.2020 / ST 2084 / `yuv420p10le`）書き出し、および MCP `nle_pro_edit` / `validate_and_package_project` を追加

### #20 v1.0 品質保証・カスタマイズ・配布エコシステム
* `ve-core` / `ve-app` / `ve-mcp`: カスタムキーバインド（AviUtl2 / Premiere Pro / DaVinci Resolve / After Effects プリセット＆JSON 入出力）、多言語 UI（日本語 / 英語 `tr`）、UI テーマ 4 種＆フォント倍率アクセシビリティ、FIPS 180-4 Pure-Rust SHA-256 署名検証＆サンドボックス権限明示付き公式拡張レジストリブラウザ、および MCP `manage_ecosystem` を追加（全 **54 MCP ツール**）
* 配布＆QA: Windows `winget` マニフェスト（`packaging/winget/0tsu-0w0.VideoEditingApp.yaml`）・macOS Universal `.app` Bundle・Linux AppDir（`packaging/linux/ve-app.desktop`）対応 `.github/workflows/release.yml`、およびゴールデン視覚回帰テストスイート（`crates/render/tests/visual_regression.rs`）を追加

## [0.9.0] - 2026-11-21

AE 級プロシージャル演出・テンプレート量産・高度な 2D/3D 合成を実現する v0.9 ロードマップ（#14〜#16）をすべて実装しました。

### #14 Pure-Rust エクスプレッションエンジン（式アニメーション）
* `ve-core`: ゼロ `unsafe`・決定論的サンドボックス式パーサ＆バイトコード VM（`expression` モジュール：`time`, `local_time`, `duration`, `progress`, `fps`, `frame`, `value`, `wiggle(freq, amp[, octaves])`, `noise`, `random`, `sin`/`cos`/`pow`/`clamp`/`linear`/`ease`, `loopOut("cycle"|"pingpong"|"offset"|"continue")`, `audio_band("low"|"mid"|"high"|"rms")`, `layer("名前").prop`）およびキーフレーム焼き込み `build_bake_expression_command`、`EditCommand::SetExpression` を実装
* `ve-render`: `RenderGraph` の全アイテム変形・トリムパス評価へエクスプレッションを統合（`audio_band` 遅延算出付き）
* `ve-app` / `ve-mcp`: インスペクタの式エディタ（リアルタイム評価値＆構文エラー表示・スニペット・Bake ボタン）、グラフエディタの式カーブ可視化、および MCP ツール `set_expression` を追加

### #15 モーショングラフィックス テンプレート（`.mogrt` 相当）＆ プロジェクトテンプレート
* `ve-core`: `MotionTemplate` / `TemplateBinding` / `ProjectTemplate`、イントロ・アウトロ尺を保護し中間のみ伸縮するレスポンシブ時間伸縮 `apply_responsive_time_stretch`、組み込みモーショングラフィックス 3 種（ニュース風ローワーサード / キネティック OP タイトル / 告知 CTA カード）＆プロジェクト雛形 3 種、`ProjectFolder` の `templates/`・`templates/projects/` 永続化 CRUD、`EditCommand::UpdateTemplateInstance` を実装
* `ve-app` / `ve-mcp`: テンプレートギャラリー、選択時に公開パラメータだけを表示する専用編集カード、現シーン保存・プロジェクト雛形新規作成 UI、および MCP ツール `manage_templates` を追加

### #16 高度な 2D/3D 合成：パーティクル・自由ベクターシェイプ・3D ライト＆モーションブラー
* `ve-core` / `ve-render`: 決定論的シード 2D/3D パーティクルエミッター `ItemKind::ParticleEmitter`（紙吹雪・雪・火花・光粒・集中線・スピードライン）、自由ベジェベクターシェイプ `ItemKind::VectorShape`（単色/線形グラデーション塗り・破線ストローク・トリムパスアニメーション）、3D カメラ空間のディレクショナル／ポイントライト＆ドロップシャドウキャッチャー（`Light3D`）、およびシャッター角度ベース サブフレーム合成モーションブラー（プレビュー軽量 / 書き出し高品質切替 `MotionBlurConfig`）を実装
* `ve-app` / `ve-mcp`: タイムライン「＋ パーティクル」「＋ ベクター」ボタン、専用インスペクタ、3D ライト＆モーションブラー UI、`add_object` 拡張および MCP ツール `set_scene_motion_blur` を追加（全 **50 MCP ツール**）

## [0.8.0] - 2026-11-21

爆速カット・音ハメ編集・プロ級カラーグレーディングを統合する v0.8 ロードマップ（#11〜#13）をすべて実装しました。

### #11 テキストベース編集（文字起こし連動リップルカット ＆ フィラー自動除去）
* `ve-core`: `text_edit` モジュールを追加（`extract_transcript_entries`・日本語/英語フィラー検出 `analyze_fillers`・語句掃除 `build_strip_filler_words_commands`・連動レイヤー一括リップルカット `build_text_based_delete_commands`・隣接区間スワップ並べ替え `build_text_based_reorder_commands`、すべて 1 ステップ Undo 対応）
* `ve-app`: `AudioAi3DPanel` に「9. テキストベース編集」パネルを追加（クリックシーク、複数選択一括カット、⬆/⬇ 区間入替、フィラー一括カット＆セリフ内語句掃除）
* `ve-mcp`: MCP ツール `text_based_edit` を追加

### #12 BGM ビート自動検出 ＆ ビートスナップ編集
* `ve-audio`: Pure-Rust オンセット包絡＋自己相関 BPM 推定＋位相アライメント（`detect_audio_beats` / `detect_beats_from_waveform`）を実装
* `ve-core`: ビート自動カット（`build_beat_cut_commands`）・最寄りビート整列（`build_snap_to_beats_commands`）・小節/全拍マーカー配置（`build_beat_markers_command`）を実装
* `ve-app` / `ve-mcp`: タイムライン「🥁 ビート検出」バー・ビートグリッド縦線描画（小節頭ゴールド / 拍シアン）・ドラッグ/トリムのビート自動吸着、および MCP ツール `detect_beats` を追加

### #13 `.cube` 3D LUT 対応 ＆ 3-Way カラーグレーディング ＆ リアルタイムビデオスコープ
* `ve-core`: 非破壊 `SceneColorGrade`（露出・コントラスト・彩度・色温度・ティント・3-Way Lift/Gamma/Gain・`.cube` LUT パス/強度）と `EditCommand::SetSceneColorGrade`、`ProjectFolder::luts_dir()` を追加
* `ve-render`: Adobe / DaVinci 標準 `.cube` 3D LUT パーサ＆四面体補間 `CubeLut3D::sample_tetrahedral`、組み込みシネマルック 3 種＋サンプル `.cube` 生成 `emit_sample_cube_luts`、`RenderGraph` 最終段への統合（プレビュー／RAM／書き出し／MCP 完全一致）、および高速ビデオスコープ解析 `compute_video_scopes`（Waveform / RGB Parade / Vectorscope / RGBY Histogram）を実装
* `ve-app` / `ve-mcp`: ドラッグ操作可能な 3-Way カラーホイール＆`.cube` ピッカー UI、プレビュー連動リアルタイム 4 分割ビデオスコープパネル、および MCP ツール `set_color_grade` を追加（全 47 MCP ツール）

## [0.7.1] - 2026-11-21

v0.3〜v0.4 の残課題（プリセット管理・専用インスペクタ・ショートカット・マーカー編集・UI 残骸整理・ドキュメント最新化）をすべて補完しました。

### 字幕スタイル＆エフェクトチェーン プリセット管理（`presets/*.json`）＋ 影色・座布団色 RGBA ピッカー
* `ve-core`: `TextStylePreset`（4 種の標準 YouTube/ゆっくり/シネマプリセット）と `EffectPreset::default_presets()`（3 種の標準エフェクトチェーン）を追加し、`ProjectFolder` に `presets/text_styles/*.json`・`presets/effects/*.json` の安全な保存・読込・削除（パストラバーサル防止）を実装
* `ve-app`: 字幕装飾インスペクタへ影色（`shadow_color`）・座布団色（`box_color`）の RGBA カラーピッカーおよび 1 クリックプリセット適用・保存・削除 UI を追加。映像エフェクト欄へエフェクトチェーンプリセット保存・適用・削除 UI を追加
* `ve-mcp`: MCP ツール `manage_presets`（`list` / `apply_text_style` / `apply_effect_preset` / `save_text_style` / `save_effect_preset`）を追加（全 44 ツール）

### フィルタオブジェクト・グループ制御・オーディオスペクトラム専用インスペクタ
* `ve-core`: Undo 対応コマンド `EditCommand::SetFilterObject` / `SetGroupControl` / `SetAudioSpectrum` を追加
* `ve-app`: `ItemKind::FilterObject`（組み込み＋プラグイン種別・適用レイヤー範囲・パラメータ）、`ItemKind::GroupControl`（対象レイヤー数）、`ItemKind::AudioSpectrum`（連動音声レイヤー・サイズ・バー本数・感度・基本色/ピーク色）の専用編集 UI をインスペクタへ統合

### タイムライン操作ショートカット補完 ＆ マーカー編集 UI
* `ve-core` / `ve-mcp`: `EditCommand::UpdateMarker` および MCP `manage_markers` の `update` アクション（ラベル・`color_hex` 更新）を追加
* `ve-app`: タイムラインへ `Ctrl+C`（コピー）、`Ctrl+V`（再生ヘッド位置へ相対オフセット維持で貼り付け）、`Ctrl+D`（複製）、`Ctrl+A`（全選択）、`Backspace`（削除 / `Shift+Backspace` リップル削除）、`Home`/`End`（先頭・末尾ジャンプ）を追加。ルーラー上のマーカークリックでラベル＆5 色パレットをその場で編集可能に拡張

### UI 残骸クリーンアップ ＆ ドキュメント最新化
* `ve-app`: 旧 Phase 1a / Phase 2 の仮文言（`"Phase 2 Hybrid Motion Project"`、`"日本語IME PoC 検証 (Phase 1a)"` および未使用 1 行テスト欄）を本番向けの簡潔な字幕エディタ＋折りたたみ IME 診断ログへ整理
* `README.md`: v0.7.1 現在の全機能・44 MCP ツール・WASM プラグイン・GitHub Releases ビルド情報へ最新化

## [0.7.0] - 2026-11-21

上級映像表現＆エコシステム拡張の v0.7 ロードマップ（#8〜#10）を全実装し、全 10 項目のロードマップを 100% 完了しました。

### #8 自由ベジェマスク ＆ 自動モーショントラッキング（顔モザイク・テロップ追従）
* `ve-core` / `ve-render`: 3次ベジェ閉曲線マスク `BezierMask`（楕円/矩形/ひし形プリセット・加算/減算・反転・2パス分離フェザー・`EditCommand::SetMasks` 1ステップ Undo 対応）とスキャンラインラスタライザ `apply_bezier_masks` を実装
* `ve-app`: プレビューキャンバス上でのベジェマスク輪郭描画および頂点・接線ハンドルの直接ドラッグ編集、インスペクタ編集 UI を追加
* `ve-render` / `ve-core`: Pure-Rust 1ポイント自動モーショントラッカー `track_point_in_scene`（粗→密テンプレートマッチング＋サブピクセル補間）と、結果を標準 `x`/`y` キーフレームへ焼き込む `build_motion_track_keyframes_command` を実装
* `ve-mcp`: MCP ツール `set_bezier_mask` を追加

### #9 WASM（WebAssembly）動的プラグインロード
* `ve-plugin-api`: Pure-Rust・ゼロ `unsafe`・ゼロ追加依存の WebAssembly 1.0 サンドボックス VM（`WasmModule` / `WasmInstance`：ホストインポート禁止・4MiB メモリ境界検証・命令フューエル上限保護）を実装
* `ve-plugin-api`: `plugins/*.wasm` の動的ロード・自動ホットリロード・破損時ロールバックを担う `WasmPluginManager` と、実行可能な実 `.wasm` サンプル 3 種生成（`emit_sample_wasm_plugins`）を実装
* `ve-app` / `ve-mcp`: 500ms ホットリロード監視、インスペクタ WASM プラグイン管理 UI、バックグラウンド書き出し時の自動ロード、MCP ツール `list_or_reload_wasm_plugins` を追加

### #10 AI ハイライト自動抽出 ＆ ショート動画自動切り抜き
* `ve-core`: 音声エネルギーピーク・発話/字幕密度・強調キーワード・マーカーを複合スコアリングして非重複（NMS）抽出する `extract_highlight_candidates` を実装
* `ve-core`: 抽出候補から **1080×1920（9:16）ショート動画シーン** を自動トリム（`source_offset`/キーフレーム補正）＆セーフゾーン 13 文字改行リレイアウトする `build_shorts_clip_scene_command`（`EditCommand::CreateScene` 1ステップ Undo）を実装
* `ve-app` / `ve-mcp`: `AudioAi3DPanel` のハイライト抽出＆1クリック 9:16 ショート生成 UI および MCP ツール `extract_highlights` を追加

## [0.6.0] - 2026-11-21

解説・実況・ショート動画の量産性を飛躍させる v0.6 ロードマップ（#4〜#7）をすべて実装しました。

### #4 BGM 自動ダッキング（サイドチェイン音量減衰）＆ オーディオリアクティブ
* `ve-audio`: 発話レイヤー（サイドチェイン）のエンベロープに応じて BGM レイヤーを滑らかに自動減衰する `apply_sidechain_ducking` を `AudioGraph` へ統合（プレビュー再生＆書き出し完全一致）
* `ve-audio`: Pure-Rust Hann 窓付き Goertzel 対数帯域スペクトラム解析 `analyze_spectrum_bands`（Low / Mid / High / RMS）を実装
* `ve-core` / `ve-render`: `ItemKind::AudioSpectrum`（音声連動リアルタイムスペクトラム描画）と `AudioReactiveConfig`（帯域エネルギー連動の拡大率・Yバウンス・不透明度アニメーション）、`EditCommand::SetAudioDucking` / `SetAudioReactive` を実装
* `ve-app` / `ve-mcp`: インスペクタの BGM ダッキング＆オーディオリアクティブ設定 UI、タイムライン「＋ スペクトラム」配置（Transform Gizmo 操作対応）、MCP ツール `set_audio_ducking` を追加

### #5 台本（Markdown / CSV）→ タイムライン一括自動生成（ゆっくり・解説動画量産モード）
* `ve-core`: `parse_script_csv` / `parse_script_markdown` / `parse_script_auto` と、話者別自動配色・日本語自動改行・座布団字幕・ポーズ間調整を 1 ステップ Undo で組み立てる `build_script_timeline_commands` を実装
* `ve-app` / `ve-mcp`: `AudioAi3DPanel` の台本一括生成 UI（`.csv` / `.md` 読込、背景画像同時配置、任意でローカル TTS 実尺連携）および MCP ツール `import_script` を追加

### #6 SRT / ASS 字幕入出力 ＆ 他ソフト互換（AviUtl `.exo` / OpenTimelineIO）
* `ve-core`: Pure-Rust の SRT / ASS (v4+) 字幕インポート＆エクスポート、AviUtl `.exo` パーサ（UTF-16LE 16進 `text=` 日本語復号＋座標・色・拡大率・透明度・回転対応）、および OpenTimelineIO `.otio` JSON エクスポートを実装
* `ve-app` / `ve-mcp`: `AudioAi3DPanel` の字幕・互換入出力 UI および MCP ツール `import_subtitles_or_exo` / `export_subtitles_or_otio` を追加

### #7 レンダーキュー（バッチ連続書き出し）
* `ve-app`: マルチジョブ連続書き出しマネージャー `RenderQueueState` を実装（MP4 H.264 / 9:16 縦動画自動リフレーム MP4 / H.265 / WebM Alpha / WAV / SRT / ASS / OTIO、1 クリック「YouTube 一括セット追加」、並び替え・削除、順次バックグラウンド書き出し＆進捗表示・キャンセル対応）

## [0.5.0] - 2026-11-21

プレビュー直感操作・動画デコード高速化＆RAM先読み・縦動画プリセット／OSネイティブファイル選択／自動保存を実装しました。

### #1 プレビュー画面上の直接操作（Transform Gizmo ＆ ガイド線）
* `PreviewCanvasState`: プレビュー上の直接クリック選択、バウンディングボックス・4隅スケールハンドル（`Shift` で比率維持）・回転ハンドル（`Shift` で 15° スナップ）・中央＆セーフマージン吸着ガイド線・3分割グリッド・9:16 Shorts/TikTok UI 被り警告エリア・ホイールズーム＆中ドラッグパンを実装（すべて `EditCommand::SetTransform` Coalescing 1ステップ Undo 対応）
* `ve-render`: `Transform2D::rotation_deg` の線形空間バイリニア回転描画と `render_project_scene_scaled`（シーン座標とプレビュー描画の 1:1 一致）を実装

### #2 動画デコード高速化 ＆ RAM プレビュー（バックグラウンド先読みキャッシュ）
* `ve-media` / `ve-render`: 常駐 FFmpeg パイプによる連続フレームデコーダ `VideoStreamDecoder` を `RenderGraph` へ統合し、毎フレームプロセス起動を排除
* `ve-app`: 専用ワーカースレッド先読みキャッシュ `RamPreviewManager`（最大 96 フレーム LRU・リビジョン更新時自動破棄）と、タイムラインルーラー下への AE 風緑色キャッシュ済みバー描画を実装

### #3 シーン設定 UI（1080p / 4K / 9:16 縦動画）・OS ネイティブファイル選択・自動保存
* `ve-core` / `ve-mcp`: `ScenePreset`（720p / 1080p / 4K / 9:16 Shorts / 1:1 Square）および `EditCommand::SetSceneSettings`（自動リフレーム＋Undo 対応）、MCP ツール `set_scene_settings` を追加
* `ve-app`: シーン設定モーダルと `WgpuPreviewPipeline` の解像度動的リサイズ、ゼロ依存 OS ネイティブファイル／フォルダ選択ダイアログ（Windows PowerShell WinForms / Linux `zenity`・`kdialog`）、および `backups/` ローテーション付き定期自動保存＆復元メニューを実装

## [0.4.0] - 2026-11-21

ロードマップ残バックログ（#3・#5・#6・#7・#8）をすべて実装完了しました。

### #3 無音区間自動カット（ジェットカット / Jump Cut）
* `ve-core` に RMS 発話区間検出（`detect_speech_intervals_from_rms`）および連動レイヤー一括カット＆左詰めコマンド生成（`build_jump_cut_commands`・1 ステップ Undo 対応）を追加
* タイムラインに「⚡ 無音カット」設定バー（解析レイヤー・RMS 閾値・最小無音長・語頭/語尾余白・連動レイヤー選択）を追加し、実メディア `AudioGraph` と `WaveformPeaks` の双方に対応
* 内蔵 MCP ツール `auto_jump_cut` を追加

### #5 カスタム WGSL シェーダー＆エフェクトの `wgpu` GPU オフロード
* `WgpuPreviewPipeline` に `Rgba16Float` Ping-Pong テクスチャ 2 枚と共通 48-byte `GpuEffectUniform`（`params[8]`, `resolution`, `time_sec`）を追加し、組み込み色調補正・2-Pass 分離ブラー・グロー・クリッピングおよびホットリロード済みカスタム WGSL パイプラインを GPU 実行
* ホットリロード成功時に `wgpu::RenderPipeline` を自動再構築し、検証エラー時は直前の正常パイプラインを保持
* オフスクリーン書き出し向け `HeadlessGpuOffloader` を実装（GPU 非搭載環境では CPU `RenderGraph` へ自動フォールバック）

### #6 Claude 字幕文脈補正 UI（差分確認）＆ MCP ワークフロー強化
* `AudioAi3DPanel` に「✨ Claude で字幕を文脈補正」（`CloudUploadGuard` 付き）およびオフライン「📝 自動改行プレビュー」と Before / After 差分確認・個別編集・1 ステップ Undo 一括適用 UI を追加
* インスペクタに「📋 MCPクライアント設定JSONをコピー」ボタンを追加
* 内蔵 MCP に `auto_jump_cut`, `refine_subtitles`, `import_media`, `manage_markers`, `start_export`（確認ガード＆出力名サニタイズ付き）を追加

### #7 キーボードショートカット完全化＆タイムライン操作性向上
* `Space`（再生/一時停止）、`←`/`→`（`Shift` で 10 フレーム移動）、`J`/`K`/`L` シャトル再生（`-4x/-2x/-1x/0/1x/2x/4x`、`AudioMasterClock` ロックフリー原子レート対応）
* `Ctrl + マウスホイール` による再生ヘッド中心のタイムラインズーム
* `M` タイムラインマーカー追加（ラベル・色付き、ルーラー旗クリックでジャンプ／右クリック削除、YouTube チャプターテキスト一括コピー）
* 空トラックドラッグによるラバーバンド矩形選択、複数選択クリップ同時ドラッグ移動、リップル削除（`Shift+Delete`）

### #8 GitHub Releases 自動配布パイプライン
* `.github/workflows/release.yml` を追加し、`v*` タグプッシュ時に Windows 11 x64（`.zip`）および Ubuntu Linux x64（`.tar.gz`）の最適化リリースバイナリと SHA-256 チェックサムを自動公開

## [0.3.0] - 2026-11-21

* AviUtl2 本体実用機能（A-1〜A-6: 名前付きカラーパレット、インライン制御文字＆補完、レイヤー常駐設定、シーンチェンジ、再生速度・逆再生・ループ、実務頻出フィルタ）
* 人気プラグイン相当機能（B-1〜B-7: 整列・隙間詰め・ミニマップ、フォント視覚化ピッカー、音声 DSP＆LUFS 正規化、エイリアス棚、テキスト検索・一括置換、かんしくん風フォルダ監視、演出プリセット）
* 字幕リッチ装飾（縁取り・ドロップシャドウ・座布団背景）

## [0.2.0] - 2026-11-21

* コードレビュー最適化・セキュリティ強化（PR #5）
* 実メディア動画・画像・PSD の本格描画＆軽量プロキシ自動生成＆バックグラウンド非同期書き出し（PR #6）

## [0.1.0] - 2026-11-21

仕様書 v1.1（[docs/spec.md](docs/spec.md)）の全ロードマップ（Phase 1a 〜 Phase 4）を実装しました。

### Phase 1a — コアエンジン PoC
* 有理数タイムベース `RationalTime`（705,600,000 ticks/sec）および 1フォルダ管理（`project.json` 原子的保存・`history.jsonl`・アセット参照＆プロジェクト収集）
* FFmpeg / FFprobe 外部 CLI サイドカーによるプローブ・デコード・書き出しと `symphonia` 音声デコード
* プレビュー／書き出し共通 `AudioGraph`・`AudioMasterClock`（A/V同期）・Windows `cpal` 出力
* Linear sRGB（`Rgba16Float`）+ Premultiplied Alpha 合成、`cosmic-text`+`swash` 日本語テキスト描画、`wgpu` プレビューパイプライン、`egui` 日本語 IME PoC パネル

### Phase 1b — 実用 NLE 土台と AI 接続
* `CommandBus`（GUI/MCP 共通実行・Undo/Redo・Command Coalescing・GUI ドラッグ排他ロック・`history.jsonl` クラッシュ復旧）
* AviUtl2 風インタラクティブタイムライン（ドラッグ移動・右端トリム・スナップ・分割・複数選択・音声波形描画）およびオフラインアセット再リンク UI
* 内蔵 MCP サーバー（ローカル Streamable HTTP `127.0.0.1` + 起動時ベアラートークン認証、`render_frame_preview`・`batch_execute` 等）
* GitHub Actions CI（Ubuntu `fmt`/`clippy`/テスト + Windows 11 x64 ビルド＆テスト）

### Phase 2 — AE 融合とワンクリック結合
* AviUtl2 中間点／AE キーフレーム統一モデルと 3次ベジェ曲線（値グラフ／速度グラフ＆ドラッグ可能なベジェハンドル）
* 2空間 UI 切替（メイン編集空間／モーション合成空間）および Smart Precompose（ワンクリック結合・ダブルクリック非破壊再編集・コンテンツハッシュ＆LRU キャッシュ管理）
* グループ制御・トラックマット（Alpha/Luma＋反転）・ブレンドモード・文字ごとアニメーター・基本 WGSL エフェクトチェーン（ブラー／グロー／色調補正／クリッピング）・透過書き出し（WebM VP9+Alpha / PNG 連番）

### Phase 3 — 音声自動化・立ち絵・3D
* 自動文字起こし3モード（ローカル Whisper/VAD・Gemini API・Claude API 文脈補正）、OS 資格情報ストア（Windows Credential Manager 等）、送信前コスト＆尺確認ガード
* 合成音声（TTS）ファイルペア自動取込（`.wav`+`.txt`+`.lab`）および VOICEVOX / COEIROINK / A.I.VOICE ローカル API 連携
* PNG 差分立ち絵の自動口パク（音量 RMS 連動／母音音素連動・設定レス音声レイヤー選択）
* 3D カメラ制御（位置・注視点・FOV・被写界深度ボケ・3D 空間配置・Z ソート合成）

### Phase 4 — 拡張性・OSS 化
* PSD ネイティブ立ち絵対応（`psd` による非表示差分レイヤー含む直接合成・自動目パチ・口パク連動）
* カスタム WGSL シェーダーのホットリロード（`naga` 静的検証＋エラー時フォールバック保護）
* 公開プラグイン API クレート `ve-plugin-api`（`PluginManifest` / `EffectPlugin` / `TimelineGeneratorPlugin` / `PluginRegistry` + リファレンス実装ビネット＆セピアプラグイン）
* MIT / Apache-2.0 デュアルライセンス、アーキテクチャ設計書、プラグイン開発ガイド、E2E 統合テスト基盤
