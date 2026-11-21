# 今後の機能追加ロードマップ（Future Features Backlog）

> 更新日: 2026-11-20  
> Phase 1〜4・最適化/セキュリティ強化（PR #5）・実メディア描画＆非同期書き出し（PR #6）・v0.3（AviUtl2 本体実用機能 A-1〜A-6 ＋ 人気プラグイン相当機能 B-1〜B-7 ＋ #4 字幕リッチ装飾、PR #7）に続き、**v0.4 で残バックログ（#3・#5・#6・#7・#8）をすべて実装完了しました。**

---

## 3. ~~無音区間自動カット（ジェットカット / Jump Cut）~~ ✅ 実装済み（v0.4）

- `ve-core`: `JumpCutConfig`, `SpeechInterval`, `detect_speech_intervals_from_rms`, `build_jump_cut_commands`（音声レイヤー＋連動する映像・字幕・立ち絵レイヤーを一括カット＆左詰め、1回の `Ctrl+Z` で復元可能な `EditCommand::Batch` 生成）。
- `ve-app`: タイムラインツールバーに「⚡ 無音カット」展開パネルを実装（解析対象レイヤー・RMS閾値・最小無音長・語頭/語尾マージン・連動リップルレイヤー指定、`AudioGraph` 実波形および `WaveformPeaks` フォールバック対応）。
- `ve-mcp`: 内蔵 MCP ツール `auto_jump_cut` を追加。

---

## 4. ~~字幕リッチ装飾（縁取り・ドロップシャドウ・座布団背景）＆ プリセット管理~~ ✅ 実装済み（v0.3）

- `TextStyle`（縁取り・ドロップシャドウ・角丸座布団背景）、AviUtl2 インライン制御文字（`<#RRGGBB>`, `<@パレット名>`, `<s64>`, `<fFont>`, `<b1>`, `</>`）、名前付きカラーパレット、および `aliases/*.json` エイリアス棚として実装完了。

---

## 5. ~~カスタム WGSL シェーダー＆エフェクトの `wgpu` GPU オフロード~~ ✅ 実装済み（v0.4）

- `ve-render`: `WgpuPreviewPipeline` に `Rgba16Float` の Ping-Pong テクスチャ 2 枚と共通 48-byte `GpuEffectUniform`（`params[8]`, `resolution`, `time_sec`、先頭 16 バイトが既存 WGSL と完全互換）を追加。
- `ShaderHotReloader`: `naga` によるフラグメントエントリポイント自動抽出し、ホットリロード時に `wgpu::RenderPipeline` を自動再構築（構文エラー時は直前の正常パイプラインを保持）。
- `HeadlessGpuOffloader`: オフスクリーン書き出し時のヘッドレス `wgpu::Device` GPU ポストプロセス＆リードバックを実装（GPU 非搭載環境では CPU `RenderGraph` へシームレスに自動フォールバック）。
- `ve-app`: インスペクタに GPU オフロード有効化・アクティブ WGSL 選択・強度/色調/2-Passブラー/グロー調整 UI を実装し、プレビューとバックグラウンド書き出しの両方へ接続。

---

## 6. ~~Claude 字幕文脈補正 UI（差分確認）＆ MCP ワークフロー強化~~ ✅ 実装済み（v0.4）

- `ve-app` (`AudioAi3DPanel`): 「✨ Claude で字幕を文脈補正」（`CloudUploadGuard` 確認ガード付き）およびオフライン「📝 自動改行プレビュー」を追加。Before / After 差分テーブルで各行を個別編集・チェック選択でき、`EditCommand::Batch` による 1 ステップ Undo で一括適用可能。
- `ve-app` (`Inspector`): 「📋 MCPクライアント設定JSONをコピー」ボタンを追加（`EmbeddedMcpServer::client_config_json()` で現在の `127.0.0.1:<port>/mcp` と Bearer トークンを整形コピー）。
- `ve-mcp`: 新ツール `auto_jump_cut`, `refine_subtitles`（オフライン日本語改行／Claude文脈補正対応）, `import_media`, `manage_markers`（追加・削除・一覧・YouTubeチャプター出力）, `start_export`（`user_confirmed: true` 必須確認ガード＋出力ファイル名サニタイズ付き）を実装。

---

## 7. ~~キーボードショートカット完全化＆タイムライン操作性向上~~ ✅ 実装済み（v0.4）

- `Space`: 再生 / 一時停止トグル（シャトル速度 1x リセット）
- `←` / `→`（`Shift` で 10 フレーム）: フレーム単位ステップ移動
- `J` / `K` / `L`: 逆再生・停止・倍速シャトル再生（`-4x / -2x / -1x / 0 / 1x / 2x / 4x`、`AudioMasterClock` ロックフリー原子レート対応）
- `Ctrl + マウスホイール`: 再生ヘッド中心のタイムラインズーム（スクロールオフセット自動補正）
- `M`: ラベル・色付きタイムラインマーカー追加（ルーラー上の旗クリックでジャンプ／右クリックで削除、ミニマップ表示、「📋 YouTubeチャプター」一括コピー対応）
- 空トラックドラッグによる複数クリップ矩形選択（ラバーバンド選択）、複数選択クリップ同時ドラッグ移動（1 ステップ Undo 結合）、およびリップル削除（`Shift+Delete` / ツールバーボタン）。

---

## 8. ~~GitHub Releases 自動配布パイプライン（Windows 11 / Linux）~~ ✅ 実装済み（v0.4）

- `.github/workflows/release.yml` を追加（`push: tags: ['v*']` および手動 `workflow_dispatch` で起動）。
- `windows-latest` で `ve-app-windows-x64.zip` + `.sha256`、`ubuntu-latest` で `ve-app-linux-x64.tar.gz` + `.sha256` を自動パッケージングし、`SHA256SUMS.txt` とともに GitHub Release へ自動公開。
