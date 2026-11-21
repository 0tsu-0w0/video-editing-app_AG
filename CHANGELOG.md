# Changelog

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
