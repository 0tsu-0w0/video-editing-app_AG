# 機能追加ロードマップ（Future Features Roadmap）

> 更新日: 2026-11-21  
> **v0.1〜v0.4 完了済み**: コアエンジン・AE融合・音声AI/立ち絵/3D・PSD/WGSL/プラグイン・最適化/セキュリティ・実メディア描画＆非同期書き出し・AviUtl2実用機能群・無音自動カット・`wgpu` GPUオフロード・Claude差分UI＆MCP強化・キーボードショートカット完全化・GitHub Releases 自動配布パイプライン。  
> 以下に **v0.5〜v0.7+ の全 10 実装案** を優先度順に整理しています。

---

## 🥇 v0.5（✅ 実装済み）：日常編集のボトルネック解消・直感操作

### 1. ~~プレビュー画面上の直接操作（Transform Gizmo ＆ ガイド線）~~ ✅ 実装済み（v0.5）
- **目的**: 位置・拡大縮小・回転を数値入力に頼らずプレビューキャンバス上で直感操作できるようにする。
- **実装内容**:
  - プレビュー上の直接クリックによるアイテム選択（ヒットテスト）
  - 選択中アイテムのバウンディングボックス・四隅スケールハンドル（`Shift` で縦横比維持）・回転ハンドル（`Shift` で 15° スナップ）・中心アンカー表示
  - 中央スナップ＆セーフマージン吸着ガイド線（シアン）、アクション/タイトルセーフ枠、3分割グリッド、縦動画（9:16）向け Shorts/TikTok UI 被り警告ガイド
  - プレビューのズーム（ホイール / フィット / 50% / 100% / 150% / 200%）＆中ドラッグ／Altドラッグパン
  - `ve-render`: `Transform2D::rotation_deg` の線形空間バイリニア回転描画と、`render_project_scene_scaled` によるシーン解像度→プレビュー解像度の 1:1 座標スケーリング
  - すべてのドラッグ編集を `EditCommand::SetTransform`（Coalescing 1ステップ Undo）で実行

### 2. ~~動画デコード高速化 ＆ RAM プレビュー（バックグラウンド先読みキャッシュ）~~ ✅ 実装済み（v0.5）
- **目的**: 1080p / 4K 60fps 動画の連続再生・スクラブ時のフレーム落ちを劇的に軽減する。
- **実装内容**:
  - `ve-media`: 常駐 FFmpeg パイプによる連続デコーダ `VideoStreamDecoder` を追加し、連続フレーム再生時の毎フレームプロセス起動を排除
  - `ve-render`: `RenderGraph` に常駐動画ストリームキャッシュを統合
  - `ve-app`: 専用ワーカースレッドによるバックグラウンド先読みリングキャッシュ `RamPreviewManager`（AE 風の緑色キャッシュ済みバーをタイムラインルーラー下へリアルタイム表示、編集リビジョン変更時は即座に破棄・再先読み）

### 3. ~~シーン設定 UI（1080p / 4K / 9:16 縦動画プリセット）・OS ネイティブファイル選択・自動保存~~ ✅ 実装済み（v0.5）
- **目的**: YouTube 本編・Shorts/TikTok 縦動画・正方形 SNS 動画への即座の切替、パス手入力からの脱却、および長時間編集時の自動バックアップ保護を実現する。
- **実装内容**:
  - `ve-core`: `ScenePreset`（720p / 1080p / 4K / 9:16 Shorts / 1:1 Square）および `EditCommand::SetSceneSettings`（比率変更時のアイテム座標・サイズ自動リフレーム＋1 ステップ Undo 対応）
  - `ve-app`: シーン設定モーダル、`WgpuPreviewPipeline` の解像度動的リサイズ＆実アスペクト比描画、純 Rust ゼロ依存 OS ネイティブファイル／フォルダ選択ダイアログ（Windows 11 PowerShell WinForms / Linux `zenity`・`kdialog`）、および `backups/` ローテーション付き自動保存（Auto-Save）＆復元メニュー
  - `ve-mcp`: AI エージェントからも解像度・縦動画リフレーム可能な `set_scene_settings` ツールを追加

---

## 🥈 v0.6（✅ 実装済み）：解説・実況・ショート動画の生産性を跳ね上げる機能

### 4. ~~BGM 自動ダッキング（サイドチェイン音量減衰）＆ オーディオリアクティブ~~ ✅ 実装済み（v0.6）
- **実装内容**:
  - `ve-audio`: 発話レイヤー（サイドチェイン入力）の高速エンベロープ検出に応じて BGM レイヤーを滑らかに自動減衰させる `apply_sidechain_ducking` DSP を `AudioGraph` へ統合（プレビュー再生＆書き出し完全一致）
  - `ve-audio`: Pure-Rust Hann窓付き Goertzel 対数帯域スペクトラム解析器 `analyze_spectrum_bands`（Low / Mid / High / RMS）
  - `ve-render` / `ve-core`: `ItemKind::AudioSpectrum`（音声連動リアルタイムスペクトラムバー描画）と `AudioReactiveConfig`（帯域エネルギーに応じた拡大率・Yバウンス・不透明度の自動連動アニメーション）
  - `ve-app` / `ve-mcp`: インスペクタのダッキング＆オーディオリアクティブ設定 UI、タイムライン「＋ スペクトラム」配置（Transform Gizmo 対応）、MCP `set_audio_ducking`

### 5. ~~台本（Markdown / CSV）→ タイムライン一括自動生成（ゆっくり・解説動画量産モード）~~ ✅ 実装済み（v0.6）
- **実装内容**:
  - `ve-core`: `parse_script_csv`（`話者,セリフ,表情,背景画像`）/ `parse_script_markdown`（`**霊夢** [smile]: セリフ`, `![bg](path)`）/ `parse_script_auto` と、話者別自動配色・日本語自動改行・座布団字幕・ポーズ間調整・背景画像配置を 1 ステップ Undo 可能な `EditCommand::Batch` へ組み立てる `build_script_timeline_commands`
  - `ve-app` / `ve-mcp`: `AudioAi3DPanel` の台本一括生成 UI（`.csv`/`.md` 読込、任意で `LocalTtsClient` 実尺連携）および MCP `import_script`

### 6. ~~SRT / ASS 字幕入出力 ＆ 他ソフト互換（AviUtl `.exo` / OpenTimelineIO）~~ ✅ 実装済み（v0.6）
- **実装内容**:
  - `ve-core`: Pure-Rust SRT / ASS (v4+) 字幕インポート＆エクスポート（`parse_srt_subtitles` / `export_srt_subtitles` / `parse_ass_subtitles` / `export_ass_subtitles`）
  - `ve-core`: AviUtl `.exo` パーサ `parse_aviutl_exo`（AviUtl 固有の UTF-16LE 16進 `text=` 日本語復号＋X/Y/拡大率/透明度/回転対応）および OpenTimelineIO `.otio` JSON エクスポート `export_otio_json`
  - `ve-app` / `ve-mcp`: `AudioAi3DPanel` の一括入出力 UI および MCP `import_subtitles_or_exo` / `export_subtitles_or_otio`

### 7. ~~レンダーキュー（バッチ連続書き出し）~~ ✅ 実装済み（v0.6）
- **実装内容**:
  - `ve-app`: `RenderQueueState`（MP4 H.264 / 9:16 縦動画自動リフレーム MP4 / H.265 / WebM Alpha / WAV / SRT / ASS / OTIO の複数ジョブ登録、YouTube 一括セット追加、並び替え・削除、順次バックグラウンド連続実行、全体＆個別進捗バー、安全キャンセル）

---

## 🥉 v0.7（✅ 実装済み）：上級映像表現 ＆ エコシステム拡張

### 8. ~~自由ベジェマスク ＆ 自動モーショントラッキング（顔モザイク・テロップ追従）~~ ✅ 実装済み（v0.7）
- **実装内容**:
  - `ve-core` / `ve-render`: 3次ベジェ閉曲線マスク `BezierMask`（楕円/矩形/ひし形プリセット・加算/減算・反転・2パス分離フェザーぼかし、`EditCommand::SetMasks` 1ステップ Undo 対応）
  - `ve-app`: プレビューキャンバス上のベジェマスク輪郭描画と頂点・接線ハンドルの直接ドラッグ編集（`Shift` で対称ハンドル）＋ インスペクタ編集 UI
  - `ve-render` / `ve-core`: Pure-Rust 1ポイント自動モーショントラッカー `track_point_in_scene`（粗→密テンプレートマッチング＋放物線サブピクセル補間）と、追跡結果を標準 `x`/`y` キーフレームへ焼き込む `build_motion_track_keyframes_command`
  - `ve-mcp`: AI エージェントからマスク操作できる `set_bezier_mask` ツールを追加

### 9. ~~WASM（WebAssembly）動的プラグインロード~~ ✅ 実装済み（v0.7）
- **実装内容**:
  - `ve-plugin-api`: Pure-Rust・ゼロ `unsafe`・外部依存ゼロの WebAssembly 1.0 バイナリインタプリタ VM `WasmModule` / `WasmInstance`（ホストインポート禁止・4MiB 線形メモリ境界チェック・命令フューエル上限による無限ループ遮断）
  - `ve-plugin-api`: `plugins/*.wasm` の動的ロード＆自動ホットリロード＆破損時自動ロールバックを担う `WasmPluginManager` と、即座に試せる実バイナリ `.wasm` 3種（色反転・ウォームティント・グレースケール）のビルダー `emit_sample_wasm_plugins`
  - `ve-app` / `ve-mcp`: 500ms 自動ホットリロード・インスペクタ WASM プラグイン管理 UI・バックグラウンド書き出し時の自動ロード・MCP `list_or_reload_wasm_plugins`

### 10. ~~AI ハイライト自動抽出 ＆ ショート動画自動切り抜き~~ ✅ 実装済み（v0.7）
- **実装内容**:
  - `ve-core`: 音声エネルギーピーク・発話/字幕密度・強調キーワード・マーカーを複合スコアリングし NMS で非重複抽出する `extract_highlight_candidates`
  - `ve-core`: 選択候補から **1080×1920（9:16）ショート動画シーン** を自動トリム（`source_offset`・キーフレーム補正）＆セーフゾーン 13 文字改行リレイアウトする `build_shorts_clip_scene_command`（`EditCommand::CreateScene` 1ステップ Undo 対応）
  - `ve-app` / `ve-mcp`: `AudioAi3DPanel` のハイライト抽出＆1クリック 9:16 ショート生成 UI および MCP `extract_highlights`

---

## ✅ 実装完了アーカイブ（v0.3〜v0.4）

- **v0.3**: AviUtl2 本体実用機能 A-1〜A-6（カラーパレット・インライン制御文字・レイヤー常駐設定・シーンチェンジ・再生速度/逆再生/ループ・実務頻出フィルタ）、人気プラグイン相当機能 B-1〜B-7（整列/隙間詰め/ミニマップ・フォントピッカー・音声DSP/LUFS・エイリアス棚・検索置換・フォルダ監視・演出プリセット）、字幕リッチ装飾（#4）
- **v0.4**: 無音区間自動カット（#3）、`wgpu` Ping-Pong GPU オフロード＆ヘッドレス書き出し（#5）、Claude 字幕差分 UI＆MCP ツール拡張（#6）、キーボードショートカット＆マーカー＆ラバーバンド選択＆一括移動＆リップル削除（#7）、GitHub Releases 自動配布パイプライン（#8）
