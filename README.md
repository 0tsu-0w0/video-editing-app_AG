# Next-Gen Hybrid Video Editor (`video-editing-app_AG`)

After Effects の**高度な表現力・アニメーション制御**、AviUtl2 の**軽快なタイムライン編集（NLE）**、そして **AI ネイティブな操作基盤（内蔵 MCP）** を単一環境で融合する Pure Rust（ゼロ `unsafe`）動画編集ソフトです。

* **仕様書 (v1.1):** [docs/spec.md](docs/spec.md)
* **アーキテクチャ設計書:** [docs/architecture.md](docs/architecture.md)
* **プラグイン＆カスタム WGSL 開発ガイド:** [docs/plugin-guide.md](docs/plugin-guide.md)
* **ロードマップ:** [docs/future_features.md](docs/future_features.md)
* **コントリビューションガイド:** [CONTRIBUTING.md](CONTRIBUTING.md)
* **変更履歴:** [CHANGELOG.md](CHANGELOG.md)

---

## 主な特徴（v8.0.0 Pro Cinema VFX, Motion Foundry & Audio Restoration — 全 52 ロードマップ項目達成）

* **Pro Cinema VFX ＆ モーションファウンドリ ＆ オーディオ修復 / 空間音声バス（v8.0 新搭載）:** 4×4 グリッド ZNCC＋RANSAC による **4 点平面ホモグラフィトラッカー（CornerPin キーフレーム 1-Undo 自動ベイク）** と **Sobel エッジ吸着＋オプティカルフロー伝播キーフレームロトブラシ**、SVG `<path>`（`M/L/H/V/C/S/Q/T/A/Z` 全コマンド対応）インポート＆等弧長パスモーフィング・接線回転追従 **Text-on-Path**・**4×4 双三次グリッドワープ＆輝度ディスプレイスメント**、擬似深度ボケ（**Rack Focus 多角形玉ボケ**）・体積光 **ゴッドレイ**・**Brown-Conrady レンズ歪み補正 ($k_1, k_2, p_1, p_2$)＆物理色収差**、さらに **50/60Hz 縦続ノッチ IIR ハム除去・MAD+Hermite デクリック・適応 De-Reverb・3-2-1 映像ストリーマ＆同期ビープ付き ADR カウント収録・ITU-R BS.775 5.1ch サラウンド / FOA Ambisonics (AmbiX `W,Y,Z,X` ⇔ バイノーラル復号) 空間音声バス** をすべて Pure Rust・ゼロ `unsafe`・追加外部依存ゼロで完備。
* **Tri-Workspace UX（3 空間）× 6 スタジオ Hub 統合 ＆ `Ctrl+K` ファジー検索コマンドパレット:** 軽快なカット・配置の「メイン編集空間 (AviUtl2)」、グラフエディタ・トラックマット・Pure-Rust エクスプレッションの「モーション合成空間 (AE)」、Fusion / Nuke スタイルの「ノード合成空間」の **3 空間を維持** しつつ、v0.9〜v8.0 の全専門ツールを制作ロール別の **6 スタジオ Hub（`Edit` / `VFX & Motion` / `Color` / `Audio` / `AI & Collab` / `Deliver`）** へ機能ゼロ欠落で完全統合。さらに **`Ctrl+K` / `Ctrl+Shift+P` ファジー検索コマンドパレット**（日英・ロードマップ番号 `#1`〜`#52`・MRU 優先・キーボード完結）から全ツールへ 1 秒で到達できます。
* **GPU-Resident 4K/8K フレームプール ＆ 10,000+ クリップ区間木 $O(\log N)$ ＆ TC 自動コンフォーム ＆ A/B スプリット比較ビュー:** 解像度別バケット＋LRU 予算上限付き **`ResidentFramePool`** を `RenderGraph` に統合し、`f16` パック＋64 MiB ストライプ分割 WGSL コンピュート高速パス（ブレンド/マスク＆ACEScg 色管理・決定論 CPU フォールバック維持）を実装。さらに 10,000+ クリップでも瞬時に描画・スナップ・検索できるフラット配列型 Augmented Interval Tree **`SceneIntervalIndex`**、リール名＆SMPTE 開始タイムコード差分からプロキシ⇔カメラ原本を 1-Undo 切替する **タイムコード自動コンフォーム**、プレビュー上で分割線を直接ドラッグできる **A/B スプリット比較ビュー** を完備。
* **内蔵 MCP ドメイン別モジュール分割（全 86 MCP ツール・完全オフライン AI 連携）:** 内蔵 MCP サーバー（ローカル Streamable HTTP + ベアラートークン認証）をドメイン別 8 サブモジュール群（`edit_nle` / `vfx_motion` / `color_finishing` / `audio_voice` / `ai_collab` / `deliver_system` / `v70_unified` / `v80_cinema_audio`）へ整理し、v8.0 新規 4 ツール（平面トラッカー＆ロトブラシ・SVG モーフィング＆Text-on-Path＆ワープ・シネマ光学 VFX・オーディオ修復＆ADR＆5.1/FOA 空間バス）を含む全 **86 MCP ツール** を提供。GUI と AI の全操作が `CommandBus` を経由し、`Ctrl+Z` Undo/Redo・`history.jsonl` クラッシュ復旧・編集レシピ・承認ゲート・視覚自己検査を保証。
* **データ駆動チャート＆非破壊 VFX ペイント/ワイヤー消し＆ポッドキャスト自動編集＆マルチプラットフォーム 1 クリック納品:** CSV/JSON 4 種インフォグラフィックチャート、Write-On グロー/クローンスタンプ/境界補間ワイヤー消し（Inpainting）/プライバシー秘匿ぼかし、複数話者ダイアリゼーション＆カメラ自動切替＆`-16 LUFS` 均一化ポッドキャスト自動編集、4 媒体自動リフォーマット＆全成果物一括出力パッケージャー。
* **2D スケルトン IK リギング＆自動 BGM 作曲＆スピードランプ＆35mm フィニッシング＆EDL/FCPXML:** 階層 FK・解析的 2-Bone IK ソルバー・二次揺れパペットリグ、6 ジャンル 48kHz オフライン自動 BGM コンポーザー、解析積分スピードランプ＆自動シーンカット検出、35mm ハレーション/アナモルフィックフレア/グレイン/SMPTE バーンイン＆CMX 3600 EDL / FCPXML v1.10 書き出し。
* **プロ・シネマカラー管理＆2D 物理演算＆オフライン生成背景/SFX＆放送 QC:** ACEScg (AP1) / OpenColorIO v2（S-Log3・V-Log・LogC3 IDT、ACES 1.3 ガマット圧縮、ACES/AgX/Reinhard、`.ocio` 書き出し）、決定論的 60Hz 2D リジッドボディ物理演算＆自動ベイク、6 種プロシージャル背景＆6 種 Foley/SFX シンセ、放送 QC 監査（ITU-R BS.1770 / BT.1702）＆高 CTR サムネイル抽出＆単一ファイル HTML5 レビュープレイヤー書き出し。
* **フル GPU コンピュート＆3D メッシュ/glTF 2.0＆オフライン AI 吹替＆P2P CRDT 同時編集＆Git-Friendly NLE:** WGSL コンピュートグレーディング＆スコープ、glTF 2.0 / `.glb` PBR 3D ラスタライザ＆3D カメラトラッカー、オフライン多言語 AI 吹替＆音素リップシンク、Lamport/Vector Clock Op-Based CRDT＆LAN TCP ピア同時共同編集、視覚 Diff＆3-Way マージ、ローカル AI ビジョン、RGB カーブ/HSL/カラーマッチ、手ぶれ補正＆4 点コーナーピン、3D バイノーラル音響＆マルチバンドコンプ＆4× True-Peak リミッター、キネティック字幕＆多言語 CSV、マルチカム相互相関同期、HW デコード＆並列書き出し＆RAM＋SSD 永続フレームキャッシュ、WASM サンドボックス＆SHA-256 公式拡張レジストリまで完備。

---

## ワークスペース構成

| クレート | 役割 |
| :--- | :--- |
| `crates/core` (`ve-core`) | 有理数タイムベース、**v8.0 4 点平面ホモグラフィ追跡＆ロトブラシ・シネマ光学 VFX・ADR セッション＆5.1/FOA 空間バス（`v80_vfx`）・Pure-Rust SVG `<path>` 全コマンドパーサ＆等弧長モーフィング＆Text-on-Path＆4×4 グリッドワープ（`motion_foundry`）**、役割別 6 スタジオ Hub＆`Ctrl+K` ファジー検索カタログ（`studio_hub`）・Augmented Interval Tree $O(\log N)$ 検索（`timeline_index`）・SMPTE TC 自動コンフォーム（`conform`）・A/B 比較設定、データ駆動チャート・VFX ペイント・マルチ納品・2D IK・スピードランプ・35mm 仕上げ＆EDL/FCPXML・ACEScg/OCIO・2D 物理・生成背景・放送 QC/HTML5・glTF 2.0 3D・ColorCurves/HSL/CornerPin/KineticSubtitle・CRDT＆LAN 同時編集・ノード合成グラフ・AI 編集レシピ・Diff/3-Way マージ・エクスプレッション VM・テンプレート・マルチカム・SSD 永続キャッシュ・納品検証＆ZIP、`CommandBus`（Undo/Redo・クラッシュ復旧） |
| `crates/media` (`ve-media`) | FFmpeg / FFprobe 外部 CLI ラッパー（SMPTE 開始タイムコード＆リール名メタデータ自動抽出・HW デコード自動検出＆フォールバック・常駐 `VideoStreamDecoder`・可変プロキシ・Rec.709 / HDR10 PQ 10-bit HEVC・H.264/H.265/VP9+Alpha/PNG 連番・NVENC/QSV/AMF 自動選択）、`symphonia` 音声デコード |
| `crates/audio` (`ve-audio`) | プレビュー／書き出し共通 `AudioGraph`、**v8.0 オーディオ修復（縦続ノッチ IIR 50/60Hz ハム除去・MAD+Hermite デクリック・適応 De-Reverb）・ADR 3-2-1 同期ビープ＆キュー配置・ITU-R BS.775 5.1ch サラウンド＆FOA Ambisonics (AmbiX `W,Y,Z,X` ⇔ バイノーラル復号) マルチチャンネル WAV 書き出し（`restoration`）**、ポッドキャスト話者ダイアリゼーション＆-16 LUFS レベラー、オフライン自動 BGM コンポーザー（6 ジャンル）、Foley/SFX 6 種合成、オフライン多言語 AI 吹替＆音素リップシンク、DSP 16 種（3D バイノーラル・マルチバンドコンプ・4× True-Peak リミッター・STFT NR・ディエッサー・修復 3 種含む）、ボーカル/BGM 分離、マルチカム相互相関同期、マイク直接録音、LUFS 正規化、ダッキング、BPM/ビート検出、`WaveformPeaks`、WASAPI 出力 |
| `crates/plugin-api` (`ve-plugin-api`) | OSS 公開プラグイン API ＋ Pure-Rust ゼロ `unsafe` WebAssembly 1.0 サンドボックス VM＆`plugins/*.wasm` ホットリローダー |
| `crates/render` (`ve-render`) | Linear sRGB（`Rgba16Float`）+ Premultiplied Alpha 合成、**v8.0 4 点平面ホモグラフィ追跡（ZNCC+RANSAC）＆Sobel エッジ吸着/フロー伝播ロトブラシ・SVG パスモーフィング/Text-on-Path/4×4 双三次グリッドワープ＆輝度変位・シネマ光学 VFX（Rack Focus 多角形ボケ・ゴッドレイ・Brown-Conrady 歪み＆物理色収差）・ADR 3-2-1 映像ストリーマ HUD（`v80_render`）**、GPU-Resident 4K/8K フレームプール＆WGSL 高速パス（`gpu_v70`）・区間木 $O(\log N)$ レイヤー探索・A/B スプリット比較、データチャート＆VFX ペイント、2D パペットリグ・カット検出・35mm フィニッシング、ACEScg/OCIO・プロシージャル背景・放送 QC、フル GPU WGSL パイプライン、PBR 3D メッシュ/glTF ラスタライザ＆3D カメラトラッカー、RGB カーブ/HSL/カラーマッチ、手ぶれ補正＆4 点コーナーピン、キネティック字幕、ノード合成、ローカル AI ビジョン、並列バッチ描画、パーティクル＆ベクター＆3D ライト/モーションブラー、`.cube` 3D LUT＆スコープ、ベジェマスク＆トラッカー、PSD 立ち絵、WGSL ホットリロード、ゴールデン視覚回帰テスト |
| `crates/ai` (`ve-ai`) | 自動文字起こし 3 モード（ローカル Whisper/VAD・Gemini・Claude）、Claude 字幕文脈補正、OS 資格情報ストア、課金確認ガード、TTS ファイルペア取込＆ローカル API |
| `crates/mcp` (`ve-mcp`) | 内蔵 MCP サーバー（ローカル Streamable HTTP `127.0.0.1` + 起動時ベアラートークン認証、**ドメイン別 8 サブモジュール分割構成 `crates/mcp/src/tools/`**、v8.0 新規 4 ツール［平面追跡＆ロト・SVG モーフィング＆Text-on-Path＆ワープ・シネマ光学 VFX・オーディオ修復＆ADR＆5.1/FOA 空間バス］含む全 **86 ツール**） |
| `crates/app` (`ve-app`) | `eframe`（`wgpu`）+ `egui_dock` デスクトップアプリ（3 空間 UI・6 スタジオ Hub 統合ウィンドウ＆`Ctrl+K` コマンドパレット・**v8.0 VFX & Motion Hub 3 タブ［4 点ドラッグ平面追跡/ロト・SVG/Text-on-Path/ワープ・シネマ光学 VFX］＆Audio Hub［修復 DSP/ADR/360° 5.1・FOA レーダー］**・プレビュー直接ドラッグ A/B 比較・10,000+ 区間木ビューポートカリング・SMPTE TC/リール自動登録・設定/SHA-256 拡張レジストリ・スコープ・RAM＋SSD キャッシュ・並列書き出し・レンダーキュー） |

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
