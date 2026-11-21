# Changelog

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
