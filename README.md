# Next-Gen Hybrid Video Editor (`video-editing-app_AG`)

After Effects の**高度な表現力・アニメーション制御**、AviUtl2 の**軽快なタイムライン編集（NLE）**、そして **AI ネイティブな操作基盤（内蔵 MCP）** を単一環境で融合する Pure Rust 動画編集ソフトです。

* **仕様書 (v1.1):** [docs/spec.md](docs/spec.md)
* **アーキテクチャ設計書:** [docs/architecture.md](docs/architecture.md)
* **プラグイン＆カスタム WGSL 開発ガイド:** [docs/plugin-guide.md](docs/plugin-guide.md)
* **コントリビューションガイド:** [CONTRIBUTING.md](CONTRIBUTING.md)
* **変更履歴:** [CHANGELOG.md](CHANGELOG.md)

---

## 主な特徴（v0.1.0: Phase 1a 〜 Phase 4 実装完了）

* **Dual-Workspace UX（2空間分離）:** 軽快なカット・配置の「メイン編集空間 (AviUtl2)」と、グラフエディタ・トラックマット・エフェクトチェーンの「モーション合成空間 (AE)」をワンクリック切替。
* **Smart Precompose & ハッシュ LRU キャッシュ:** 複数レイヤーをワンクリック結合して新シーンへ移送。コンテンツハッシュで自動無効化＆LRU 容量管理し、ダブルクリックでいつでも非破壊再編集。
* **人間と AI の完全対等な操作系:** GUI 操作も内蔵 MCP サーバー（ローカル Streamable HTTP + 起動時ベアラートークン認証・24ツール）もすべて `CommandBus` を経由し、`Ctrl+Z` Undo/Redo と `history.jsonl` クラッシュ復旧を保証。
* **Linear sRGB (`Rgba16Float`) + Premultiplied Alpha 合成:** `wgpu` + WGSL パイプライン、3D カメラ制御＆被写界深度、カスタム WGSL ホットリロード（`naga` 静的検証付き）、`ve-plugin-api` プラグイン拡張に対応。
* **音声自動化・TTS・立ち絵自動口パク（PNG 差分 & PSD ネイティブ）:** Whisper / Gemini / Claude 3モード文字起こし（OS 資格情報ストア保管＋課金確認ガード付き）、VOICEVOX / COEIROINK / `.wav+.txt+.lab` ファイルペア自動取込、音量 RMS／母音音素連動＆PSD 自動目パチに対応。

---

## ワークスペース構成

| クレート | 役割 |
| :--- | :--- |
| `crates/core` (`ve-core`) | 有理数タイムベース（`RationalTime`）、プロジェクトモデル、ベジェ曲線（値・速度）、`PrecompCacheManager`（ハッシュ＆LRU）、1フォルダ管理、`CommandBus`（Undo/Redo・Coalescing・Smart Precompose・クラッシュ復旧） |
| `crates/media` (`ve-media`) | FFmpeg / FFprobe 外部CLIラッパー（プローブ・デコード・MP4 H.264/H.265・WebM VP9+Alpha透過・PNG連番透過書き出し）、`symphonia` 音声デコード |
| `crates/audio` (`ve-audio`) | プレビュー／書き出し共通 `AudioGraph`（異サンプルレート変換・ステレオミキサー）、`AudioMasterClock`（A/V同期）、`WaveformPeaks`（波形＆RMS解析）、Windows WASAPI（`cpal`）出力 |
| `crates/plugin-api` (`ve-plugin-api`) | OSS 公開プラグイン API（`PluginManifest` / `EffectPlugin` / `TimelineGeneratorPlugin` / `PluginRegistry` + リファレンス実装ビネット＆セピア） |
| `crates/render` (`ve-render`) | Linear sRGB（`Rgba16Float`）+ Premultiplied Alpha 合成、`cosmic-text`+`swash` 文字ごとアニメーター、PSD ネイティブ立ち絵（目パチ・口パク）、カスタム WGSL ホットリロード（`naga`検証）、3D カメラ・被写界深度、`wgpu` プレビューパイプライン |
| `crates/ai` (`ve-ai`) | 自動文字起こし3モード（ローカル Whisper/VAD・Gemini・Claude）、OS 資格情報ストア（Windows Credential Manager 等・`project.json`非保存）、課金確認ガード、TTS ファイルペア取込＆ローカル API（VOICEVOX / COEIROINK / A.I.VOICE） |
| `crates/mcp` (`ve-mcp`) | 内蔵 MCP サーバー（ローカル Streamable HTTP `127.0.0.1` + 起動時ベアラートークン認証、視覚プレビュー・編集・プラグイン一覧・WGSL検証 等24ツール） |
| `crates/app` (`ve-app`) | `eframe`（`wgpu`）+ `egui_dock` デスクトップアプリ（2空間UI・AviUtl2タイムライン・グラフエディタ・音声自動化/立ち絵/3Dパネル・WGSLホットリロード・日本語IME PoC） |

---

## ビルド・テスト・実行

```bash
# フォーマット・静的解析・全テスト（CI と同一）
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# デスクトップアプリ起動（Windows 11 推奨、FFmpeg が PATH に必要）
cargo run -p ve-app
```

## ライセンス

本リポジトリのソースコードは **MIT OR Apache-2.0** のデュアルライセンスで公開しています（[LICENSE-MIT](LICENSE-MIT) / [LICENSE-APACHE](LICENSE-APACHE)）。
GPL 汚染を避けるため、FFmpeg は外部 CLI サイドカーとして本体から分離しています。
