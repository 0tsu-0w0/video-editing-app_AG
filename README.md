# Next-Gen Hybrid Video Editor (`video-editing-app_AG`)

After Effects の**高度な表現力・アニメーション制御**、AviUtl2 の**軽快なタイムライン編集（NLE）**、そして **AI ネイティブな操作基盤（内蔵 MCP）** を単一環境で融合する Pure Rust（ゼロ `unsafe`）動画編集ソフトです。

* **仕様書 (v1.1):** [docs/spec.md](docs/spec.md)
* **アーキテクチャ設計書:** [docs/architecture.md](docs/architecture.md)
* **プラグイン＆カスタム WGSL 開発ガイド:** [docs/plugin-guide.md](docs/plugin-guide.md)
* **ロードマップ:** [docs/future_features.md](docs/future_features.md)
* **コントリビューションガイド:** [CONTRIBUTING.md](CONTRIBUTING.md)
* **変更履歴:** [CHANGELOG.md](CHANGELOG.md)

---

## 主な特徴（v6.0.0 クリエイター Pro スタジオ完成版 — 全 44 ロードマップ項目達成）

* **Tri-Workspace UX（3 空間シームレス切替） & プレビュー直接操作 & カスタマイズ:** 軽快なカット・配置の「メイン編集空間 (AviUtl2)」、グラフエディタ・トラックマット・エフェクトチェーン・Pure-Rust エクスプレッションの「モーション合成空間 (AE)」、そして **Fusion / Nuke スタイルの「ノード合成空間」（チャンネル分離・マスク合流・3D 深度フォグ・レイヤー⇔ノード双方向同期）** をワンクリック切替。カスタムキーバインド・多言語 UI（日/英）・4 テーマ・プレビュー直接操作ギズモ＆自由ベジェマスク頂点ドラッグに対応。
* **データ駆動チャート＆非破壊 VFX ペイント/ワイヤー消し＆ポッドキャスト自動編集＆マルチプラットフォーム 1 クリック納品（v6.0 新搭載）:** CSV/JSON から 4 種（棒・折れ線・ドーナツ・KPI カウンター）を決定論イージング描画する **データ駆動インフォグラフィック・チャート**；Write-On グローブラシ・クローンスタンプ・境界補間 **ワイヤー/不要物消し（Inpainting）**・局所プライバシーモザイク秘匿の **非破壊 VFX ペイントパス**；複数話者ダイアリゼーション・アクティブ話者カメラ自動切替・`-16 LUFS` 均一化レベラーの **マルチトラック・ポッドキャスト自動編集**；さらに 1 本のマスターから YouTube 16:9 / Shorts 9:16 / Square 1:1 / 4K 派生シーン＋媒体別 SRT・チャプター・1280×720 サムネイル・EDL・FCPXML・QC レポート・`manifest.json` を一括出力する **マルチプラットフォーム納品パッケージャー**。
* **2D スケルトン IK リギング＆自動 BGM 作曲＆スピードランプ＆35mm フィニッシング＆EDL/FCPXML:** 階層 FK・解析的 **2-Bone IK ソルバー**・髪/リボン二次揺れを備える **2D スケルトンパペットリグ**；著作権フリー完全オフラインで 6 ジャンルの 48kHz ステレオ楽曲＆小節マーカーを自動作曲する **オフライン BGM コンポーザー**；解析積分 **スピードランプ** ＆ 64 ビン RGB ヒストグラム χ² **自動シーンカット検出＆1-Undo 自動分割**；35mm フィルム **ハレーション・アナモルフィック水平フレア・グレイン・レターボックス・SMPTE 透かしバーンイン**、業界標準 **CMX 3600 EDL**＆**Final Cut Pro XML v1.10 (FCPXML)** 書き出しに対応。
* **プロ・シネマカラー管理＆2D 物理演算＆オフライン生成背景/SFX＆放送 QC:** **ACEScg (AP1) / OpenColorIO v2** 広色域パイプライン（S-Log3・V-Log・LogC3 IDT、ACES 1.3 ガマット圧縮、ACES Fitted / AgX / Reinhard、Display P3 / Rec.2020 ODT、`.ocio` 書き出し）；決定論的 60Hz **2D リジッドボディ物理演算**＆キーフレーム自動ベイク；6 種 **プロシージャル動画背景**＆6 種 **48kHz Foley/SFX 効果音シンセ**；**放送 QC 監査**（ITU-R BS.1770 / BT.1702）＆高 CTR サムネイル抽出＆**ゼロサーバー単一ファイル HTML5 レビュープレイヤー書き出し**。
* **フル GPU コンピュート ＆ 3D メッシュ/glTF 2.0 ＆ オフライン AI 吹替 ＆ P2P CRDT 同時編集:** `@compute @workgroup_size(8, 8)` WGSL シェーダーによる **GPU カラーグレーディング＆原子ヒストグラムスコープ＆4K ベンチマーク**；Pure-Rust **glTF 2.0 / `.glb` パーサ＆PBR 3D ラスタライザ＆3D カメラトラッカー**；クラウド送信ゼロの **オフライン多言語 AI 吹替 WAV 合成＆音素リップシンク自動同期**；Lamport/Vector Clock による **Op-Based CRDT ＆ トークン認証付き LAN TCP ピア同時共同編集**。
* **完全オフライン ローカル AI ビジョン ＆ 自律 AI 編集ディレクター（78 MCP ツール）:** グリーンバック不要人物切り抜き・9:16 自動リフレーム・ローカル超解像・オプティカルフロー補間；内蔵 MCP サーバー（ローカル Streamable HTTP + ベアラートークン認証・**78 ツール**）と GUI 全操作が `CommandBus` を経由し、`Ctrl+Z` Undo/Redo・`history.jsonl` クラッシュ復旧・YAML/JSON 編集レシピ・人間承認ゲート・視覚自己検査を保証。
* **Git-Friendly NLE・マルチカム・HW 並列合成・シネマ色調/VFX/3D 音響・配布エコシステム:** 視覚 Diff＆3-Way 意味的ブランチマージ＆Frame.io/YouTube レビュー連携、RGB トーンカーブ/HSL/自動カラーマッチ、自動 2D 手ぶれ補正＆4 点コーナーピン、3D バイノーラル音響＆マルチバンドコンプ＆4× True-Peak リミッター、キネティック字幕＆多言語 CSV 切替、音声相互相関マルチカム同期＆ライブスイッチング、FFmpeg HW デコード＆並列書き出し＆RAM＋SSD 永続フレームキャッシュ、プロ NLE トリム＆トラックミキサー＆マイク録音＆STFT NR/ディエッサー/ボーカル分離、`.cube` 3D LUT＆スコープ、Rec.709＆HDR10 PQ 10-bit、WASM サンドボックス＆SHA-256 公式拡張レジストリ、納品検証＆ZIP アーカイブまで完備。

---

## ワークスペース構成

| クレート | 役割 |
| :--- | :--- |
| `crates/core` (`ve-core`) | 有理数タイムベース、プロジェクトモデル（**データ駆動チャート `DataChartConfig`・非破壊 VFX ペイント `VfxPaintConfig`・マルチプラットフォーム自動リフォーマット＆納品マニフェスト `multi_deliver`**・2D スケルトン/2-Bone IK リグ・スピードランプ解析積分・シネマフィニッシング＆CMX 3600 EDL / FCPXML v1.10・ACEScg/OCIO カラー管理・2D 剛体物理エンジン・`GenerativeBgConfig`・放送 QC ＆ HTML5 レビュー・`Mesh3DConfig`/glTF 2.0・`ColorCurves`/`HslQualifier`/`CornerPinConfig`/`KineticSubtitleConfig`）、Op-Based CRDT ＆ LAN TCP 同時共同編集、ノード合成グラフ、AI 編集レシピ、視覚 Diff/3-Way マージ、エクスプレッション VM、テンプレート、マルチカム、SSD 永続キャッシュ、納品検証＆ZIP、`CommandBus`（Undo/Redo・クラッシュ復旧） |
| `crates/media` (`ve-media`) | FFmpeg / FFprobe 外部 CLI ラッパー（HW デコード自動検出＆フォールバック・常駐 `VideoStreamDecoder`・可変プロキシ・Rec.709 / HDR10 PQ 10-bit HEVC・H.264/H.265/VP9+Alpha/PNG 連番・NVENC/QSV/AMF 自動選択）、`symphonia` 音声デコード |
| `crates/audio` (`ve-audio`) | プレビュー／書き出し共通 `AudioGraph`、**マルチトラック・ポッドキャスト話者ダイアリゼーション＆-16 LUFS 自動レベラー（`podcast_studio`）**、オフライン自動 BGM コンポーザー（6 ジャンル・48kHz WAV ＆ 小節マーカー）、オフライン Foley/SFX 6 種合成、オフライン多言語 AI 吹替 WAV 合成＆音素リップシンクアライナー、DSP 13 種（3D バイノーラル・3 バンドマルチバンドコンプ・4× True-Peak リミッター・STFT NR・ディエッサー含む）、ボーカル/BGM 分離、マルチカム相互相関同期、マイク直接録音、True-Peak / LUFS 正規化、ダッキング、BPM/ビート検出、`WaveformPeaks`、WASAPI 出力 |
| `crates/plugin-api` (`ve-plugin-api`) | OSS 公開プラグイン API ＋ Pure-Rust ゼロ `unsafe` WebAssembly 1.0 サンドボックス VM＆`plugins/*.wasm` ホットリローダー |
| `crates/render` (`ve-render`) | Linear sRGB（`Rgba16Float`）+ Premultiplied Alpha 合成、**データ駆動インフォグラフィックチャート描画＆非破壊 VFX ペイント（Write-On グロー・クローンスタンプ・境界補間ワイヤー消し・モザイク秘匿）**、2D パペットリグ描画・RGB ヒストグラム χ² 自動シーンカット検出・35mm ハレーション/アナモルフィックフレア/グレイン/バーンイン、ACEScg/OCIO パイプライン・6 種プロシージャル背景・高 CTR サムネイル抽出＆放送 QC 映像計測、フル GPU WGSL コンピュートパイプライン、PBR 3D メッシュ/glTF ラスタライザ＆3D カメラトラッカー、RGB カーブ/HSL/自動カラーマッチ、手ぶれ補正＆4 点コーナーピン、動的キネティック字幕、ノード合成、ローカル AI ビジョン、並列バッチ描画、パーティクル＆ベクター＆3D ライト/モーションブラー、`.cube` 3D LUT＆スコープ、ベジェマスク＆トラッカー、PSD 立ち絵、WGSL ホットリロード、ゴールデン視覚回帰テスト |
| `crates/ai` (`ve-ai`) | 自動文字起こし 3 モード（ローカル Whisper/VAD・Gemini・Claude）、Claude 字幕文脈補正、OS 資格情報ストア、課金確認ガード、TTS ファイルペア取込＆ローカル API |
| `crates/mcp` (`ve-mcp`) | 内蔵 MCP サーバー（ローカル Streamable HTTP `127.0.0.1` + 起動時ベアラートークン認証、**データチャート・VFX ペイント/ワイヤー消し・ポッドキャスト自動編集・マルチプラットフォーム納品パッケージャー**・2D リグ/IK・自動 BGM・スピードランプ/カット検出・シネマ仕上げ/EDL/FCPXML・ACEScg/OCIO・2D 物理・背景/Foley・放送 QC・GPU/3D/吹替/CRDT 含む全 **78 ツール**） |
| `crates/app` (`ve-app`) | `eframe`（`wgpu`）+ `egui_dock` デスクトップアプリ（3 空間 UI・**v6.0 クリエイター Pro スタジオ［データチャート・VFX ペイント/ワイヤー消し・ポッドキャスト自動編集・マルチプラットフォーム納品］**・v5.0 プロフィニッシング＆リギング・v4.0 マスタースタジオ・v3.0 ワールド＆GPU スタジオ・v2.1 シネマ＆クリエイタースタジオ・v2.0 フロンティアスタジオ・v1.0 プロダクションスタジオ・設定/SHA-256 拡張レジストリ・スコープ・RAM＋SSD キャッシュ・並列書き出し・レンダーキュー） |

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
