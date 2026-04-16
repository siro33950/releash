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

- `beforeEach`では`vi.clearAllMocks()`ではなく`vi.resetAllMocks()`を使用する（clearAllMocksは`mockResolvedValueOnce`のキューと`mockImplementation`をクリアしない）
- 各テストの冒頭でデフォルトmock値を明示的に再設定する
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

## Claude Agent SDK統合
- SDK（@anthropic-ai/claude-code）の型定義（.d.ts）と実装（cli.js）は乖離することがある。SDK関連のコード変更時は型定義を鵜呑みにせず、実際のSDK動作をE2Eテストまたはnamed pipe経由で検証する
- SDKのZodスキーマでoptionalと思われるフィールドがrequiredな場合がある。新しいSDKメッセージ形式を使う際はminified cli.jsのパターン検索で実動作を確認する
- resume失敗時にSDKがthrowせず`result{errors}`をyieldするケースがある。エラーハンドリングはthrow以外の経路も考慮する

## Rust↔JS間の規約
- タイムスタンプはフロントエンドに渡す前にミリ秒に変換する（`as_secs_f64() * 1000.0`）。Rust内部では秒単位、JS側ではミリ秒単位で統一

## パス管理
- ファイルパスは保存レイヤーで`strip_prefix`により相対パスに正規化する。比較レイヤーでも相対パスを使う二重防御を適用する
- macOSの`/var`→`/private/var`シンボリックリンクに注意。パス比較時はcanonicalizeを検討する

## Tauri権限管理
- 新しいTauriコマンドやファイル操作を追加したら`capabilities/default.json`の権限を必ず確認・追加する。`readFile`と`readTextFile`は別権限（`fs:allow-read-file` vs `fs:allow-read-text-file`）
- CSPの`style-src`/`script-src`にCDN URLを追加する場合、dev時は動くがbuild時に壊れるパターンに注意

## サブプロセス管理
- 外部ツール（Claude Agent SDK等）をサブプロセスとして起動する場合、ネスト検出用の環境変数マーカー（`CLAUDECODE`, `CLAUDE_CODE_ENTRYPOINT`等）を`env_remove()`で明示的に除去する
- GUIアプリからのサブプロセスはシェル環境変数を継承しない。`fix-path-env`クレートでPATHを補完し、`TERM`/`COLORTERM`/`LANG`を明示的に設定する

## LSPプロセス管理
- LSPセッションにはCancellationTokenを導入し、全バックグラウンドタスク（stdout reader, stderr logger, monitor_exit）にキャンセル監視を追加する
- shutdown失敗時は必ずkillフォールバックを実行する。Worktree削除時には`kill_lsp_by_worktree`を呼ぶ
- diagnostics_cacheとpending_requestsはshutdown/kill時にクリアする
- メモリ問題の調査時は`ps aux`/`pstree`でプロセスツリー全体を確認し、自アプリ外（PTY内のClaude Code等）からのLSP起動も疑う

