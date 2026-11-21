# Next-Gen Hybrid Video Editor (`video-editing-app_AG`)

After Effects の表現力、AviUtl2 の軽快なタイムライン編集、そして AI ネイティブ操作基盤（内蔵 MCP）を単一環境で融合する Pure Rust 動画編集ソフトです。

詳細な仕様は [docs/spec.md](docs/spec.md)（v1.1）を参照してください。

## ライセンス
本リポジトリのソースコードは **MIT OR Apache-2.0** のデュアルライセンスです。
FFmpeg は外部 CLI（サイドカー）または LGPL 動的リンクとして本体から分離しています。

## ワークスペース構成（Phase 1a: コアエンジン PoC）

| クレート | 役割 |
| :--- | :--- |
| `crates/core` (`ve-core`) | 有理数タイムベース（`RationalTime`, 705,600,000 ticks/sec）、プロジェクトモデル（`Project` / `Scene` / `Layer` / `TimelineItem` / AviUtl2風 `FilterObject`）、1フォルダ管理（`project.json`・`history.jsonl`・アセット参照＆収集） |
| `crates/media` (`ve-media`) | FFmpeg / FFprobe 外部CLIラッパー（プローブ・デコード・MP4/WebM透過/PNG連番書き出し）、`symphonia` 音声デコード |
| `crates/audio` (`ve-audio`) | プレビュー／書き出し共通 `AudioGraph`（異サンプルレート変換・ステレオミキサー）、`AudioMasterClock`（A/V同期）、Windows WASAPI（`cpal`）出力 |
| `crates/render` (`ve-render`) | Linear sRGB（`Rgba16Float`）+ Premultiplied Alpha 合成、`cosmic-text`+`swash` 日本語テキスト描画、Render Graph（個別エフェクトチェーン → レイヤー合成 → AviUtl2フィルタオブジェクト）、`wgpu` + WGSL プレビューパイプライン |
| `crates/app` (`ve-app`) | `eframe`（`wgpu`）+ `egui_dock` デスクトップアプリ、Phase 1a 日本語IME PoC 検証パネル |

## ビルド・テスト

```bash
# 全クレートのコンパイル確認
cargo check --workspace

# ユニット・統合テスト実行（FFmpeg が PATH にある環境）
cargo test --workspace

# デスクトップアプリ起動（Windows 11 推奨）
cargo run -p ve-app
```
