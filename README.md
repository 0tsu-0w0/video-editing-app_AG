# Next-Gen Hybrid Video Editor (`video-editing-app_AG`)

After Effects の**高度な表現力・アニメーション制御**、AviUtl2 の**軽快なタイムライン編集（NLE）**、そして **AI ネイティブな操作基盤（内蔵 MCP）** を単一環境で融合する Pure Rust（ゼロ `unsafe`）動画編集ソフトです。

* **仕様書 (v1.1):** [docs/spec.md](docs/spec.md)
* **アーキテクチャ設計書:** [docs/architecture.md](docs/architecture.md)
* **プラグイン＆カスタム WGSL 開発ガイド:** [docs/plugin-guide.md](docs/plugin-guide.md)
* **ロードマップ:** [docs/future_features.md](docs/future_features.md)
* **コントリビューションガイド:** [CONTRIBUTING.md](CONTRIBUTING.md)
* **変更履歴:** [CHANGELOG.md](CHANGELOG.md)

---

## 主な特徴（v9.0.0 Live Broadcast, VTuber & Hardware Ecosystem — 全 56 ロードマップ項目達成）

* **ライブ配信スイッチャー ＆ VTuber 2D 三角メッシュ/VMC ＆ ハードウェアブリッジ ＆ ローカル スマート DAM（v9.0 新搭載）:** PGM/PVW 2 バスライブスイッチャー（6 種アルファ付きスティンガー遷移・放送タリー HUD・RTMP/RTMPS/SRT＋ローカル MKV ISO tee FFmpeg サイドカー生成・スイッチング履歴の `.veproj` NLE 同時生成）、**VTuber 2D 三角メッシュ変形（ブレンドシェイプ＆擬似 3D 頭部回転）・距離拘束 Verlet 物理揺れ・UDP OSC / VMC 顔トラッキングリアルタイム受信＆キーフレームベイク**、**OSC (UDP) / MIDI-over-UDP / Stream Deck (Pure-Rust SHA-1 + RFC 6455 WebSocket)** 3 プロトコル統合ハードウェアブリッジ、そして 5 色ドミナントパレット・16 次元コサイン類似ショット検索・条件式スマートビン・**SHA-256 先頭 64KiB 指紋によるリネーム耐性付き一括自動再リンク** のローカル スマート DAM をすべて Pure Rust・ゼロ `unsafe`・追加外部依存ゼロで完備。
* **Pro Cinema VFX ＆ モーションファウンドリ ＆ オーディオ修復 / 空間音声バス:** 4×4 グリッド ZNCC＋RANSAC による **4 点平面ホモグラフィトラッカー（CornerPin 1-Undo ベイク）** と **Sobel エッジ吸着＋オプティカルフロー伝播キーフレームロトブラシ**、SVG `<path>` 全コマンドパーサ＆等弧長パスモーフィング・接線回転追従 **Text-on-Path**・**4×4 双三次グリッドワープ＆輝度変位**、擬似深度ボケ（**Rack Focus**）・体積光 **ゴッドレイ**・**Brown-Conrady レンズ歪み補正 ($k_1, k_2, p_1, p_2$)＆物理色収差**、50/60Hz 縦続ノッチハム除去・MAD+Hermite デクリック・適応 De-Reverb・3-2-1 ADR カウント収録・ITU-R BS.775 5.1ch / FOA Ambisonics (AmbiX `W,Y,Z,X` ⇔ バイノーラル) 空間音声バス。
* **Tri-Workspace UX（3 空間）× 6 スタジオ Hub 統合 ＆ `Ctrl+K` ファジー検索コマンドパレット:** 軽快な「メイン編集空間 (AviUtl2)」「モーション合成空間 (AE)」「ノード合成空間 (Fusion/Nuke)」の **3 空間を維持** しつつ、v0.9〜v9.0 の全専門ツールを制作ロール別の **6 スタジオ Hub（`Edit` / `VFX & Motion` / `Color` / `Audio` / `AI & Collab` / `Deliver`）** へ機能ゼロ欠落で完全統合。**`Ctrl+K` ファジー検索コマンドパレット**（日英・`#1`〜`#56`・MRU 優先）から全ツールへ 1 秒で到達できます。
* **内蔵 MCP ドメイン別モジュール分割（全 90 MCP ツール / 9 ドメイン・完全オフライン AI 連携）:** 内蔵 MCP サーバー（ローカル Streamable HTTP + ベアラートークン認証）をドメイン別 9 サブモジュール群（`edit_nle` / `vfx_motion` / `color_finishing` / `audio_voice` / `ai_collab` / `deliver_system` / `v70_unified` / `v80_cinema_audio` / `v90_broadcast_ecosystem`）へ整理し、v9.0 新規 4 ツールを含む全 **90 MCP ツール** を提供。GUI・ハードウェア・AI の全操作が `CommandBus` を経由し、`Ctrl+Z` Undo/Redo・`history.jsonl` クラッシュ復旧・編集レシピ・承認ゲート・視覚自己検査を保証。
* **GPU-Resident 4K/8K プール＆10,000+ 区間木＆TC 自動コンフォーム＆A/B 比較＆プロ制作スイート完備:** 4K/8K フレームプール＆WGSL 高速パス、10,000+ クリップ区間木 $O(\log N)$ 検索、SMPTE TC プロキシ⇔原本自動コンフォーム、A/B 比較ワイプ、データ駆動チャート＆VFX ペイント/ワイヤー消し＆ポッドキャスト自動編集＆マルチプラットフォーム納品、2D ボーン＆2-Bone IK・自動 BGM 作曲・スピードランプ・35mm フィニッシング＆EDL/FCPXML、ACEScg/OCIO・2D 物理・生成背景/Foley・放送 QC＆HTML5 レビュー、glTF 2.0 3D PBR・オフライン AI 吹替・P2P CRDT 同時編集・ローカル AI ビジョン・RGB カーブ/HSL・3D 空間音響・マルチカム・WASM サンドボックス＆SHA-256 拡張レジストリまで完全統合。

---

## ワークスペース構成

| クレート | 役割 |
| :--- | :--- |
| `crates/core` (`ve-core`) | 有理数タイムベース、**v9.0 ライブスイッチャー＆`.veproj` 同時生成（`live_switcher`）・VTuber 2D 三角メッシュ＆Verlet 物理＆OSC/VMC 受信機（`vtuber_rig`）・OSC/MIDI-over-UDP/RFC 6455 Stream Deck WebSocket ブリッジ（`hardware_ctrl`）・ローカル スマート DAM 索引/類似検索/SHA-256 再リンク（`smart_dam`）**、v8.0 平面追跡/ロト/光学 VFX/ADR/空間バス＆SVG パーサ/ワープ、6 スタジオ Hub＆`Ctrl+K` カタログ・区間木 $O(\log N)$・TC コンフォーム・A/B 比較、データチャート・VFX ペイント・マルチ納品・2D IK・スピードランプ・35mm 仕上げ＆EDL/FCPXML・ACEScg/OCIO・2D 物理・生成背景・放送 QC/HTML5・glTF 2.0 3D・CRDT＆LAN 同時編集・ノード合成グラフ・AI 編集レシピ・Diff/3-Way マージ・エクスプレッション VM・テンプレート・マルチカム・SSD 永続キャッシュ・納品検証＆ZIP、`CommandBus`（Undo/Redo・クラッシュ復旧） |
| `crates/media` (`ve-media`) | FFmpeg / FFprobe 外部 CLI ラッパー（**v9.0 RTMP/RTMPS/SRT＋ローカル MKV ISO tee 配信引数ビルダー `build_live_stream_ffmpeg_args`**・SMPTE 開始タイムコード＆リール名抽出・HW デコード自動検出＆フォールバック・常駐 `VideoStreamDecoder`・可変プロキシ・Rec.709 / HDR10 PQ 10-bit HEVC・NVENC/QSV/AMF 自動選択）、`symphonia` 音声デコード |
| `crates/audio` (`ve-audio`) | プレビュー／書き出し共通 `AudioGraph`、オーディオ修復（縦続ノッチ IIR 50/60Hz ハム除去・MAD+Hermite デクリック・適応 De-Reverb）・ADR 3-2-1 同期ビープ・ITU-R BS.775 5.1ch サラウンド＆FOA Ambisonics (AmbiX `W,Y,Z,X` ⇔ バイノーラル復号) マルチチャンネル WAV 書き出し、ポッドキャスト話者ダイアリゼーション＆-16 LUFS レベラー、オフライン自動 BGM コンポーザー、Foley/SFX、オフライン多言語 AI 吹替＆音素リップシンク、DSP 16 種、ボーカル/BGM 分離、マルチカム相互相関同期、マイク直接録音、LUFS 正規化、ダッキング、BPM/ビート検出、`WaveformPeaks`、WASAPI 出力 |
| `crates/plugin-api` (`ve-plugin-api`) | OSS 公開プラグイン API ＋ Pure-Rust ゼロ `unsafe` WebAssembly 1.0 サンドボックス VM＆`plugins/*.wasm` ホットリローダー |
| `crates/render` (`ve-render`) | Linear sRGB（`Rgba16Float`）+ Premultiplied Alpha 合成、**v9.0 ライブスイッチャー 6 種スティンガー遷移コンポジタ＆放送タリー HUD・VTuber 2D 三角メッシュアフィンテクスチャラスタライザ（`v90_render`）**、v8.0 平面ホモグラフィ追跡/ロト・SVG モーフィング/Text-on-Path/グリッドワープ・シネマ光学 VFX・ADR ストリーマ、GPU 4K/8K プール＆WGSL 高速パス・区間木探索・A/B 比較、データチャート＆VFX ペイント、2D パペットリグ・カット検出・35mm フィニッシング、ACEScg/OCIO・プロシージャル背景・放送 QC、フル GPU WGSL パイプライン、PBR 3D メッシュ/glTF ラスタライザ＆3D カメラトラッカー、RGB カーブ/HSL/カラーマッチ、手ぶれ補正＆4 点コーナーピン、キネティック字幕、ノード合成、ローカル AI ビジョン、並列バッチ描画、パーティクル＆ベクター＆3D ライト/モーションブラー、`.cube` 3D LUT＆スコープ、ベジェマスク＆トラッカー、PSD 立ち絵、WGSL ホットリロード、ゴールデン視覚回帰テスト |
| `crates/ai` (`ve-ai`) | 自動文字起こし 3 モード（ローカル Whisper/VAD・Gemini・Claude）、Claude 字幕文脈補正、OS 資格情報ストア、課金確認ガード、TTS ファイルペア取込＆ローカル API |
| `crates/mcp` (`ve-mcp`) | 内蔵 MCP サーバー（ローカル Streamable HTTP `127.0.0.1` + 起動時ベアラートークン認証、**ドメイン別 9 サブモジュール分割構成 `crates/mcp/src/tools/`**、v9.0 新規 4 ツール［ライブスイッチャー・VTuber メッシュ/VMC・OSC/MIDI/StreamDeck・スマート DAM］含む全 **90 ツール**） |
| `crates/app` (`ve-app`) | `eframe`（`wgpu`）+ `egui_dock` デスクトップアプリ（3 空間 UI・6 スタジオ Hub 統合ウィンドウ＆`Ctrl+K` コマンドパレット・**v9.0 Deliver Hub［ライブスイッチャー PGM/PVW タリーバンク］・VFX Hub［VTuber 2D メッシュ＆VMC UDP 受信］・AI Hub［3 プロトコルハードウェアブリッジ］・Edit Hub［スマート DAM 5 色スウォッチ＆類似検索＆一括再リンク］**・v8.0 光学 VFX/平面追跡/修復 ADR・A/B 比較・10,000+ 区間木・スコープ・RAM＋SSD キャッシュ・並列書き出し・レンダーキュー） |

---

## ビルド・テスト・実行

```bash
# フォーマット・静的解析・全テスト（CI と同一）
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# デスクトップアプリ起動（Windows 11 / macOS / Linux、FFmpeg が PATH に必要）
cargo run -p ve-app
```

> **マルチ OS 配布バイナリ:** タグ（`v*`）プッシュ時は GitHub Actions（`.github/workflows/release.yml`）により Windows 11 x64（ZIP + `winget` マニフェスト）、macOS Universal（`.app` Bundle）、Linux x64（`.tar.gz` + `AppDir`）が SHA-256 チェックサム付きで GitHub Releases へ自動生成されます。

## ライセンス

本リポジトリのソースコードは **MIT OR Apache-2.0** のデュアルライセンスで公開しています（[LICENSE-MIT](LICENSE-MIT) / [LICENSE-APACHE](LICENSE-APACHE)）。
GPL 汚染を避けるため、FFmpeg は外部 CLI サイドカーとして本体から分離しています。
