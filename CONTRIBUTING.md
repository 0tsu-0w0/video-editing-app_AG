# Contributing to `video-editing-app_AG`

プロジェクトへのコントリビューションを歓迎します！バグ報告、機能提案、プラグイン／カスタム WGSL シェーダーの提供、ドキュメント改善まで幅広く受け付けています。

## 開発環境セットアップ

### 必須ツール
* **Rust:** 1.95 以上（`rustup` 経由でのインストールを推奨）
* **FFmpeg / FFprobe:** PATH に通っていること（MIT/Apache-2.0 互換性を維持するため外部 CLI サイドカー方式を採用）
* **Linux のみ:** `pkg-config`, `libssl-dev`, `libfontconfig1-dev`, `libxkbcommon-dev`, `libwayland-dev`, `libgtk-3-dev`

### ローカル検証コマンド（CI と同一）
PR を作成する前に以下がすべて警告ゼロで通過することを確認してください。

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## 設計原則（厳守事項）
1. **Command Bus 一元化:** プロジェクト状態を変更する GUI 操作・MCP 操作はすべて `ve_core::CommandBus` を経由させ、`Ctrl+Z` による Undo/Redo と `history.jsonl` クラッシュ復旧を保証する。
2. **ライセンス互換性:** 本リポジトリは **MIT OR Apache-2.0** です。GPL 依存クレートを直接リンクせず、FFmpeg 等は外部プロセスまたは LGPL 動的リンクで分離する。
3. **シークレット非保存:** 外部 API キー（Gemini / Claude 等）は必ず OS 資格情報ストア（`ve_ai::OsCredentialStore`）に保管し、`project.json` には一切出力しない。
4. **Linear sRGB + Premultiplied Alpha:** `ve-render` 内の合成・エフェクト・プラグイン処理はすべて Linear sRGB かつ Premultiplied Alpha で統一する。

## Pull Request フロー
1. `main` から機能ブランチ（例: `feat/my-effect` または `fix/timeline-snap`）を作成
2. 変更内容とユニット／統合テストを追加
3. 上記のローカル検証を実行し、PR を作成（GitHub Actions で Ubuntu + Windows 11 CI が自動実行されます）

## ライセンス同意
本リポジトリへ提出されたコントリビューションは、特段の明示がない限り `MIT OR Apache-2.0` のデュアルライセンスの下で公開されることに同意したものとみなされます。
