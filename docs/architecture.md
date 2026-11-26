# アーキテクチャ設計書 (`video-editing-app_AG` — v10.0.0)

## 1. ワークスペース依存関係（8 クレート・Pure Rust・ゼロ `unsafe`）

すべての機能を疎結合な **8 つの Rust クレート** に分離し、コアエンジン・プラグイン API・レンダラー・MCP サーバー・GUI を独立してテスト・再利用できる構成としています。

```mermaid
flowchart TD
    app["ve-app (eframe / egui_dock / Tri-Workspace + 6 Studio Hubs)"]
    mcp["ve-mcp (内蔵 MCP: Streamable HTTP / 10 ドメイン・全 100 ツール)"]
    render["ve-render (wgpu / Linear f16 / 8 WGSL Compute / 3D PBR + IBL + 流体/360°)"]
    ai["ve-ai (Whisper / Gemini / Claude / TTS / Keyring)"]
    audio["ve-audio (AudioGraph / 修復 / ADR / 5.1・FOA Ambisonics)"]
    media["ve-media (FFmpeg CLI / HW Decode / RTMP・SRT / Symphonia)"]
    plugin["ve-plugin-api (公開 Plugin API + Pure-Rust WASM 1.0 VM)"]
    core["ve-core (RationalTime / Project / CommandBus / 区間木 / DAM / Cinema OS)"]

    app --> mcp
    app --> render
    app --> ai
    app --> audio
    app --> media
    app --> plugin
    app --> core

    mcp --> render
    mcp --> ai
    mcp --> plugin
    mcp --> core

    render --> audio
    render --> plugin
    render --> core

    ai --> media
    ai --> core

    audio --> media
    audio --> core

    media --> core
    plugin --> core
```

### 大規模モジュールの責務分割（v10.0.0）
* `crates/core/src/command/` — `mod.rs`（`EditCommand` / `CommandBus`）・`apply.rs`（決定論コマンド適用器）・`tests.rs`
* `crates/render/src/graph/` — `mod.rs`（`RenderGraph` コア合成＆並列バッチ描画）・`effects.rs`（エフェクトチェーン・トランジション・ブラー・回転）・`tests.rs`
* `crates/app/src/timeline/` — `mod.rs`（タイムライン描画＆操作）・`actions.rs`（ミニマップ・分割・コピペ・ジャンプカット・ビート検出・ヘルパー）・`tests.rs`
* `crates/mcp/src/tools/` — 10 ドメイン別サブモジュール（`edit_nle` / `vfx_motion` / `color_finishing` / `audio_voice` / `ai_collab` / `deliver_system` / `v70_unified` / `v80_cinema_audio` / `v90_broadcast_ecosystem` / `v100_spatial_cinema_os`）

## 2. 時間表現（有理数タイムベース）
* `ve_core::RationalTime` は `TICKS_PER_SECOND = 705_600_000`（Flicks）を採用しています。
* 24 / 23.976 / 29.97 / 30 / 59.94 / 60 / 120fps の映像フレームおよび 44.1kHz / 48kHz / 96kHz の音声サンプル位置を丸め誤差ゼロの整数で表現できます。

## 3. Command Bus と耐障害性
* GUI・ハードウェアコントローラー（OSC/MIDI/Stream Deck）・内蔵 MCP からの編集はすべて `ve_core::CommandBus::execute` を経由します。
* **Command Coalescing:** タイムラインドラッグ移動やスライダー連続操作は `coalesce_key` により 1 つの Undo 単位へ自動統合されます。
* **GUI ドラッグ排他ロック:** ユーザーがタイムラインやベジェハンドルをドラッグ中は `CommandSource::Mcp` を排他ブロックします。
* **クラッシュ復旧:** 実行成功したコマンドはプロジェクトフォルダの `history.jsonl` に即時追記・`sync_data()` され、クラッシュ後も `ProjectFolder::read_history` ＋ `CommandBus::replay_history` で完全復旧できます。

## 4. レンダリングパイプライン・WGSL Compute・カラーマネジメント
1. **入力変換:** すべての素材（PNG / PSD / テキストグリフ / 映像フレーム）は入口で **Linear sRGB + Premultiplied Alpha**（`Rgba16Float` 4K/8K フレームプール対応）へ変換します。
2. **評価順序:**
   * Step 1: 10,000+ クリップ区間木 $O(\log N)$ クエリ → キーフレーム＆エクスプレッション評価 → 立ち絵口パク・VTuber 2D メッシュ・2D Navier-Stokes 流体・PBD クロス・3D PBR（2 パス PCF ソフトシャドウマップ＆HDRI IBL） → 個別エフェクト＆WASM プラグイン＆ロトブラシ
   * Step 2: トラックマット（Alpha / Luma ＆各反転） → グループ制御 → AviUtl2 風タイムラインフィルタオブジェクト → シーンチェンジ
   * Step 3: 最終合成（3D Z ソート / ノード合成グラフ） → シネマ光学 VFX → ACEScg/OCIO＆3D LUT → 35mm フィニッシング → 360° 全天球（OverCapture / Tiny Planet / 水平ロック）＆ ステレオスコピック空間 3D パス
3. **組み込み WGSL Compute シェーダー（全 8 本・Naga 静的検証済・決定論 CPU フォールバック付）:**
   * `effects.wgsl` (v0.2) / `composite_compute.wgsl` (v1.0) / `color_grade_compute.wgsl` ＆ `video_scopes_compute.wgsl` (v3.0) / `fast_blend_mask.wgsl` ＆ `fast_aces_color_mgmt.wgsl` (v7.0) / `spatial_360_equirect.wgsl` ＆ `fluid_2d_advection.wgsl` (v10.0)

## 5. 内蔵 MCP サーバー仕様（全 100 ツール / 10 ドメイン）
* バインド先: `127.0.0.1:<ephemeral_port>`（`POST /mcp`、Streamable HTTP / JSON-RPC 2.0）
* 認証: アプリ起動ごとにランダム生成される `Authorization: Bearer <token>` が必須
* 公開ツール（全 **100 種** / **10 ドメイン**）: タイムライン NLE・VFX/モーション・カラー/3D LUT/ACEScg・音声 DSP/修復/ADR/Ambisonics・AI ビジョン/ディレクター/CRDT・納品/WASM/QC・6 スタジオ Hub/GPU プール/区間木/A/B 比較・平面追跡/ロト/SVG ワープ/光学 VFX・ライブスイッチャー/VTuber VMC/OSC・MIDI・StreamDeck/スマート DAM・360° 空間 3D/流体・クロス・HDRI IBL/BVH・シルエットモキャップ/LAN 分散レンダーファーム/自律 AI シネマプロデューサーまで完全網羅。
