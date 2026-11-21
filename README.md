# Next-Gen Hybrid Video Editor (`video-editing-app_AG`)

After Effects の**高度な表現力・アニメーション制御**、AviUtl2 の**軽快なタイムライン編集（NLE）**、そして **AI ネイティブな操作基盤（内蔵 MCP）** を単一環境で融合する Pure Rust（ゼロ `unsafe`）動画編集ソフトです。

* **仕様書 (v1.1):** [docs/spec.md](docs/spec.md)
* **アーキテクチャ設計書:** [docs/architecture.md](docs/architecture.md)
* **プラグイン＆カスタム WGSL 開発ガイド:** [docs/plugin-guide.md](docs/plugin-guide.md)
* **ロードマップ:** [docs/future_features.md](docs/future_features.md)
* **コントリビューションガイド:** [CONTRIBUTING.md](CONTRIBUTING.md)
* **変更履歴:** [CHANGELOG.md](CHANGELOG.md)

---

## 主な特徴（v1.0.0 プロダクション完成版）

* **Dual-Workspace UX（2空間分離） & プレビュー直接操作 & カスタマイズ:** 軽快なカット・配置の「メイン編集空間 (AviUtl2)」と、グラフエディタ・トラックマット・エフェクトチェーン・**Pure-Rust エクスプレッション式アニメーション**の「モーション合成空間 (AE)」をワンクリック切替。**カスタムキーバインド（AviUtl2 / Premiere / DaVinci / After Effects プリセット＆JSON 入出力）・多言語 UI（日本語/英語）・4 テーマ＆フォント倍率アクセシビリティ**、プレビュー直接操作ギズモ＆**自由ベジェマスク頂点ドラッグ**に対応。
* **マルチカメラ編集 ＆ プロ NLE 操作 ＆ 爆速カット/音ハメ/テンプレート:** **2 段階正規化相互相関によるマルチカム音声自動同期・2×2/3×3 ライブスイッチング（数字キー `1`〜`9`）**、**スリップ／ロール／スライドトリム・クリップグルーピング＆A/V リンク解除（`Ctrl+G`/`Ctrl+Shift+G`）・ボリューム/パン オートメーションカーブ描画**、`J・K・L` シャトル再生、テキストベース編集＆日本語フィラー自動除去、BGM ビート自動検出＆スナップ、モーショングラフィックス（`.mogrt` 相当）＆プロジェクト雛形、`presets/*.json` プリセット管理。
* **HW デコード ＆ マルチスレッド並列合成 ＆ SSD 永続フレームキャッシュ:** FFmpeg HW デコード（`d3d11va` / `dxva2` / `vaapi` / `videotoolbox`）自動検出＆安全フォールバック、解像度/コーデック可変プロキシ、**マルチスレッド並列バッチ書き出し**、`wgpu` **コンピュートシェーダーコンポジタ**、**RAM プレビュー（96 フレーム先読み）＋SSD 永続フレームキャッシュ（`cache/frames/`）**、自動バックアップ（`backups/`）を搭載。
* **人間と AI の完全対等な操作系（54 MCP ツール）:** GUI 操作も内蔵 MCP サーバー（ローカル Streamable HTTP + 起動時ベアラートークン認証・**54 ツール**）もすべて `CommandBus` を経由し、`Ctrl+Z` Undo/Redo と `history.jsonl` クラッシュ復旧を保証。
* **Linear sRGB (`Rgba16Float`) + 高度 2D/3D 合成 + カラーマネージメント + WASM/SHA-256 公式レジストリ:** 決定論的 2D/3D パーティクル（6 種）・自由ベジェベクターシェイプ（グラデーション塗り・破線・トリムパス）・3D ライト＆シャドウキャッチャー・シャッター角モーションブラー、3-Way カラーホイール・`.cube` 3D LUT・リアルタイムビデオスコープ（Waveform / Parade / Vectorscope / Histogram）、**Rec.709 SDR & HDR10 PQ 10-bit（BT.2020 / ST 2084）書き出し**、カスタム WGSL ホットリロード、Pure-Rust WebAssembly 1.0 サンドボックス VM（`plugins/*.wasm`）、**SHA-256 署名検証＆サンドボックス権限明示付き公式拡張レジストリブラウザ**に対応。
* **プロ音声ポストプロダクション・AI 連携・納品チェック＆ZIP パッケージ:** **トラックミキサー（パン/ソロ/バス/Aux リバーブセンド）・マイク直接アフレコ録音・Pure-Rust STFT スペクトル減算ノイズリダクション＆ディエッサー＆オフラインボーカル/BGM 分離**、BGM 自動ダッキング＆オーディオリアクティブ、Whisper/Gemini/Claude 文字起こし、VOICEVOX/COEIROINK 連携、PSD ネイティブ立ち絵口パク、AI ハイライト＆9:16 ショート自動切り抜き、SRT/ASS/`.exo`/OTIO 互換、**書き出し前検証チェックリスト＆Pure-Rust ZIP アーカイブパッケージ化**に対応。

---

## ワークスペース構成

| クレート | 役割 |
| :--- | :--- |
| `crates/core` (`ve-core`) | 有理数タイムベース、プロジェクトモデル、ベジェ曲線、Pure-Rust エクスプレッション VM、モーショングラフィックス＆プロジェクトテンプレート、**マルチカム同期/切替**、**SSD 永続フレームキャッシュ (`cache/frames/`)**、**納品検証＆Pure-Rust ZIP アーカイブ**、**キーバインド/多言語/テーマ設定＆SHA-256 公式拡張レジストリ**、`CommandBus`（Undo/Redo・スリップ/ロール/スライド/グループ/ミキサー・Smart Precompose・クラッシュ復旧） |
| `crates/media` (`ve-media`) | FFmpeg / FFprobe 外部 CLI ラッパー（**HW デコード自動検出＆フォールバック**・常駐 `VideoStreamDecoder`・可変プロキシ・**Rec.709 / HDR10 PQ 10-bit HEVC**・H.264/H.265/VP9+Alpha/PNG 連番・NVENC/QSV/AMF 自動選択）、`symphonia` 音声デコード |
| `crates/audio` (`ve-audio`) | プレビュー／書き出し共通 `AudioGraph`（**ソロ/等パワーパン/Vol・Pan オートメーション/Aux リバーブセンド**）、DSP 10 種（**STFT スペクトル減算 NR・ディエッサー**含む）、**オフラインボーカル/BGM 分離**、**2 段階相互相関マルチカム自動同期**、**マイク直接アフレコ録音 (`VoiceoverRecorder`)**、LUFS 正規化、ダッキング、BPM/ビート検出、`WaveformPeaks`、Windows WASAPI 出力 |
| `crates/plugin-api` (`ve-plugin-api`) | OSS 公開プラグイン API ＋ Pure-Rust ゼロ `unsafe` WebAssembly 1.0 サンドボックス VM＆`plugins/*.wasm` ホットリローダー |
| `crates/render` (`ve-render`) | Linear sRGB（`Rgba16Float`）+ Premultiplied Alpha 合成、**マルチスレッド並列バッチ描画＆`wgpu` コンピュートシェーダーコンポジタ**、パーティクル＆自由ベジェベクターシェイプ＆3D ライト/影＆モーションブラー、`.cube` 3D LUT＆ビデオスコープ、ベジェマスク＆モーショントラッカー、PSD ネイティブ立ち絵、WGSL ホットリロード、**ゴールデン視覚回帰テストスイート** |
| `crates/ai` (`ve-ai`) | 自動文字起こし 3 モード（ローカル Whisper/VAD・Gemini・Claude）、Claude 字幕文脈補正、OS 資格情報ストア、課金確認ガード、TTS ファイルペア取込＆ローカル API |
| `crates/mcp` (`ve-mcp`) | 内蔵 MCP サーバー（ローカル Streamable HTTP `127.0.0.1` + 起動時ベアラートークン認証、マルチカム・プロNLE・納品検証/ZIP・エコシステム管理・式/テンプレート・テキスト編集・ビート・カラー・WASM 等 **54 ツール**） |
| `crates/app` (`ve-app`) | `eframe`（`wgpu`）+ `egui_dock` デスクトップアプリ（2 空間 UI・**v1.0 プロダクションスタジオ［マルチカム 2×2/3×3・トラックミキサー＆録音・納品検証/ZIP］**・**設定/キーバインド/公式 SHA-256 拡張レジストリ**・オートメーションカーブ描画＆プロトリム・式エディタ・テンプレートギャラリー・スコープ・RAM＋SSD キャッシュ・並列書き出し・レンダーキュー） |

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
