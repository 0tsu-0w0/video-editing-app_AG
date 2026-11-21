# Next-Gen Hybrid Video Editor (`video-editing-app_AG`)

After Effects の**高度な表現力・アニメーション制御**、AviUtl2 の**軽快なタイムライン編集（NLE）**、そして **AI ネイティブな操作基盤（内蔵 MCP）** を単一環境で融合する Pure Rust（ゼロ `unsafe`）動画編集ソフトです。

* **仕様書 (v1.1):** [docs/spec.md](docs/spec.md)
* **アーキテクチャ設計書:** [docs/architecture.md](docs/architecture.md)
* **プラグイン＆カスタム WGSL 開発ガイド:** [docs/plugin-guide.md](docs/plugin-guide.md)
* **ロードマップ:** [docs/roadmap.md](docs/roadmap.md)
* **コントリビューションガイド:** [CONTRIBUTING.md](CONTRIBUTING.md)
* **変更履歴:** [CHANGELOG.md](CHANGELOG.md)

---

## 主な特徴（v0.7.1 現在）

* **Dual-Workspace UX（2空間分離） & プレビュー直接操作:** 軽快なカット・配置の「メイン編集空間 (AviUtl2)」と、グラフエディタ・トラックマット・エフェクトチェーンの「モーション合成空間 (AE)」をワンクリック切替。プレビュー画面上で移動・拡大縮小・回転ギズモ、中央/セーフマージン吸着ガイド、9:16 Shorts/TikTok UI 被り警告、**自由ベジェマスク頂点＆接線ハンドル直接ドラッグ**に対応。
* **AviUtl2 定番編集＋高速タイムライン:** `Space` / `J・K・L` シャトル再生、`←`/`→`（`Shift` で 10F）ステップ、`Home`/`End`、`S` 分割、`Ctrl+C`/`V`/`D`/`A`、`Del`/`Backspace`（`Shift` でリップル削除）、ラバーバンド複数選択、無音区間自動カット（ジェットカット）、ミニマップ、名前付きカラーパレット、**`presets/*.json` 字幕スタイル＆エフェクトチェーンプリセット管理**、チャプターマーカー編集＆YouTube チャプター出力。
* **Smart Precompose & ハッシュ LRU キャッシュ & RAM 先読み:** 複数レイヤーをワンクリック結合して新シーンへ移送。常駐 FFmpeg ストリームデコーダ＆専用ワーカースレッド **RAM プレビュー（96 フレーム先読み）**、軽量プロキシ、自動バックアップ（`backups/` ローテーション）を搭載。
* **人間と AI の完全対等な操作系（44 MCP ツール）:** GUI 操作も内蔵 MCP サーバー（ローカル Streamable HTTP + 起動時ベアラートークン認証・**44 ツール**）もすべて `CommandBus` を経由し、`Ctrl+Z` Undo/Redo と `history.jsonl` クラッシュ復旧を保証。
* **Linear sRGB (`Rgba16Float`) + GPU オフロード + WASM/WGSL プラグイン:** `wgpu` Ping-Pong GPU パイプライン、カスタム WGSL ホットリロード（`naga` 静的検証付き）、Pure-Rust・ゼロ `unsafe` **WebAssembly 1.0 サンドボックス VM（`plugins/*.wasm` ホットリロード）**、3D カメラ＆被写界深度、1 ポイント自動モーショントラッカーに対応。
* **音声自動化・台本量産・他ソフト互換・バッチ書き出し:** Whisper / Gemini / Claude 3 モード文字起こし＆文脈補正（OS 資格情報ストア＋課金確認ガード付き）、VOICEVOX / COEIROINK / `.wav+.txt+.lab` 自動取込、PNG 差分＆PSD ネイティブ立ち絵（自動口パク・目パチ）、音声 DSP 8 種＆LUFS 正規化、**BGM 自動ダッキング＆オーディオリアクティブ／スペクトラム**、CSV/Markdown 台本一括生成、**AI ハイライト抽出＆9:16 ショート自動切り抜き**、SRT/ASS/AviUtl `.exo`/OpenTimelineIO 入出力、マルチジョブレンダーキューに対応。

---

## ワークスペース構成

| クレート | 役割 |
| :--- | :--- |
| `crates/core` (`ve-core`) | 有理数タイムベース（`RationalTime`）、プロジェクトモデル、ベジェ曲線（値・速度）、`PrecompCacheManager`、1 フォルダ管理（`assets/`・`aliases/`・`presets/`・`backups/`）、`CommandBus`（Undo/Redo・Coalescing・Smart Precompose・クラッシュ復旧）、無音カット・台本生成・SRT/ASS/`.exo`/OTIO・ハイライト抽出 |
| `crates/media` (`ve-media`) | FFmpeg / FFprobe 外部 CLI ラッパー（プローブ・常駐 `VideoStreamDecoder`・MP4 H.264/H.265・WebM VP9+Alpha・PNG 連番・NVENC/QSV/AMF 自動選択）、`symphonia` 音声デコード |
| `crates/audio` (`ve-audio`) | プレビュー／書き出し共通 `AudioGraph`、DSP エフェクト 8 種、ITU-R BS.1770 LUFS 計測＆正規化、サイドチェイン BGM ダッキング、Goertzel 対数スペクトラム解析、`AudioMasterClock`（シャトル対応）、`WaveformPeaks`、Windows WASAPI（`cpal`）出力 |
| `crates/plugin-api` (`ve-plugin-api`) | OSS 公開プラグイン API（`PluginManifest` / `EffectPlugin` / `TimelineGeneratorPlugin` / `PluginRegistry`）＋ Pure-Rust ゼロ `unsafe` WebAssembly 1.0 サンドボックス VM＆`plugins/*.wasm` ホットリローダー |
| `crates/render` (`ve-render`) | Linear sRGB（`Rgba16Float`）+ Premultiplied Alpha 合成、`cosmic-text`+`swash` 装飾字幕＆インライン制御文字、自由ベジェマスク＆1 ポイントモーショントラッカー、PSD ネイティブ立ち絵、カスタム WGSL ホットリロード（`naga`検証）、3D カメラ・被写界深度、`wgpu` プレビュー／オフスクリーン GPU パイプライン |
| `crates/ai` (`ve-ai`) | 自動文字起こし 3 モード（ローカル Whisper/VAD・Gemini・Claude）、Claude 字幕文脈補正、OS 資格情報ストア（`project.json` 非保存）、課金確認ガード、TTS ファイルペア取込＆ローカル API（VOICEVOX / COEIROINK / A.I.VOICE） |
| `crates/mcp` (`ve-mcp`) | 内蔵 MCP サーバー（ローカル Streamable HTTP `127.0.0.1` + 起動時ベアラートークン認証、視覚プレビュー・編集・プリセット・マスク・WASM・ハイライト抽出 等 44 ツール） |
| `crates/app` (`ve-app`) | `eframe`（`wgpu`）+ `egui_dock` デスクトップアプリ（2 空間 UI・プレビューギズモ＆ベジェマスク直接編集・RAM プレビュー・AviUtl2 タイムライン・グラフエディタ・各種インスペクタ・レンダーキュー・フォルダ監視） |

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

> **配布バイナリ:** タグ（`v*`）プッシュ時は GitHub Actions（`.github/workflows/release.yml`）により Windows 11 向け zip パッケージが GitHub Releases へ自動生成されます。

## ライセンス

本リポジトリのソースコードは **MIT OR Apache-2.0** のデュアルライセンスで公開しています（[LICENSE-MIT](LICENSE-MIT) / [LICENSE-APACHE](LICENSE-APACHE)）。
GPL 汚染を避けるため、FFmpeg は外部 CLI サイドカーとして本体から分離しています。
