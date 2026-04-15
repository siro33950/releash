# Releash

Tauri + React + Monaco Editor のデスクトップGitエディタ。
モバイル/タブレットからWebSocket経由でリモートアクセスする機能を持つ。

## アーキテクチャ

### ロジック配置の原則
- **全てのロジックはRustに実装し、フロントエンドはインターフェースに徹する（例外なし）**
- フロントエンドの責務: 表示、入力受付、Tauriコマンド呼び出し、表示用フォーマット
- 新機能のロジックは必ずTauriコマンドとして実装し、フロントからは `invoke` で呼ぶ

### デスクトップアプリ（メイン）
- **フロントエンド**: React 19 + TypeScript + TailwindCSS 4 + Monaco Editor
- **バックエンド**: Rust (Tauri 2) + git2 + tokio
- **ビルド**: Vite + Biome (lint/format)

### リモートアクセス（モバイル対応）
- `src/remote/` に独立したReactアプリ（`RemoteApp`）
- デスクトップ側がWebSocketサーバー（`ws_server/`）を起動
- モバイルブラウザからQRコード + HMACトークンで接続
- `vite.config.remote.ts` で別途ビルド → `src-tauri/resources/remote/` に配置
- WebSocket経由でGit操作・ターミナル・diff閲覧・コメント・ソース管理が可能

### 通信レイヤー
- `src-tauri/src/protocol/` — WebSocketメッセージの型定義（auth, git, pty, comment, agent等）
- `src-tauri/src/ws_bridge.rs` — ブロードキャスター（PTY出力バッファリング含む）
- `src-tauri/src/ws_server/` — セッション管理、認証、ルーティング、レート制限

## ディレクトリ構造

```text
src/                        # フロントエンド
├── components/panels/      # EditorTabContent, FileTree, TerminalPanel, SourceControlPanel 等
├── components/layout/      # ActivityBar, StatusBar
├── components/workspace/   # Worktree管理UI
├── components/ui/          # shadcn/ui プリミティブ
├── hooks/                  # カスタムフック（useFileContents, useGitStatus 等）
├── lib/                    # ユーティリティ（computeHunks, generatePatch 等）
├── contexts/               # EditorContext
├── screens/                # WorktreeView, WorkspaceManagerScreen
├── remote/                 # モバイル用リモートアプリ（独立エントリーポイント）
│   ├── components/         # RemoteDashboard, RemoteTerminalPanel 等
│   └── hooks/              # useWebSocket, useRemoteGitActions 等
└── types/                  # 型定義

src-tauri/src/              # バックエンド
├── git/                    # Git操作（branch, commit, status, diff, stage, worktree, log）
├── ws_server/              # WebSocketサーバー（handlers, session, auth, routing）
├── protocol/               # 通信プロトコル型定義
├── git_host/               # GitHub連携（PR取得等）
├── pty.rs                  # 疑似端末管理
├── config.rs               # アプリ設定（TOML）
├── webhook.rs              # Slack/Discord Webhook通知
├── watcher.rs              # ファイル変更監視
└── shell_integration.rs    # Bash/Zsh/Fish シェル統合
```

## ビルド・テスト・Lint

CIと同じコマンドを使うこと（`.github/workflows/ci.yml` 参照）。

### フロントエンド（プロジェクトルートで実行）
```bash
pnpm lint          # Biome check（失敗時は pnpm lint:fix で修正）
pnpm test          # Vitest
pnpm build         # TSC + Vite build（メイン + リモート）
```

### Rust（src-tauri/ ディレクトリで実行）
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

### 統合テスト
```bash
pnpm test:integration   # Playwright
```

## テスト方針

### 配置
- フロントエンド: ソースファイルと同じディレクトリに `*.test.tsx` / `*.test.ts`
- Rust: 各モジュール内に `#[cfg(test)] mod tests { ... }`

### 何をテストするか
- 新規ロジックには必ずテストを書く
- ユーティリティ関数（`lib/`）: 入出力の網羅テスト、境界値テスト
- カスタムフック（`hooks/`）: 状態遷移と副作用のテスト
- コンポーネント: ユーザーインタラクションと表示条件のテスト
- Rustコマンド: 正常系・エラー系の両方

### モックの方針
- `react-resizable-panels`: jsdomで動作しないため `vi.mock` 必須
- Tauri API（`@tauri-apps/api`）: `vi.mock` でスタブ化
- Monaco Editor: 命令型APIのため統合テストではなくロジックのみ単体テスト
- 外部プロセス（`git push` 等）: テストでは実行しない

## コーディング規約

### フロントエンド
- インデント: Tab（Biome設定）
- インポート整理: Biomeの自動整理に任せる
- UIコンポーネント: shadcn/ui (Radix UI) ベース
- スタイル: TailwindCSS

### Rust
- `git2` クレートでGit操作。pushのみ `std::process::Command` で `git push`
- 非同期処理: tokio
- エラー型: 各モジュールに専用エラー型

## 既知の制約・注意点

- Monaco Editorのファイル切り替え: `key={filePath}` で再マウントすること（中間状態でフリーズする）
- `git2` の UnbornBranch: `repo.head()` が `ErrorCode::UnbornBranch` → 初回コミット前の特別処理が必要
- `git apply --cached`: パッチのベースはStagedにすること（HEADベースだとコンテキスト行不一致）
- worktreeをリポジトリルート内に作るとBiomeがネスト設定エラーを起こす → worktreeは外部に配置
- `ignore::WalkBuilder` はテストで `.git` ディレクトリが必要 → `git2::Repository::init()` で初期化
