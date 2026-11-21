# Next-Gen Hybrid Video Editor (`video-editing-app_AG`)

After Effects の表現力、AviUtl2 の軽快なタイムライン編集、そして AI ネイティブ操作基盤（内蔵 MCP）を単一環境で融合する Pure Rust 動画編集ソフトです。

詳細な仕様は [docs/spec.md](docs/spec.md)（v1.1）を参照してください。

## ライセンス
本リポジトリのソースコードは **MIT OR Apache-2.0** のデュアルライセンスです。
FFmpeg は外部 CLI（サイドカー）または LGPL 動的リンクとして本体から分離しています。

## ワークスペース構成（Phase 1a + 1b + 2: コアエンジン・NLE・内蔵MCP・AE融合）

| クレート | 役割 |
| :--- | :--- |
| `crates/core` (`ve-core`) | 有理数タイムベース（`RationalTime`）、プロジェクトモデル（`Project` / `Scene` / `Layer` / `TimelineItem` / `FilterObject` / `GroupControl` / `TrackMatte` / `TextAnimator`）、キーフレーム＆3次ベジェ曲線（値・速度評価）、`PrecompCacheManager`（コンテンツハッシュ＆LRU制限）、1フォルダ管理、`CommandBus`（Undo/Redo・Command Coalescing・Smart Precompose・クラッシュ復旧） |
| `crates/media` (`ve-media`) | FFmpeg / FFprobe 外部CLIラッパー（プローブ・デコード・MP4 H.264/H.265・WebM VP9+Alpha透過・PNG連番透過書き出し）、`symphonia` 音声デコード |
| `crates/audio` (`ve-audio`) | プレビュー／書き出し共通 `AudioGraph`（異サンプルレート変換・ステレオミキサー）、`AudioMasterClock`（A/V同期）、`WaveformPeaks`（波形＆RMS解析）、Windows WASAPI（`cpal`）出力 |
| `crates/render` (`ve-render`) | Linear sRGB（`Rgba16Float`）+ Premultiplied Alpha 合成、`cosmic-text`+`swash` 文字ごとアニメーター、Render Graph（Precompハッシュキャッシュ・グループ制御・トラックマット・ブレンドモード・WGSLエフェクトチェーン（ブラー/グロー/色調補正/クリップ）・AviUtl2フィルタオブジェクト）、`wgpu` プレビューパイプライン |
| `crates/mcp` (`ve-mcp`) | 内蔵 MCP サーバー（ローカル Streamable HTTP `127.0.0.1` + 起動時ベアラートークン認証、`render_frame_preview`・`set_keyframe`・`precompose_selection`・`set_track_matte`・`batch_execute` 等18ツール） |
| `crates/app` (`ve-app`) | `eframe`（`wgpu`）+ `egui_dock` デスクトップアプリ、2空間UI切替（メイン編集空間／AEモーション合成空間）、ワンクリック結合＆ダブルクリック再編集、グラフエディタ（値/速度グラフ・ベジェハンドル）、透過書き出しUI、日本語IME PoC |

## ビルド・テスト

```bash
# 全クレートのコンパイル確認
cargo check --workspace

# ユニット・統合テスト実行（FFmpeg が PATH にある環境）
cargo test --workspace

# デスクトップアプリ起動（Windows 11 推奨）
cargo run -p ve-app
```
