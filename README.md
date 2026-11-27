# Next-Gen Hybrid Video Editor (`video-editing-app_AG`)

After Effects の**高度な表現力・アニメーション制御**、AviUtl2 の**軽快なタイムライン編集（NLE）**、そして **AI ネイティブな操作基盤（内蔵 MCP）** を単一環境で融合する Pure Rust（ゼロ `unsafe`）動画編集ソフトです。

* **仕様書 (v1.1):** [docs/spec.md](docs/spec.md)
* **アーキテクチャ設計書:** [docs/architecture.md](docs/architecture.md)
* **プラグイン＆カスタム WGSL 開発ガイド:** [docs/plugin-guide.md](docs/plugin-guide.md)
* **ロードマップ:** [docs/future_features.md](docs/future_features.md)
* **コントリビューションガイド:** [CONTRIBUTING.md](CONTRIBUTING.md)
* **変更履歴:** [CHANGELOG.md](CHANGELOG.md)

---

## 主な特徴（v11.0.0 Zero-Copy GPU Engine, Real-Time 8K & Native Studio Performance — 全 64 ロードマップ項目達成・全 104 MCP ツール）

* **完全ゼロコピー GPU レンダリンググラフ・適応解像度 8K 再生・FastBin SHA-256 超軽量化・リアルタイムプロファイラ HUD（v11.0 新搭載）:** VRAM 常駐 Ping-Pong バッファ上で合成→グレーディング→360° 逆射影を 1 コマンドバッファ・中間 CPU リードバック 0 回で連結実行する **ゼロコピー GPU グラフ（全 9 本組み込み WGSL シェーダー Naga 一括検証・決定論 CPU フォールバック付）**、EMA ヒステリシスによる **適応解像度プロキシ自動切替（1/1→1/2→1/4・停止時フル解像度即復帰）＆複雑度ホットスポット優先スマート事前レンダリング**、FIPS 180-4 SHA-256 整合性検証付き **FastBin 高速スナップショット（`cache/project.fastbin`）・Undo `Arc` 50% 省メモリ共有監査・1-Undo コンパクション**、および **ダークガラス調プロファイラ HUD（p95/p99 統計・パス別マイクロ秒内訳フレームグラフ・自動ボトルネック診断＆1-Undo 安全最適化）** をすべて Pure Rust・ゼロ `unsafe`・追加外部依存ゼロで統合。
* **360° 全天球＆空間 3D・2D Navier-Stokes 流体/PBD クロス物理・3D シャドウ/HDRI IBL・BVH/シルエットモキャップ・LAN 分散レンダー＆自律 AI シネマプロデューサー（v10.0）:** Equirectangular ⇔ 透視投影 **OverCapture** / 逆ステレオ投影 **Tiny Planet** / ジャイロ指数平滑化 **水平ロック（Horizon Lock）** ＆ **Side-by-Side / Top-and-Bottom / Red-Cyan アナグリフ空間 3D**、Jos Stam 安定法 **決定論 2D Navier-Stokes 流体（煙・炎・墨・プラズマ）** ＆ **PBD クロス物理** ＋ **2 パス直交デプスシャドウマップ（PCF ソフトシャドウ＆床影受け）＆ Equirectangular HDRI 拡散/鏡面 IBL**、Pure-Rust **`.bvh` 3D FK パーサ＆動画シルエット 7 関節姿勢推定 → 体格比スケーリング＆足滑り防止 Foot Skate Locking リターゲット**、そして **RFC 2104 HMAC-SHA256 認証付き LAN 分散レンダーファーム** ＆ **自律 AI シネマプロデューサー（台本→絵コンテアニマティック・秒別視聴維持率予測・A/B 自動編集バリアント）**。
* **ライブ配信スイッチャー ＆ VTuber 2D 三角メッシュ/VMC ＆ ハードウェアブリッジ ＆ ローカル スマート DAM:** PGM/PVW 2 バスライブスイッチャー（6 種スティンガー・放送タリー HUD・RTMP/SRT＋MKV ISO tee・`.veproj` 同時生成）、VTuber 2D 三角メッシュ変形＆Verlet 物理揺れ＆UDP OSC/VMC 受信、OSC / MIDI-over-UDP / Stream Deck (Pure-Rust SHA-1 + RFC 6455 WebSocket) 統合ブリッジ、5 色パレット・16D 類似検索・SHA-256 指紋一括自動再リンクのローカル スマート DAM。
* **Pro Cinema VFX ＆ モーションファウンドリ ＆ オーディオ修復 / 空間音声バス:** 4 点平面ホモグラフィトラッカー＆Sobel エッジ吸着キーフレームロトブラシ、SVG `<path>` モーフィング・Text-on-Path・4×4 双三次グリッドワープ、擬似深度ボケ（Rack Focus）・ゴッドレイ・Brown-Conrady レンズ歪み補正＆物理色収差、50/60Hz 縦続ノッチ・MAD+Hermite デクリック・適応 De-Reverb・3-2-1 ADR カウント収録・ITU-R BS.775 5.1ch / FOA Ambisonics (AmbiX) 空間音声バス。
* **Tri-Workspace UX（3 空間）× 6 スタジオ Hub 統合 ＆ `Ctrl+K` ファジー検索コマンドパレット:** 「メイン編集空間 (AviUtl2)」「モーション合成空間 (AE)」「ノード合成空間 (Fusion/Nuke)」の **3 空間を維持** しつつ、v0.9〜v11.0 全 64 ロードマップ機能を制作ロール別の **6 スタジオ Hub（`Edit` / `VFX & Motion` / `Color` / `Audio` / `AI & Collab` / `Deliver`）** へ機能ゼロ欠落で完全統合。**`Ctrl+K` ファジー検索コマンドパレット**（日英・`#1`〜`#64`・MRU 優先）から全ツールへ 1 秒で到達できます。
* **内蔵 MCP ドメイン別モジュール分割（全 104 MCP ツール / 11 ドメイン・完全オフライン AI 連携）:** 内蔵 MCP サーバー（ローカル Streamable HTTP + ベアラートークン認証）をドメイン別 **11 サブモジュール群**（`edit_nle` / `vfx_motion` / `color_finishing` / `audio_voice` / `ai_collab` / `deliver_system` / `v70_unified` / `v80_cinema_audio` / `v90_broadcast_ecosystem` / `v100_spatial_cinema_os` / `v110_performance_gpu`）へ体系化し、v11.0 新規 4 ツールを含む **全 104 MCP ツール** を提供。GUI・ハードウェア・AI の全操作が `CommandBus` を経由し、`Ctrl+Z` Undo/Redo・`history.jsonl` クラッシュ復旧・編集レシピ・承認ゲート・視覚自己検査を保証。
* **GPU-Resident 4K/8K プール＆10,000+ 区間木＆TC 自動コンフォーム＆A/B 比較＆プロ制作スイート完備:** 4K/8K フレームプール＆WGSL 高速パス、10,000+ クリップ区間木 $O(\log N)$ 検索、SMPTE TC プロキシ⇔原本自動コンフォーム、A/B 比較ワイプ、データ駆動チャート＆VFX ペイント/ワイヤー消し＆ポッドキャスト自動編集＆マルチプラットフォーム納品、2D ボーン＆2-Bone IK・自動 BGM 作曲・スピードランプ・35mm フィニッシング＆EDL/FCPXML、ACEScg/OCIO・2D 物理・生成背景/Foley・放送 QC＆HTML5 レビュー、glTF 2.0 3D PBR・オフライン AI 吹替・P2P CRDT 同時編集・ローカル AI ビジョン・RGB カーブ/HSL・3D 空間音響・マルチカム・WASM サンドボックス＆SHA-256 拡張レジストリまで完全統合。

---

## ワークスペース構成

| クレート | 役割 |
| :--- | :--- |
| `crates/core` (`ve-core`) | 有理数タイムベース、**v11.0 ゼロコピー GPU DAG プランナー・適応解像度 EMA＆スマート事前レンダリング・FastBin SHA-256 スナップショット＆メモリ監査・プロファイラ統計＆ボトルネック診断（`perf_engine`）**、v10.0 360° 全天球＆空間 3D・2D Navier-Stokes 流体＆PBD クロス＆HDRI IBL・`.bvh`＆シルエットモキャップ・RFC 2104 HMAC-SHA256 LAN 分散レンダーファーム＆自律 AI シネマプロデューサー、v9.0 ライブスイッチャー/VTuber メッシュ＆VMC/OSC・MIDI・Stream Deck WS/スマート DAM、v8.0 平面追跡/ロト/光学 VFX/ADR/空間バス＆SVG ワープ、6 スタジオ Hub＆`Ctrl+K`・区間木 $O(\log N)$・TC コンフォーム・A/B 比較、データチャート・VFX ペイント・マルチ納品・2D IK・スピードランプ・35mm 仕上げ＆EDL/FCPXML・ACEScg/OCIO・2D 物理・生成背景・放送 QC/HTML5・glTF 2.0 3D・CRDT＆LAN 同時編集・ノード合成グラフ・AI 編集レシピ・Diff/3-Way マージ・エクスプレッション VM・テンプレート・マルチカム・SSD 永続キャッシュ・納品検証＆ZIP、`CommandBus`（`Arc<Project>` 50% 省メモリ Undo/Redo・クラッシュ復旧） |
| `crates/media` (`ve-media`) | FFmpeg / FFprobe 外部 CLI ラッパー（RTMP/RTMPS/SRT＋ローカル MKV ISO tee 配信引数ビルダー・SMPTE 開始タイムコード＆リール名抽出・HW デコード自動検出＆フォールバック・常駐 `VideoStreamDecoder`・可変プロキシ・Rec.709 / HDR10 PQ 10-bit HEVC・NVENC/QSV/AMF 自動選択）、`symphonia` 音声デコード |
| `crates/audio` (`ve-audio`) | プレビュー／書き出し共通 `AudioGraph`、オーディオ修復（縦続ノッチ IIR 50/60Hz ハム除去・MAD+Hermite デクリック・適応 De-Reverb）・ADR 3-2-1 同期ビープ・ITU-R BS.775 5.1ch サラウンド＆FOA Ambisonics (AmbiX `W,Y,Z,X` ⇔ バイノーラル復号) マルチチャンネル WAV 書き出し、ポッドキャスト話者ダイアリゼーション＆-16 LUFS レベラー、オフライン自動 BGM コンポーザー、Foley/SFX、オフライン多言語 AI 吹替＆音素リップシンク、DSP 16 種、ボーカル/BGM 分離、マルチカム相互相関同期、マイク直接録音、LUFS 正規化、ダッキング、BPM/ビート検出、`WaveformPeaks`、WASAPI 出力 |
| `crates/plugin-api` (`ve-plugin-api`) | OSS 公開プラグイン API ＋ Pure-Rust ゼロ `unsafe` WebAssembly 1.0 サンドボックス VM＆`plugins/*.wasm` ホットリローダー |
| `crates/render` (`ve-render`) | Linear sRGB（`Rgba16Float`）+ Premultiplied Alpha 合成、**v11.0 ゼロコピー GPU レンダリンググラフ（`ZERO_COPY_COMPOSITE_GRADE_WGSL`・全 9 本組み込み WGSL Naga 検証）・パス別マイクロ秒計測 `render_project_scene_profiled`＆ダークガラスプロファイラ HUD（`gpu_v110`）**、v10.1 テキスト BBox LRU・`Arc` 3D LUT・SIMD フレンドリー合成最適化、v10.0 360° OverCapture/Tiny Planet/水平ロック＆空間 3D パス・2D 流体＆PBD クロスラスタライザ・3D PCF ソフトシャドウマップ＆HDRI IBL、v9.0 スティンガー＆VTuber 2D 三角メッシュラスタライザ、v8.0 平面追跡/ロト/SVG ワープ/光学 VFX/ADR、GPU 4K/8K プール＆WGSL 高速パス・区間木探索・A/B 比較、データチャート＆VFX ペイント、2D パペットリグ・カット検出・35mm フィニッシング、ACEScg/OCIO・プロシージャル背景・放送 QC、フル GPU WGSL パイプライン、PBR 3D メッシュ/glTF ラスタライザ＆3D カメラトラッカー、RGB カーブ/HSL/カラーマッチ、手ぶれ補正＆4 点コーナーピン、キネティック字幕、ノード合成、ローカル AI ビジョン、並列バッチ描画、パーティクル＆ベクター＆3D ライト/モーションブラー、`.cube` 3D LUT＆スコープ、ベジェマスク＆トラッカー、PSD 立ち絵、WGSL ホットリロード、ゴールデン視覚回帰テスト |
| `crates/ai` (`ve-ai`) | 自動文字起こし 3 モード（ローカル Whisper/VAD・Gemini・Claude）、Claude 字幕文脈補正、OS 資格情報ストア、課金確認ガード、TTS ファイルペア取込＆ローカル API |
| `crates/mcp` (`ve-mcp`) | 内蔵 MCP サーバー（ローカル Streamable HTTP `127.0.0.1` + 起動時ベアラートークン認証、**ドメイン別 11 サブモジュール分割構成 `crates/mcp/src/tools/`**、v11.0 新規 4 ツール含む **全 104 ツール**） |
| `crates/app` (`ve-app`) | `eframe`（`wgpu`）+ `egui_dock` デスクトップアプリ（3 空間 UI・6 スタジオ Hub 統合ウィンドウ＆`Ctrl+K` コマンドパレット・**v11.0 ライブ適応解像度スケーリング・プロファイラ HUD＆p95/p99 グラフ・ゼロコピー GPU グラフパネル・FastBin メモリ監査＆1-Undo コンパクション**・v10.0 360°/流体/モキャップ/分散レンダー/自律 AI・v9.0 ライブスイッチャー/VTuber/HW/スマート DAM・v8.0 光学 VFX/平面追跡/修復 ADR・A/B 比較・10,000+ 区間木・スコープ・f16 RAM＋SSD キャッシュ・並列書き出し・レンダーキュー） |

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
