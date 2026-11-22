# Next-Gen Hybrid Video Editor (`video-editing-app_AG`)

After Effects の**高度な表現力・アニメーション制御**、AviUtl2 の**軽快なタイムライン編集（NLE）**、そして **AI ネイティブな操作基盤（内蔵 MCP）** を単一環境で融合する Pure Rust（ゼロ `unsafe`）動画編集ソフトです。

* **仕様書 (v1.1):** [docs/spec.md](docs/spec.md)
* **アーキテクチャ設計書:** [docs/architecture.md](docs/architecture.md)
* **プラグイン＆カスタム WGSL 開発ガイド:** [docs/plugin-guide.md](docs/plugin-guide.md)
* **ロードマップ:** [docs/future_features.md](docs/future_features.md)
* **コントリビューションガイド:** [CONTRIBUTING.md](CONTRIBUTING.md)
* **変更履歴:** [CHANGELOG.md](CHANGELOG.md)

---

## 主な特徴（v4.0.0 マスタースタジオ完成版 — 全 36 ロードマップ項目達成）

* **Tri-Workspace UX（3 空間シームレス切替） & プレビュー直接操作 & カスタマイズ:** 軽快なカット・配置の「メイン編集空間 (AviUtl2)」、グラフエディタ・トラックマット・エフェクトチェーン・Pure-Rust エクスプレッションの「モーション合成空間 (AE)」、そして **Fusion / Nuke スタイルの「ノード合成空間」（チャンネル分離・マスク合流・3D 深度フォグ・レイヤー⇔ノード双方向同期）** をワンクリック切替。カスタムキーバインド・多言語 UI（日/英）・4 テーマ・プレビュー直接操作ギズモ＆自由ベジェマスク頂点ドラッグに対応。
* **プロ・シネマカラー管理＆2D 物理演算＆オフライン生成背景/SFX＆放送 QC（v4.0 新搭載）:** **ACEScg (AP1) / OpenColorIO v2** 広色域パイプライン（Sony S-Log3・Panasonic V-Log・ARRI LogC3 IDT、ACES 1.3 ガマット圧縮、ACES Fitted / AgX / Reinhard トーンマップ、Display P3 / Rec.2020 ODT、`.ocio` YAML 書き出し）；決定論的 60Hz **2D リジッドボディ物理演算エンジン**（バウンス落下・ドミノ連鎖衝突・無重力・ボテックス＆1-Undo キーフレーム自動ベイク）；6 種 **プロシージャル動画背景ジェネレーター**（オーロラ・シネマ玉ボケ・サイバー 3D グリッド・リキッドマーブル・ワープ星雲・アブストラクト波）＆ 6 種 **48kHz ステレオ Foley/SFX 効果音シンセ**；**放送コンプライアンス QC 監査**（ITU-R BS.1770 True-Peak/LUFS・ITU-R BT.1702 光過敏フラッシュ検査）＆ `1280×720` 高 CTR ヒーローサムネイル自動抽出＆**ゼロサーバー単一ファイル HTML5 インタラクティブレビュープレイヤー書き出し**。
* **フル GPU コンピュート ＆ 3D メッシュ/glTF 2.0 ＆ オフライン AI 吹替 ＆ P2P CRDT 同時編集:** `@compute @workgroup_size(8, 8)` WGSL シェーダーによる **GPU カラーグレーディング＆原子ヒストグラムスコープ＆4K ベンチマーク**；Pure-Rust **glTF 2.0 / `.glb` パーサ＆PBR 3D ラスタライザ（キューブ/球体/トーラス/3D 立体押し出しテキスト・GGX 鏡面・Schlick Fresnel・環境反射）＆3D カメラトラッカー**；クラウド送信ゼロの **オフライン多言語 AI 吹替 WAV 合成＆音素リップシンク自動同期**；Lamport/Vector Clock による **可換・冪等 Op-Based CRDT ＆ トークン認証付き LAN TCP ピア同時共同編集（ライブカーソル共有）**。
* **シネマ色調＆VFX トラッキング＆3D 空間音響＆キネティック字幕:** Fritsch-Carlson 単調スプライン **RGB/マスタートーンカーブ**、マット可視化付き **HSL 二次色補正クオリファイア**、Reinhard **自動ショットカラーマッチ**、ゼロ位相 **自動 2D 手ぶれ補正**＆厳密 $3\times 3$ ホモグラフィ **4 点コーナーピン**、Woodworth ITD+ILD **3D バイノーラル空間オーディオ**＆**3 バンドマルチバンドコンプ**＆**4× True-Peak（-1.0 dBTP）リミッター**、**ワード単位キネティック字幕**＆**多言語 CSV ローカライズ 1-Undo 切替**。
* **完全オフライン ローカル AI ビジョン ＆ 自律 AI 編集ディレクター（70 MCP ツール）:** グリーンバック不要人物切り抜き・9:16 自動リフレーム・ローカル超解像・オプティカルフロー補間；内蔵 MCP サーバー（ローカル Streamable HTTP + ベアラートークン認証・**70 ツール**）と GUI 全操作が `CommandBus` を経由し、`Ctrl+Z` Undo/Redo・`history.jsonl` クラッシュ復旧・YAML/JSON 編集レシピ・人間承認ゲート・視覚自己検査を保証。
* **Git-Friendly NLE・マルチカム・HW 並列合成・配布エコシステム:** 視覚 Diff＆3-Way 意味的ブランチマージ＆Frame.io/YouTube レビュー連携、音声相互相関マルチカム同期＆ライブスイッチング、FFmpeg HW デコード＆並列書き出し＆RAM＋SSD 永続フレームキャッシュ、プロ NLE トリム＆トラックミキサー＆マイク録音＆STFT NR/ディエッサー/ボーカル分離、`.cube` 3D LUT＆スコープ、Rec.709＆HDR10 PQ 10-bit、WASM サンドボックス＆SHA-256 公式拡張レジストリ、納品検証＆ZIP アーカイブまで完備。

---

## ワークスペース構成

| クレート | 役割 |
| :--- | :--- |
| `crates/core` (`ve-core`) | 有理数タイムベース、プロジェクトモデル（**ACEScg/OCIO カラー管理・2D 剛体物理エンジン・`GenerativeBgConfig`・放送 QC ＆ HTML5 レビュー書き出し**・`Mesh3DConfig`/glTF 2.0・`ColorCurves`/`HslQualifier`/`CornerPinConfig`/`KineticSubtitleConfig`）、Op-Based CRDT ＆ LAN TCP 同時共同編集、ノード合成グラフ＆双方向変換、AI 編集レシピ（YAML/JSON）、タイムライン視覚 Diff・3-Way マージ・レビュー連携、多言語字幕 CSV、エクスプレッション VM、テンプレート、マルチカム、SSD 永続キャッシュ、納品検証＆ZIP、キーバインド/設定/SHA-256 拡張レジストリ、`CommandBus`（Undo/Redo・クラッシュ復旧） |
| `crates/media` (`ve-media`) | FFmpeg / FFprobe 外部 CLI ラッパー（HW デコード自動検出＆フォールバック・常駐 `VideoStreamDecoder`・可変プロキシ・Rec.709 / HDR10 PQ 10-bit HEVC・H.264/H.265/VP9+Alpha/PNG 連番・NVENC/QSV/AMF 自動選択）、`symphonia` 音声デコード |
| `crates/audio` (`ve-audio`) | プレビュー／書き出し共通 `AudioGraph`、**オフラインプロシージャル Foley/SFX 6 種 48kHz WAV 合成**、オフライン多言語 AI 吹替 3 フォルマント WAV 合成＆音素リップシンクアライナー、DSP 13 種（3D バイノーラル空間オーディオ・3 バンドマルチバンドコンプ・4× True-Peak リミッター・STFT NR・ディエッサー含む）、オフラインボーカル/BGM 分離、マルチカム相互相関自動同期、マイク直接録音、True-Peak (dBTP) / LUFS 計測＆正規化、ダッキング、BPM/ビート検出、`WaveformPeaks`、Windows WASAPI 出力 |
| `crates/plugin-api` (`ve-plugin-api`) | OSS 公開プラグイン API ＋ Pure-Rust ゼロ `unsafe` WebAssembly 1.0 サンドボックス VM＆`plugins/*.wasm` ホットリローダー |
| `crates/render` (`ve-render`) | Linear sRGB（`Rgba16Float`）+ Premultiplied Alpha 合成、**ACEScg/OCIO パイプライン統合・6 種プロシージャル背景描画・高 CTR サムネイル自動抽出＆放送 QC 映像計測**、フル GPU WGSL コンピュートパイプライン（色調＆原子スコープ＆4K ベンチ）、PBR 3D メッシュ/glTF ラスタライザ＆3D カメラトラッカー、RGB トーンカーブ/HSL/自動カラーマッチ、自動 2D 手ぶれ補正＆4 点コーナーピン、動的キネティック字幕、ノード合成グラフ評価器、ローカル AI ビジョン、並列バッチ描画、パーティクル＆ベクター＆3D ライト/モーションブラー、`.cube` 3D LUT＆スコープ、ベジェマスク＆トラッカー、PSD 立ち絵、WGSL ホットリロード、ゴールデン視覚回帰テスト |
| `crates/ai` (`ve-ai`) | 自動文字起こし 3 モード（ローカル Whisper/VAD・Gemini・Claude）、Claude 字幕文脈補正、OS 資格情報ストア、課金確認ガード、TTS ファイルペア取込＆ローカル API |
| `crates/mcp` (`ve-mcp`) | 内蔵 MCP サーバー（ローカル Streamable HTTP `127.0.0.1` + 起動時ベアラートークン認証、**ACEScg/OCIO・2D 物理演算ベイク・プロシージャル背景/Foley SFX・放送 QC/サムネ/HTML5 レビュー**・GPU パイプライン・3D/glTF＆カメラ追跡・オフライン多言語吹替＆リップシンク・CRDT 同時共同編集・シネマ色調/VFX/3D 音響・AI 編集ディレクター・ノードグラフ・ローカル AI ビジョン含む全 **70 ツール**） |
| `crates/app` (`ve-app`) | `eframe`（`wgpu`）+ `egui_dock` デスクトップアプリ（3 空間 UI・**v4.0 マスタースタジオ［ACEScg/OCIO・物理演算軌跡＆ベイク・背景＆Foley SFX・放送 QC＆HTML5 レビュー］**・v3.0 ワールド＆GPU スタジオ・v2.1 シネマ＆クリエイタースタジオ・v2.0 フロンティアスタジオ・v1.0 プロダクションスタジオ・設定/SHA-256 拡張レジストリ・スコープ・RAM＋SSD キャッシュ・並列書き出し・レンダーキュー） |

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
