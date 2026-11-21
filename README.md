# Next-Gen Hybrid Video Editor (`video-editing-app_AG`)

After Effects の表現力、AviUtl2 の軽快なタイムライン編集、そして AI ネイティブ操作基盤（内蔵 MCP）を単一環境で融合する Pure Rust 動画編集ソフトです。

詳細な仕様は [docs/spec.md](docs/spec.md)（v1.1）を参照してください。

## ライセンス
本リポジトリのソースコードは **MIT OR Apache-2.0** のデュアルライセンスです。
FFmpeg は外部 CLI（サイドカー）または LGPL 動的リンクとして本体から分離しています。

## ワークスペース構成（Phase 1a + 1b + 2 + 3: コア・NLE・AE融合・音声AI・立ち絵・3D）

| クレート | 役割 |
| :--- | :--- |
| `crates/core` (`ve-core`) | 有理数タイムベース（`RationalTime`）、プロジェクトモデル（`FilterObject` / `GroupControl` / `TrackMatte` / `TextAnimator` / `CharacterSprite` / `Camera3D` / `Transform3D`）、ベジェ曲線（値・速度）、`PrecompCacheManager`（ハッシュ＆LRU）、1フォルダ管理、`CommandBus`（Undo/Redo・Coalescing・Smart Precompose・クラッシュ復旧） |
| `crates/media` (`ve-media`) | FFmpeg / FFprobe 外部CLIラッパー（プローブ・デコード・MP4 H.264/H.265・WebM VP9+Alpha透過・PNG連番透過書き出し）、`symphonia` 音声デコード |
| `crates/audio` (`ve-audio`) | プレビュー／書き出し共通 `AudioGraph`（異サンプルレート変換・ステレオミキサー）、`AudioMasterClock`（A/V同期）、`WaveformPeaks`（波形＆RMS解析）、Windows WASAPI（`cpal`）出力 |
| `crates/render` (`ve-render`) | Linear sRGB（`Rgba16Float`）+ Premultiplied Alpha 合成、`cosmic-text`+`swash` 文字ごとアニメーター、Render Graph（Precompハッシュキャッシュ・トラックマット・グループ制御・WGSLエフェクト・PNG差分立ち絵自動口パク（音量RMS/母音音素）・3Dカメラ透視投影/Zソート/被写界深度ボケ） |
| `crates/ai` (`ve-ai`) | 自動文字起こし3モード（ローカルWhisper/VAD、Gemini API書き起こし、Claude API文脈補正）、OS資格情報ストア（Windows Credential Manager等・`project.json`非保存）、送信前コスト＆尺確認ガード、TTSファイルペア自動取込（`.wav`/`.txt`/`.lab`）＆ローカルAPI（VOICEVOX / COEIROINK / A.I.VOICE） |
| `crates/mcp` (`ve-mcp`) | 内蔵 MCP サーバー（ローカル Streamable HTTP `127.0.0.1` + 起動時ベアラートークン認証、`render_frame_preview`・`set_camera_3d`・`add_character_sprite`・`import_tts_file_pair`・`estimate_transcription_cost` 等22ツール） |
| `crates/app` (`ve-app`) | `eframe`（`wgpu`）+ `egui_dock` デスクトップアプリ、2空間UI切替、グラフエディタ、音声自動化・立ち絵口パク・3Dカメラパネル、透過書き出しUI、日本語IME PoC |

## ビルド・テスト

```bash
# 全クレートのコンパイル確認
cargo check --workspace

# ユニット・統合テスト実行（FFmpeg が PATH にある環境）
cargo test --workspace

# デスクトップアプリ起動（Windows 11 推奨）
cargo run -p ve-app
```
