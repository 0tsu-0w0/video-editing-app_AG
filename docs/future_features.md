# 今後の機能追加ロードマップ（Future Features Backlog）

> 更新日: 2026-11-20  
> Phase 1〜4・最適化/セキュリティ強化（PR #5）・実メディア描画＆非同期書き出し（PR #6）に続き、**v0.3（AviUtl2 本体実用機能 A-1〜A-6 ＋ 人気プラグイン相当機能 B-1〜B-7 ＋ #4 字幕リッチ装飾）**を実装完了。以下に未着手の将来候補を保持しています。

---

## 3. 無音区間自動カット（ジェットカット / Jump Cut）

### 目的
解説動画・実況動画・トーク動画の編集において最も時間を要する「無音・間延び区間の手動カット」を 1 アクションで自動化する。

### 設計メモ・既存フック
- **既存資産**:
  - `ve-ai`: `segment_audio_by_vad(&AudioBuffer, rms_threshold, min_segment_sec)`（`crates/ai/src/transcribe.rs`）
  - `ve-audio`: `WaveformPeaks` の `rms` バケット列（`crates/audio/src/waveform.rs`）
  - `ve-core`: `EditCommand::Batch`（複数分割＋移動を 1 回の `Ctrl+Z` Undo 単位で実行可能）
- **実装イメージ**:
  - パラメータ: `rms_threshold`（無音判定しきい値）、`min_silence_sec`（カット対象の最小無音尺）、`pad_before_sec` / `pad_after_sec`（語頭・語尾の食い込み防止マージン）、`ripple_linked_layers`（連動して詰める映像・字幕レイヤー指定）。
  - 実行経路: GUI（タイムラインツールバー「⚡ 無音カット」）および内蔵 MCP ツール `auto_jump_cut` の両方から `CommandBus` 経由で実行。

---

## 4. ~~字幕リッチ装飾（縁取り・ドロップシャドウ・座布団背景）＆ プリセット管理~~ ✅ 実装済み（v0.3）

- `TextStyle`（縁取り・ドロップシャドウ・角丸座布団背景）、AviUtl2 インライン制御文字（`<#RRGGBB>`, `<@パレット名>`, `<s64>`, `<fFont>`, `<b1>`, `</>`）、名前付きカラーパレット、および `aliases/*.json` エイリアス棚として実装完了。

---

## 5. カスタム WGSL シェーダー＆エフェクトの `wgpu` GPU オフロード

### 目的
1080p / 4K 60fps での重いエフェクト（大半径ブラー・グロー・色調補正・ユーザー自作 `.wgsl`）を CPU 処理から GPU パイプラインへ完全移行する。

### 設計メモ・既存フック
- **既存資産**:
  - `ve-render`: `ShaderHotReloader::build_wgpu_module`（`crates/render/src/hot_shader.rs`）
  - `ve-render`: `WgpuPreviewPipeline`（`crates/render/src/pipeline.rs`）
  - `ve-render`: 組み込み `shaders/effects.wgsl`、サンプル `shaders/custom_chromatic_aberration.wgsl`
- **実装イメージ**:
  - `WgpuPreviewPipeline` に `Rgba16Float` の Ping-Pong テクスチャ 2 枚と共通 Uniform バッファ（`resolution`, `time_sec`, `params[8]`）を追加。
  - ホットリロード成功時に `wgpu::RenderPipeline` を自動再構築し、エラー時は直前の正常パイプラインを保持。
  - オフスクリーン書き出し時もヘッドレス `wgpu::Device` 経由で GPU レンダリング結果をリードバック可能にする（GPU 非搭載環境では現行 CPU `RenderGraph` へ自動フォールバック）。

---

## 6. Claude 字幕文脈補正 UI（差分確認）＆ MCP ワークフロー強化

### 目的
Whisper / Gemini での一次書き起こし結果を Claude API で自然な日本語・読みやすい改行幅へ仕上げ、外部 AI エージェント（Claude Desktop / Cursor / Codex 等）との連携障壁を極小化する。

### 設計メモ・既存フック
- **既存資産**:
  - `ve-ai`: `ClaudeRefiner::refine_subtitles`（`crates/ai/src/transcribe.rs`）、`CloudUploadGuard`（`crates/ai/src/guard.rs`）
  - `ve-mcp`: `EmbeddedMcpServer` / `tools.rs`（`crates/mcp/src/`）
- **実装イメージ**:
  - `AudioAi3DPanel` に「✨ Claude で字幕を文脈補正」ボタンと Before / After 差分プレビュー＆一括適用（`EditCommand::Batch` で Undo 対応）を追加。
  - インスペクタに「MCP クライアント設定 JSON をコピー」ボタンを追加（現在の `127.0.0.1:<port>/mcp` と一時トークンをフォーマット済みでコピー）。
  - MCP ツール拡張: `refine_subtitles`, `import_media`, `auto_jump_cut`, `start_export`（確認ガード付き）。

---

## 7. キーボードショートカット完全化＆タイムライン操作性向上

### 目的
AviUtl2 / Premiere / DaVinci 経験者がマウスを持ち替えずに高速カット編集できる操作感を実現する。

### 設計メモ・既存フック
- **既存資産**:
  - `ve-app`: `TimelineState::show`（`crates/app/src/timeline.rs`、テキストフォーカス時は `egui_wants_keyboard_input()` でショートカット抑止済み）
- **実装イメージ**:
  - `Space`: 再生 / 一時停止トグル
  - `←` / `→`（`Shift` で 10 フレーム）: フレーム単位ステップ移動
  - `J` / `K` / `L`: 逆再生・停止・倍速再生（1x / 2x / 4x）
  - `Ctrl + マウスホイール`: 再生ヘッド中心のタイムラインズーム
  - `M`: タイムラインマーカー追加（ラベル・色付き、YouTube チャプターテキスト自動出力機能付き）
  - 複数クリップの矩形選択（ラバーバンド選択）と一括移動・リップル削除

---

## 8. GitHub Releases 自動配布パイプライン（Windows 11 / Linux）

### 目的
OSS 公開・自分用デプロイの双方で、タグを打つだけで最新の最適化済みネイティブバイナリを即座に配布できるようにする。

### 設計メモ・既存フック
- **既存資産**:
  - `.github/workflows/ci.yml`（Ubuntu + Windows 11 ビルド実績あり）
  - ルート `Cargo.toml` の `[profile.release]`（`lto = "thin"`, `codegen-units = 1`, `strip = "symbols"` 設定済み）
- **実装イメージ**:
  - `.github/workflows/release.yml` を追加（`push: tags: ['v*']` で起動）。
  - `windows-latest` で `cargo build --release -p ve-app` → `ve-app.exe` + `shaders/` + `LICENSE-*` + `README.md` を `ve-app-windows-x64.zip` へパッケージングし SHA-256 チェックサム付きで GitHub Release へ自動添付。
  - `ubuntu-latest` 向け tar.gz アーカイブも併せて公開。
