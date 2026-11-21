# Changelog

## [1.0.0] - 2026-11-21

長尺・大規模・プロ品質のプロダクション完成版マイルストーン v1.0（#17〜#20）をすべて実装しました（Pure Rust / ゼロ `unsafe` / ゼロ追加依存・全 **54 MCP ツール**）。

### #17 マルチカメラ編集 ＆ 音声相互相関による自動同期
* `ve-audio`: 2 段階正規化相互相関（500Hz RMS エンベロープ粗探索 → 8kHz 波形精密探索、`estimate_audio_offset_xcorr`）によるサブフレーム精度（0.125ms）の自動同期オフセット算出を実装
* `ve-core` / `ve-app` / `ve-mcp`: マルチカム自動整列（`build_multicam_sync_align_commands`）・2×2 / 3×3 同時プレビューグリッド＆数字キー `1`〜`9` ライブスイッチング記録・1 ステップ Undo 切替カット確定（`build_multicam_switch_cut_commands`）および MCP `sync_multicam` を追加

### #18 パフォーマンス革命：HW デコード ＆ 並列合成 ＆ 永続ディスクキャッシュ
* `ve-media`: FFmpeg ハードウェアデコード（`d3d11va` / `dxva2` / `vaapi` / `videotoolbox`）自動検出＆安全 Software フォールバック（`open_video_stream_with_hwaccel`）および解像度・コーデック可変プロキシ生成（`ProxyProfile` / `H264Fast` / `MjpegIntra`）を実装
* `ve-render`: マルチスレッド並列フレームバッチレンダリング（`render_frame_batch_parallel`）と `@compute @workgroup_size(8, 8)` `wgpu` コンピュートシェーダーコンポジタ（`WgpuComputeCompositor` / `composite_compute.wgsl`）を実装
* `ve-core` / `ve-app`: アプリ再起動後も再利用できる SSD 永続フレームキャッシュ（`PersistentFrameCache` / `cache/frames/` コンテンツハッシュ＋LRU 管理）を `RenderGraph` / `RamPreviewManager` へ統合し、バックグラウンド書き出しも並列化

### #19 ノンリニア編集プロ機能 ＆ 音声ポストプロダクション完成
* `ve-core` / `ve-app`: スリップ／ロール／スライドトリム（`SlipTrim` / `RollTrim` / `SlideTrim`）、クリップグルーピング＆A/V リンク解除（`Ctrl+G` / `Ctrl+Shift+G` / `SetGroupId`）、タイムライン上のボリューム（黄）・パン（シアン）オートメーションカーブ直接描画、トラックミキサー（`SetLayerMixer`：フェーダー/パン/ソロ/ミュート/バス/Aux リバーブセンド）、マイク直接アフレコ録音（`VoiceoverRecorder`）を実装
* `ve-audio`: Pure-Rust Hann 窓 STFT スペクトル減算ノイズリダクション（`noise_reduction`）、ダイナミック LPF ディエッサー（`de_esser`）、オフライン Mid/Side＋帯域ボーカル／BGM 分離（`separate_vocal_bgm_stems`）を追加
* `ve-core` / `ve-media` / `ve-mcp`: 書き出し前検証チェックリスト（`validate_project_delivery`）、Pure-Rust IEEE CRC-32 ZIP アーカイブパッケージ化（`archive_project_to_zip`）、Rec.709 SDR カラータグ＆HDR10 PQ 10-bit（BT.2020 / ST 2084 / `yuv420p10le`）書き出し、および MCP `nle_pro_edit` / `validate_and_package_project` を追加

### #20 v1.0 品質保証・カスタマイズ・配布エコシステム
* `ve-core` / `ve-app` / `ve-mcp`: カスタムキーバインド（AviUtl2 / Premiere Pro / DaVinci Resolve / After Effects プリセット＆JSON 入出力）、多言語 UI（日本語 / 英語 `tr`）、UI テーマ 4 種＆フォント倍率アクセシビリティ、FIPS 180-4 Pure-Rust SHA-256 署名検証＆サンドボックス権限明示付き公式拡張レジストリブラウザ、および MCP `manage_ecosystem` を追加（全 **54 MCP ツール**）
* 配布＆QA: Windows `winget` マニフェスト（`packaging/winget/0tsu-0w0.VideoEditingApp.yaml`）・macOS Universal `.app` Bundle・Linux AppDir（`packaging/linux/ve-app.desktop`）対応 `.github/workflows/release.yml`、およびゴールデン視覚回帰テストスイート（`crates/render/tests/visual_regression.rs`）を追加

## [0.9.0] - 2026-11-21

AE 級プロシージャル演出・テンプレート量産・高度な 2D/3D 合成を実現する v0.9 ロードマップ（#14〜#16）をすべて実装しました。

### #14 Pure-Rust エクスプレッションエンジン（式アニメーション）
* `ve-core`: ゼロ `unsafe`・決定論的サンドボックス式パーサ＆バイトコード VM（`expression` モジュール：`time`, `local_time`, `duration`, `progress`, `fps`, `frame`, `value`, `wiggle(freq, amp[, octaves])`, `noise`, `random`, `sin`/`cos`/`pow`/`clamp`/`linear`/`ease`, `loopOut("cycle"|"pingpong"|"offset"|"continue")`, `audio_band("low"|"mid"|"high"|"rms")`, `layer("名前").prop`）およびキーフレーム焼き込み `build_bake_expression_command`、`EditCommand::SetExpression` を実装
* `ve-render`: `RenderGraph` の全アイテム変形・トリムパス評価へエクスプレッションを統合（`audio_band` 遅延算出付き）
* `ve-app` / `ve-mcp`: インスペクタの式エディタ（リアルタイム評価値＆構文エラー表示・スニペット・Bake ボタン）、グラフエディタの式カーブ可視化、および MCP ツール `set_expression` を追加

### #15 モーショングラフィックス テンプレート（`.mogrt` 相当）＆ プロジェクトテンプレート
* `ve-core`: `MotionTemplate` / `TemplateBinding` / `ProjectTemplate`、イントロ・アウトロ尺を保護し中間のみ伸縮するレスポンシブ時間伸縮 `apply_responsive_time_stretch`、組み込みモーショングラフィックス 3 種（ニュース風ローワーサード / キネティック OP タイトル / 告知 CTA カード）＆プロジェクト雛形 3 種、`ProjectFolder` の `templates/`・`templates/projects/` 永続化 CRUD、`EditCommand::UpdateTemplateInstance` を実装
* `ve-app` / `ve-mcp`: テンプレートギャラリー、選択時に公開パラメータだけを表示する専用編集カード、現シーン保存・プロジェクト雛形新規作成 UI、および MCP ツール `manage_templates` を追加

### #16 高度な 2D/3D 合成：パーティクル・自由ベクターシェイプ・3D ライト＆モーションブラー
* `ve-core` / `ve-render`: 決定論的シード 2D/3D パーティクルエミッター `ItemKind::ParticleEmitter`（紙吹雪・雪・火花・光粒・集中線・スピードライン）、自由ベジェベクターシェイプ `ItemKind::VectorShape`（単色/線形グラデーション塗り・破線ストローク・トリムパスアニメーション）、3D カメラ空間のディレクショナル／ポイントライト＆ドロップシャドウキャッチャー（`Light3D`）、およびシャッター角度ベース サブフレーム合成モーションブラー（プレビュー軽量 / 書き出し高品質切替 `MotionBlurConfig`）を実装
* `ve-app` / `ve-mcp`: タイムライン「＋ パーティクル」「＋ ベクター」ボタン、専用インスペクタ、3D ライト＆モーションブラー UI、`add_object` 拡張および MCP ツール `set_scene_motion_blur` を追加（全 **50 MCP ツール**）

## [0.8.0] - 2026-11-21

爆速カット・音ハメ編集・プロ級カラーグレーディングを統合する v0.8 ロードマップ（#11〜#13）をすべて実装しました。

### #11 テキストベース編集（文字起こし連動リップルカット ＆ フィラー自動除去）
* `ve-core`: `text_edit` モジュールを追加（`extract_transcript_entries`・日本語/英語フィラー検出 `analyze_fillers`・語句掃除 `build_strip_filler_words_commands`・連動レイヤー一括リップルカット `build_text_based_delete_commands`・隣接区間スワップ並べ替え `build_text_based_reorder_commands`、すべて 1 ステップ Undo 対応）
* `ve-app`: `AudioAi3DPanel` に「9. テキストベース編集」パネルを追加（クリックシーク、複数選択一括カット、⬆/⬇ 区間入替、フィラー一括カット＆セリフ内語句掃除）
* `ve-mcp`: MCP ツール `text_based_edit` を追加

### #12 BGM ビート自動検出 ＆ ビートスナップ編集
* `ve-audio`: Pure-Rust オンセット包絡＋自己相関 BPM 推定＋位相アライメント（`detect_audio_beats` / `detect_beats_from_waveform`）を実装
* `ve-core`: ビート自動カット（`build_beat_cut_commands`）・最寄りビート整列（`build_snap_to_beats_commands`）・小節/全拍マーカー配置（`build_beat_markers_command`）を実装
* `ve-app` / `ve-mcp`: タイムライン「🥁 ビート検出」バー・ビートグリッド縦線描画（小節頭ゴールド / 拍シアン）・ドラッグ/トリムのビート自動吸着、および MCP ツール `detect_beats` を追加

### #13 `.cube` 3D LUT 対応 ＆ 3-Way カラーグレーディング ＆ リアルタイムビデオスコープ
* `ve-core`: 非破壊 `SceneColorGrade`（露出・コントラスト・彩度・色温度・ティント・3-Way Lift/Gamma/Gain・`.cube` LUT パス/強度）と `EditCommand::SetSceneColorGrade`、`ProjectFolder::luts_dir()` を追加
* `ve-render`: Adobe / DaVinci 標準 `.cube` 3D LUT パーサ＆四面体補間 `CubeLut3D::sample_tetrahedral`、組み込みシネマルック 3 種＋サンプル `.cube` 生成 `emit_sample_cube_luts`、`RenderGraph` 最終段への統合（プレビュー／RAM／書き出し／MCP 完全一致）、および高速ビデオスコープ解析 `compute_video_scopes`（Waveform / RGB Parade / Vectorscope / RGBY Histogram）を実装
* `ve-app` / `ve-mcp`: ドラッグ操作可能な 3-Way カラーホイール＆`.cube` ピッカー UI、プレビュー連動リアルタイム 4 分割ビデオスコープパネル、および MCP ツール `set_color_grade` を追加（全 47 MCP ツール）

## [0.7.1] - 2026-11-21

v0.3〜v0.4 の残課題（プリセット管理・専用インスペクタ・ショートカット・マーカー編集・UI 残骸整理・ドキュメント最新化）をすべて補完しました。

### 字幕スタイル＆エフェクトチェーン プリセット管理（`presets/*.json`）＋ 影色・座布団色 RGBA ピッカー
* `ve-core`: `TextStylePreset`（4 種の標準 YouTube/ゆっくり/シネマプリセット）と `EffectPreset::default_presets()`（3 種の標準エフェクトチェーン）を追加し、`ProjectFolder` に `presets/text_styles/*.json`・`presets/effects/*.json` の安全な保存・読込・削除（パストラバーサル防止）を実装
* `ve-app`: 字幕装飾インスペクタへ影色（`shadow_color`）・座布団色（`box_color`）の RGBA カラーピッカーおよび 1 クリックプリセット適用・保存・削除 UI を追加。映像エフェクト欄へエフェクトチェーンプリセット保存・適用・削除 UI を追加
* `ve-mcp`: MCP ツール `manage_presets`（`list` / `apply_text_style` / `apply_effect_preset` / `save_text_style` / `save_effect_preset`）を追加（全 44 ツール）

### フィルタオブジェクト・グループ制御・オーディオスペクトラム専用インスペクタ
* `ve-core`: Undo 対応コマンド `EditCommand::SetFilterObject` / `SetGroupControl` / `SetAudioSpectrum` を追加
* `ve-app`: `ItemKind::FilterObject`（組み込み＋プラグイン種別・適用レイヤー範囲・パラメータ）、`ItemKind::GroupControl`（対象レイヤー数）、`ItemKind::AudioSpectrum`（連動音声レイヤー・サイズ・バー本数・感度・基本色/ピーク色）の専用編集 UI をインスペクタへ統合

### タイムライン操作ショートカット補完 ＆ マーカー編集 UI
* `ve-core` / `ve-mcp`: `EditCommand::UpdateMarker` および MCP `manage_markers` の `update` アクション（ラベル・`color_hex` 更新）を追加
* `ve-app`: タイムラインへ `Ctrl+C`（コピー）、`Ctrl+V`（再生ヘッド位置へ相対オフセット維持で貼り付け）、`Ctrl+D`（複製）、`Ctrl+A`（全選択）、`Backspace`（削除 / `Shift+Backspace` リップル削除）、`Home`/`End`（先頭・末尾ジャンプ）を追加。ルーラー上のマーカークリックでラベル＆5 色パレットをその場で編集可能に拡張

### UI 残骸クリーンアップ ＆ ドキュメント最新化
* `ve-app`: 旧 Phase 1a / Phase 2 の仮文言（`"Phase 2 Hybrid Motion Project"`、`"日本語IME PoC 検証 (Phase 1a)"` および未使用 1 行テスト欄）を本番向けの簡潔な字幕エディタ＋折りたたみ IME 診断ログへ整理
* `README.md`: v0.7.1 現在の全機能・44 MCP ツール・WASM プラグイン・GitHub Releases ビルド情報へ最新化

## [0.7.0] - 2026-11-21

上級映像表現＆エコシステム拡張の v0.7 ロードマップ（#8〜#10）を全実装し、全 10 項目のロードマップを 100% 完了しました。

### #8 自由ベジェマスク ＆ 自動モーショントラッキング（顔モザイク・テロップ追従）
* `ve-core` / `ve-render`: 3次ベジェ閉曲線マスク `BezierMask`（楕円/矩形/ひし形プリセット・加算/減算・反転・2パス分離フェザー・`EditCommand::SetMasks` 1ステップ Undo 対応）とスキャンラインラスタライザ `apply_bezier_masks` を実装
* `ve-app`: プレビューキャンバス上でのベジェマスク輪郭描画および頂点・接線ハンドルの直接ドラッグ編集、インスペクタ編集 UI を追加
* `ve-render` / `ve-core`: Pure-Rust 1ポイント自動モーショントラッカー `track_point_in_scene`（粗→密テンプレートマッチング＋サブピクセル補間）と、結果を標準 `x`/`y` キーフレームへ焼き込む `build_motion_track_keyframes_command` を実装
* `ve-mcp`: MCP ツール `set_bezier_mask` を追加

### #9 WASM（WebAssembly）動的プラグインロード
* `ve-plugin-api`: Pure-Rust・ゼロ `unsafe`・ゼロ追加依存の WebAssembly 1.0 サンドボックス VM（`WasmModule` / `WasmInstance`：ホストインポート禁止・4MiB メモリ境界検証・命令フューエル上限保護）を実装
* `ve-plugin-api`: `plugins/*.wasm` の動的ロード・自動ホットリロード・破損時ロールバックを担う `WasmPluginManager` と、実行可能な実 `.wasm` サンプル 3 種生成（`emit_sample_wasm_plugins`）を実装
* `ve-app` / `ve-mcp`: 500ms ホットリロード監視、インスペクタ WASM プラグイン管理 UI、バックグラウンド書き出し時の自動ロード、MCP ツール `list_or_reload_wasm_plugins` を追加

### #10 AI ハイライト自動抽出 ＆ ショート動画自動切り抜き
* `ve-core`: 音声エネルギーピーク・発話/字幕密度・強調キーワード・マーカーを複合スコアリングして非重複（NMS）抽出する `extract_highlight_candidates` を実装
* `ve-core`: 抽出候補から **1080×1920（9:16）ショート動画シーン** を自動トリム（`source_offset`/キーフレーム補正）＆セーフゾーン 13 文字改行リレイアウトする `build_shorts_clip_scene_command`（`EditCommand::CreateScene` 1ステップ Undo）を実装
* `ve-app` / `ve-mcp`: `AudioAi3DPanel` のハイライト抽出＆1クリック 9:16 ショート生成 UI および MCP ツール `extract_highlights` を追加

## [0.6.0] - 2026-11-21

解説・実況・ショート動画の量産性を飛躍させる v0.6 ロードマップ（#4〜#7）をすべて実装しました。

### #4 BGM 自動ダッキング（サイドチェイン音量減衰）＆ オーディオリアクティブ
* `ve-audio`: 発話レイヤー（サイドチェイン）のエンベロープに応じて BGM レイヤーを滑らかに自動減衰する `apply_sidechain_ducking` を `AudioGraph` へ統合（プレビュー再生＆書き出し完全一致）
* `ve-audio`: Pure-Rust Hann 窓付き Goertzel 対数帯域スペクトラム解析 `analyze_spectrum_bands`（Low / Mid / High / RMS）を実装
* `ve-core` / `ve-render`: `ItemKind::AudioSpectrum`（音声連動リアルタイムスペクトラム描画）と `AudioReactiveConfig`（帯域エネルギー連動の拡大率・Yバウンス・不透明度アニメーション）、`EditCommand::SetAudioDucking` / `SetAudioReactive` を実装
* `ve-app` / `ve-mcp`: インスペクタの BGM ダッキング＆オーディオリアクティブ設定 UI、タイムライン「＋ スペクトラム」配置（Transform Gizmo 操作対応）、MCP ツール `set_audio_ducking` を追加

### #5 台本（Markdown / CSV）→ タイムライン一括自動生成（ゆっくり・解説動画量産モード）
* `ve-core`: `parse_script_csv` / `parse_script_markdown` / `parse_script_auto` と、話者別自動配色・日本語自動改行・座布団字幕・ポーズ間調整を 1 ステップ Undo で組み立てる `build_script_timeline_commands` を実装
* `ve-app` / `ve-mcp`: `AudioAi3DPanel` の台本一括生成 UI（`.csv` / `.md` 読込、背景画像同時配置、任意でローカル TTS 実尺連携）および MCP ツール `import_script` を追加

### #6 SRT / ASS 字幕入出力 ＆ 他ソフト互換（AviUtl `.exo` / OpenTimelineIO）
* `ve-core`: Pure-Rust の SRT / ASS (v4+) 字幕インポート＆エクスポート、AviUtl `.exo` パーサ（UTF-16LE 16進 `text=` 日本語復号＋座標・色・拡大率・透明度・回転対応）、および OpenTimelineIO `.otio` JSON エクスポートを実装
* `ve-app` / `ve-mcp`: `AudioAi3DPanel` の字幕・互換入出力 UI および MCP ツール `import_subtitles_or_exo` / `export_subtitles_or_otio` を追加

### #7 レンダーキュー（バッチ連続書き出し）
* `ve-app`: マルチジョブ連続書き出しマネージャー `RenderQueueState` を実装（MP4 H.264 / 9:16 縦動画自動リフレーム MP4 / H.265 / WebM Alpha / WAV / SRT / ASS / OTIO、1 クリック「YouTube 一括セット追加」、並び替え・削除、順次バックグラウンド書き出し＆進捗表示・キャンセル対応）

## [0.5.0] - 2026-11-21

プレビュー直感操作・動画デコード高速化＆RAM先読み・縦動画プリセット／OSネイティブファイル選択／自動保存を実装しました。

### #1 プレビュー画面上の直接操作（Transform Gizmo ＆ ガイド線）
* `PreviewCanvasState`: プレビュー上の直接クリック選択、バウンディングボックス・4隅スケールハンドル（`Shift` で比率維持）・回転ハンドル（`Shift` で 15° スナップ）・中央＆セーフマージン吸着ガイド線・3分割グリッド・9:16 Shorts/TikTok UI 被り警告エリア・ホイールズーム＆中ドラッグパンを実装（すべて `EditCommand::SetTransform` Coalescing 1ステップ Undo 対応）
* `ve-render`: `Transform2D::rotation_deg` の線形空間バイリニア回転描画と `render_project_scene_scaled`（シーン座標とプレビュー描画の 1:1 一致）を実装

### #2 動画デコード高速化 ＆ RAM プレビュー（バックグラウンド先読みキャッシュ）
* `ve-media` / `ve-render`: 常駐 FFmpeg パイプによる連続フレームデコーダ `VideoStreamDecoder` を `RenderGraph` へ統合し、毎フレームプロセス起動を排除
* `ve-app`: 専用ワーカースレッド先読みキャッシュ `RamPreviewManager`（最大 96 フレーム LRU・リビジョン更新時自動破棄）と、タイムラインルーラー下への AE 風緑色キャッシュ済みバー描画を実装

### #3 シーン設定 UI（1080p / 4K / 9:16 縦動画）・OS ネイティブファイル選択・自動保存
* `ve-core` / `ve-mcp`: `ScenePreset`（720p / 1080p / 4K / 9:16 Shorts / 1:1 Square）および `EditCommand::SetSceneSettings`（自動リフレーム＋Undo 対応）、MCP ツール `set_scene_settings` を追加
* `ve-app`: シーン設定モーダルと `WgpuPreviewPipeline` の解像度動的リサイズ、ゼロ依存 OS ネイティブファイル／フォルダ選択ダイアログ（Windows PowerShell WinForms / Linux `zenity`・`kdialog`）、および `backups/` ローテーション付き定期自動保存＆復元メニューを実装

## [0.4.0] - 2026-11-21

ロードマップ残バックログ（#3・#5・#6・#7・#8）をすべて実装完了しました。

### #3 無音区間自動カット（ジェットカット / Jump Cut）
* `ve-core` に RMS 発話区間検出（`detect_speech_intervals_from_rms`）および連動レイヤー一括カット＆左詰めコマンド生成（`build_jump_cut_commands`・1 ステップ Undo 対応）を追加
* タイムラインに「⚡ 無音カット」設定バー（解析レイヤー・RMS 閾値・最小無音長・語頭/語尾余白・連動レイヤー選択）を追加し、実メディア `AudioGraph` と `WaveformPeaks` の双方に対応
* 内蔵 MCP ツール `auto_jump_cut` を追加

### #5 カスタム WGSL シェーダー＆エフェクトの `wgpu` GPU オフロード
* `WgpuPreviewPipeline` に `Rgba16Float` Ping-Pong テクスチャ 2 枚と共通 48-byte `GpuEffectUniform`（`params[8]`, `resolution`, `time_sec`）を追加し、組み込み色調補正・2-Pass 分離ブラー・グロー・クリッピングおよびホットリロード済みカスタム WGSL パイプラインを GPU 実行
* ホットリロード成功時に `wgpu::RenderPipeline` を自動再構築し、検証エラー時は直前の正常パイプラインを保持
* オフスクリーン書き出し向け `HeadlessGpuOffloader` を実装（GPU 非搭載環境では CPU `RenderGraph` へ自動フォールバック）

### #6 Claude 字幕文脈補正 UI（差分確認）＆ MCP ワークフロー強化
* `AudioAi3DPanel` に「✨ Claude で字幕を文脈補正」（`CloudUploadGuard` 付き）およびオフライン「📝 自動改行プレビュー」と Before / After 差分確認・個別編集・1 ステップ Undo 一括適用 UI を追加
* インスペクタに「📋 MCPクライアント設定JSONをコピー」ボタンを追加
* 内蔵 MCP に `auto_jump_cut`, `refine_subtitles`, `import_media`, `manage_markers`, `start_export`（確認ガード＆出力名サニタイズ付き）を追加

### #7 キーボードショートカット完全化＆タイムライン操作性向上
* `Space`（再生/一時停止）、`←`/`→`（`Shift` で 10 フレーム移動）、`J`/`K`/`L` シャトル再生（`-4x/-2x/-1x/0/1x/2x/4x`、`AudioMasterClock` ロックフリー原子レート対応）
* `Ctrl + マウスホイール` による再生ヘッド中心のタイムラインズーム
* `M` タイムラインマーカー追加（ラベル・色付き、ルーラー旗クリックでジャンプ／右クリック削除、YouTube チャプターテキスト一括コピー）
* 空トラックドラッグによるラバーバンド矩形選択、複数選択クリップ同時ドラッグ移動、リップル削除（`Shift+Delete`）

### #8 GitHub Releases 自動配布パイプライン
* `.github/workflows/release.yml` を追加し、`v*` タグプッシュ時に Windows 11 x64（`.zip`）および Ubuntu Linux x64（`.tar.gz`）の最適化リリースバイナリと SHA-256 チェックサムを自動公開

## [0.3.0] - 2026-11-21

* AviUtl2 本体実用機能（A-1〜A-6: 名前付きカラーパレット、インライン制御文字＆補完、レイヤー常駐設定、シーンチェンジ、再生速度・逆再生・ループ、実務頻出フィルタ）
* 人気プラグイン相当機能（B-1〜B-7: 整列・隙間詰め・ミニマップ、フォント視覚化ピッカー、音声 DSP＆LUFS 正規化、エイリアス棚、テキスト検索・一括置換、かんしくん風フォルダ監視、演出プリセット）
* 字幕リッチ装飾（縁取り・ドロップシャドウ・座布団背景）

## [0.2.0] - 2026-11-21

* コードレビュー最適化・セキュリティ強化（PR #5）
* 実メディア動画・画像・PSD の本格描画＆軽量プロキシ自動生成＆バックグラウンド非同期書き出し（PR #6）

## [0.1.0] - 2026-11-21

仕様書 v1.1（[docs/spec.md](docs/spec.md)）の全ロードマップ（Phase 1a 〜 Phase 4）を実装しました。

### Phase 1a — コアエンジン PoC
* 有理数タイムベース `RationalTime`（705,600,000 ticks/sec）および 1フォルダ管理（`project.json` 原子的保存・`history.jsonl`・アセット参照＆プロジェクト収集）
* FFmpeg / FFprobe 外部 CLI サイドカーによるプローブ・デコード・書き出しと `symphonia` 音声デコード
* プレビュー／書き出し共通 `AudioGraph`・`AudioMasterClock`（A/V同期）・Windows `cpal` 出力
* Linear sRGB（`Rgba16Float`）+ Premultiplied Alpha 合成、`cosmic-text`+`swash` 日本語テキスト描画、`wgpu` プレビューパイプライン、`egui` 日本語 IME PoC パネル

### Phase 1b — 実用 NLE 土台と AI 接続
* `CommandBus`（GUI/MCP 共通実行・Undo/Redo・Command Coalescing・GUI ドラッグ排他ロック・`history.jsonl` クラッシュ復旧）
* AviUtl2 風インタラクティブタイムライン（ドラッグ移動・右端トリム・スナップ・分割・複数選択・音声波形描画）およびオフラインアセット再リンク UI
* 内蔵 MCP サーバー（ローカル Streamable HTTP `127.0.0.1` + 起動時ベアラートークン認証、`render_frame_preview`・`batch_execute` 等）
* GitHub Actions CI（Ubuntu `fmt`/`clippy`/テスト + Windows 11 x64 ビルド＆テスト）

### Phase 2 — AE 融合とワンクリック結合
* AviUtl2 中間点／AE キーフレーム統一モデルと 3次ベジェ曲線（値グラフ／速度グラフ＆ドラッグ可能なベジェハンドル）
* 2空間 UI 切替（メイン編集空間／モーション合成空間）および Smart Precompose（ワンクリック結合・ダブルクリック非破壊再編集・コンテンツハッシュ＆LRU キャッシュ管理）
* グループ制御・トラックマット（Alpha/Luma＋反転）・ブレンドモード・文字ごとアニメーター・基本 WGSL エフェクトチェーン（ブラー／グロー／色調補正／クリッピング）・透過書き出し（WebM VP9+Alpha / PNG 連番）

### Phase 3 — 音声自動化・立ち絵・3D
* 自動文字起こし3モード（ローカル Whisper/VAD・Gemini API・Claude API 文脈補正）、OS 資格情報ストア（Windows Credential Manager 等）、送信前コスト＆尺確認ガード
* 合成音声（TTS）ファイルペア自動取込（`.wav`+`.txt`+`.lab`）および VOICEVOX / COEIROINK / A.I.VOICE ローカル API 連携
* PNG 差分立ち絵の自動口パク（音量 RMS 連動／母音音素連動・設定レス音声レイヤー選択）
* 3D カメラ制御（位置・注視点・FOV・被写界深度ボケ・3D 空間配置・Z ソート合成）

### Phase 4 — 拡張性・OSS 化
* PSD ネイティブ立ち絵対応（`psd` による非表示差分レイヤー含む直接合成・自動目パチ・口パク連動）
* カスタム WGSL シェーダーのホットリロード（`naga` 静的検証＋エラー時フォールバック保護）
* 公開プラグイン API クレート `ve-plugin-api`（`PluginManifest` / `EffectPlugin` / `TimelineGeneratorPlugin` / `PluginRegistry` + リファレンス実装ビネット＆セピアプラグイン）
* MIT / Apache-2.0 デュアルライセンス、アーキテクチャ設計書、プラグイン開発ガイド、E2E 統合テスト基盤
