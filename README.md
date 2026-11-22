# Next-Gen Hybrid Video Editor (`video-editing-app_AG`)

After Effects の**高度な表現力・アニメーション制御**、AviUtl2 の**軽快なタイムライン編集（NLE）**、そして **AI ネイティブな操作基盤（内蔵 MCP）** を単一環境で融合する Pure Rust（ゼロ `unsafe`）動画編集ソフトです。

* **仕様書 (v1.1):** [docs/spec.md](docs/spec.md)
* **アーキテクチャ設計書:** [docs/architecture.md](docs/architecture.md)
* **プラグイン＆カスタム WGSL 開発ガイド:** [docs/plugin-guide.md](docs/plugin-guide.md)
* **ロードマップ:** [docs/future_features.md](docs/future_features.md)
* **コントリビューションガイド:** [CONTRIBUTING.md](CONTRIBUTING.md)
* **変更履歴:** [CHANGELOG.md](CHANGELOG.md)

---

## 主な特徴（v2.0.0 フロンティア版 — 全 24 ロードマップ項目完了）

* **Tri-Workspace UX（3 空間シームレス切替） & プレビュー直接操作 & カスタマイズ:** 軽快なカット・配置の「メイン編集空間 (AviUtl2)」、グラフエディタ・トラックマット・エフェクトチェーン・Pure-Rust エクスプレッションの「モーション合成空間 (AE)」、そして **Fusion / Nuke スタイルの「ノード合成空間」（チャンネル分離 RGB/Alpha/Luma/3D Depth・マスク合流・3D 深度フォグ・レイヤー⇔ノード双方向同期）** をワンクリック切替。カスタムキーバインド（AviUtl2 / Premiere / DaVinci / After Effects）・多言語 UI（日/英）・4 テーマ＆フォント倍率・プレビュー直接操作ギズモ＆自由ベジェマスク頂点ドラッグに対応。
* **完全オフライン ローカル AI ビジョン（クラウド送信ゼロ）:** グリーンバック不要の人物切り抜き（自動ロトブラシ `ai_segment`）、顔・被写体・動き追尾による **16:9→9:16 / 1:1 インテリジェント自動リフレーム**、Lanczos-3＋エッジ誘導 **ローカル超解像**（2×/4× / `ai_super_res`）、双方向オクルージョン保護 **オプティカルフロー補間スローモーション**（Pure-Rust 内蔵＋オプショナルローカル `models/*.onnx` 連携）。
* **マルチエージェント AI 編集ディレクター ＆ 人間と AI の完全対等操作（58 MCP ツール）:** 内蔵 MCP サーバー（ローカル Streamable HTTP + 起動時ベアラートークン認証・**58 ツール**）と GUI のすべてが `CommandBus` を経由し、`Ctrl+Z` Undo/Redo と `history.jsonl` クラッシュ復旧を保証。YAML/JSON 編集レシピの記録・共有、5 専門エージェントロール、**人間承認ゲート**、および `render_frame_preview` **自動視覚自己検査**による安全な反復編集を実現。
* **Git-Friendly NLE 共同編集・視覚 Diff・3-Way マージ・タイムコードレビュー:** 1 フォルダ管理＋決定論的 JSON スキーマを活かし、スナップショット間の **タイムライン視覚 Diff**、非競合を自動統合する **3-Way 意味的ブランチマージ**（1-Undo 適用）、**Frame.io（CSV/JSON）＆YouTube タイムスタンプレビューコメント**入出力とタイムラインマーカー自動変換に対応。
* **マルチカメラ・HW デコード/並列合成・プロ NLE・プロ音声・配布エコシステム:** 音声相互相関マルチカム自動同期＆2×2/3×3 ライブスイッチング、FFmpeg HW デコード＆マルチスレッド並列書き出し＆`wgpu` コンピュートコンポジタ＆RAM＋SSD 永続フレームキャッシュ、スリップ/ロール/スライドトリム＆グループ化＆Vol/Pan オートメーションカーブ、トラックミキサー＆マイク直接アフレコ＆STFT ノイズリダクション/ディエッサー/ボーカル分離、パーティクル＆ベクターシェイプ＆3D ライト＆モーションブラー、`.cube` 3D LUT＆スコープ、Rec.709＆HDR10 PQ 10-bit、WASM サンドボックス＆SHA-256 公式拡張レジストリ、納品検証＆ZIP アーカイブまで完備。

---

## ワークスペース構成

| クレート | 役割 |
| :--- | :--- |
| `crates/core` (`ve-core`) | 有理数タイムベース、プロジェクトモデル、**ノード合成グラフ＆レイヤー双方向変換**、**AI 編集レシピ（Pure-Rust YAML/JSON）**、**タイムライン視覚 Diff・3-Way 意味的ブランチマージ・Frame.io/YouTube レビュー連携**、エクスプレッション VM、テンプレート、マルチカム、SSD 永続キャッシュ、納品検証＆ZIP、キーバインド/設定/SHA-256 拡張レジストリ、`CommandBus`（Undo/Redo・クラッシュ復旧） |
| `crates/media` (`ve-media`) | FFmpeg / FFprobe 外部 CLI ラッパー（HW デコード自動検出＆フォールバック・常駐 `VideoStreamDecoder`・可変プロキシ・Rec.709 / HDR10 PQ 10-bit HEVC・H.264/H.265/VP9+Alpha/PNG 連番・NVENC/QSV/AMF 自動選択）、`symphonia` 音声デコード |
| `crates/audio` (`ve-audio`) | プレビュー／書き出し共通 `AudioGraph`（ソロ/等パワーパン/Vol・Pan オートメーション/Aux リバーブセンド）、DSP 10 種（STFT スペクトル減算 NR・ディエッサー含む）、オフラインボーカル/BGM 分離、2 段階相互相関マルチカム自動同期、マイク直接アフレコ録音、LUFS 正規化、ダッキング、BPM/ビート検出、`WaveformPeaks`、Windows WASAPI 出力 |
| `crates/plugin-api` (`ve-plugin-api`) | OSS 公開プラグイン API ＋ Pure-Rust ゼロ `unsafe` WebAssembly 1.0 サンドボックス VM＆`plugins/*.wasm` ホットリローダー |
| `crates/render` (`ve-render`) | Linear sRGB（`Rgba16Float`）+ Premultiplied Alpha 合成、**ノード合成グラフ評価器（3D 深度・チャンネル分離/合流）**、**完全オフライン ローカル AI ビジョン（自動切り抜き・顔/被写体追尾リフレーム・超解像・オプティカルフロー補間）**、並列バッチ描画＆`wgpu` コンピュートコンポジタ、パーティクル＆ベクター＆3D ライト/モーションブラー、`.cube` 3D LUT＆スコープ、ベジェマスク＆トラッカー、PSD 立ち絵、WGSL ホットリロード、ゴールデン視覚回帰テスト |
| `crates/ai` (`ve-ai`) | 自動文字起こし 3 モード（ローカル Whisper/VAD・Gemini・Claude）、Claude 字幕文脈補正、OS 資格情報ストア、課金確認ガード、TTS ファイルペア取込＆ローカル API |
| `crates/mcp` (`ve-mcp`) | 内蔵 MCP サーバー（ローカル Streamable HTTP `127.0.0.1` + 起動時ベアラートークン認証、**マルチエージェント AI 編集ディレクターオーケストレータ**・ノードグラフ・ローカル AI ビジョン・共同編集 Diff/マージ/レビュー含む全 **58 ツール**） |
| `crates/app` (`ve-app`) | `eframe`（`wgpu`）+ `egui_dock` デスクトップアプリ（**3 空間 UI［メイン編集／AE モーション／Fusion-Nuke ノード合成］**・**v2.0 フロンティアスタジオ［ローカル AI ビジョン・AI 編集ディレクター・共同編集 Diff＆レビュー］**・v1.0 プロダクションスタジオ・設定/公式 SHA-256 拡張レジストリ・スコープ・RAM＋SSD キャッシュ・並列書き出し・レンダーキュー） |

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
