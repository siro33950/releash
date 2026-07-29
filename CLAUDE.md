# Releash

Releash は、ソフトウェア開発のための **programmable agentic workflow workbench**。

プロダクトの中心は workflow である。開発者が agentic workflow を定義し、実行し、観測し、承認し、却下し、再指示し、その判断に必要な作業状態を同じ場所で扱えることを目指す。

Releash は、特定の作業単位や特定の道具を主語にしない。コード、差分、terminal、テスト出力、review comment、agent session、workflow run、approval、実行履歴などを、workflow が扱う artifact として統合する。

## プロダクト方針

- Releash は programmable agentic workflow の workbench であり、小さな IDE クローンではない。
- 第一級の状態は workflow state である。
- human checkpoint を第一級に扱う。観測、承認、却下、指示修正、再開を自然にできるようにする。
- artifact は workflow の入力・出力・判断材料であり、プロダクトの主語ではない。
- remote / mobile を扱う場合は、workflow 判断点への監督・介入を中心にする。
- 実装は、実際の workflow action が軽くなる・信頼できるようになる薄い縦切りを優先する。

## アーキテクチャ原則

### Rust がロジックを所有する

- **全てのアプリケーションロジックは Rust に置く。例外なし。**
- フロントエンドの責務は、表示、入力受付、backend command 呼び出し、最小限の表示用フォーマットに限る。
- 新しい振る舞いは Rust の usecase / query service / Tauri command の背後に実装し、frontend からは `invoke` で呼ぶ。
- workflow、session、artifact、terminal、review、persistence のロジックを React hook や UI component に追加しない。
- 触った frontend code に既存ロジックが残っている場合は、タスクのスコープ内で可能な範囲で Rust 境界へ移す。

### 状態の所有者を明確にする

- workflow runtime、workflow artifact、agent session state、review comment、terminal state、persistence の所有者を明確にする。
- full-retention 設計を避ける。summary、page、id-based operation、delta で足りる場合に、session body、artifact、stream、workflow state 全体を clone / store / recompute / resend しない。
- read model は、Tauri、WebSocket、将来の daemon / native client が同じ backend-owned state を読める形にする。
- frontend state は UI に必要な状態の mirror に留め、domain behavior の source of truth にしない。

### デスクトップアプリ

- **フロントエンド**: React 19 + TypeScript + TailwindCSS 4 + Monaco Editor
- **バックエンド**: Rust (Tauri 2) + tokio
- **ビルド**: Vite + Biome

### 通信レイヤー

- `src-tauri/src/protocol/` - WebSocket message type / DTO
- `src-tauri/src/ws_bridge.rs` - broadcaster / sync bridge
- `src-tauri/src/ws_server/` - session、auth、routing、handler、rate limit

## ディレクトリ構造

```text
src/                        # フロントエンド
├── components/panels/      # EditorTabContent, FileTree, TerminalPanel, SourceControlPanel 等
├── components/layout/      # ActivityBar, StatusBar
├── components/workspace/   # Worktree 管理 UI
├── components/ui/          # shadcn/ui primitive
├── hooks/                  # React hook。interface-oriented に保つ
├── lib/                    # frontend utility。domain logic を追加しない
├── contexts/               # EditorContext と UI context
├── screens/                # WorktreeView, WorkspaceManagerScreen
└── types/                  # frontend-facing type

src-tauri/src/              # バックエンド
├── domain/                 # domain logic
├── usecase/                # application workflow / usecase
├── adaptor/                # controller, gateway, presenter, protocol adapter
├── infrastructure/         # 外部世界の都合をその形のまま扱う（変換しない）
├── protocol/               # WebSocket protocol type
├── ws_server/              # WebSocket server
├── ws_bridge.rs            # WebSocket / app bridge
├── pty.rs                  # PTY management
├── config.rs               # app config
├── webhook.rs              # Slack / Discord webhook notification
├── watcher.rs              # file watching
└── shell_integration.rs    # Bash / Zsh / Fish shell integration
```

バックエンドは clean architecture へ段階移行中。Rust layer の詳細な規約は `src-tauri/AGENTS.md` と `docs/architecture/` を参照。

## ビルド・テスト・Lint

CI と同じコマンドを使う。`.github/workflows/ci.yml` を参照。

### フロントエンド

プロジェクトルートで実行:

```bash
pnpm lint
pnpm test
pnpm build
```

format / import order で lint が落ちた場合:

```bash
pnpm lint:fix
```

### Rust

`src-tauri/` で実行:

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

### 統合テスト

プロジェクトルートで実行:

```bash
pnpm test:integration
```

## テスト方針

### 配置

- フロントエンドテストは対象ファイルの隣に `*.test.tsx` / `*.test.ts` として置く。
- Rust テストは該当 module 内に `#[cfg(test)] mod tests { ... }` として置く。

### カバレッジ期待値

- 新規ロジックにはテストを書く。
- frontend utility は入出力と edge case をテストする。
- hook は状態遷移と副作用をテストする。
- component は user interaction と conditional rendering をテストする。
- Rust command / usecase は正常系とエラー系をテストする。

### モック方針

- `react-resizable-panels` は jsdom で動作しないため `vi.mock` する。
- `@tauri-apps/api` の Tauri API は `vi.mock` で stub する。
- Monaco Editor は imperative API を持つため、Monaco 自体の integration test より周辺ロジックの unit test を優先する。
- 外部プロセスはテストで実行しない。

## コーディング規約

### フロントエンド

- インデントは tab。Biome に従う。
- import 整理は Biome に任せる。
- UI component は shadcn/ui と Radix UI をベースにする。
- styling は TailwindCSS。
- React component は interface-oriented に保つ。domain decision を hook、reducer、view helper に埋め込まない。

### Rust

- async 処理は tokio を使う。
- module ごとに専用 error type を使う。
- domain code は infrastructure dependency を持たない。
- controller は Tauri / WebSocket input を usecase call へ変換する。business behavior を controller に持たせない。

## 既知の制約

- Monaco Editor のファイル切り替えは `key={filePath}` で remount する。中間状態で freeze することがある。
- worktree は repository root 内に作らない。Biome が nested config で失敗することがある。

## レビュー観点

Releash を変更するときは、次を確認する。

- workflow action の定義、実行、観測、承認、却下、再指示のどれかが軽くなるか。
- 新しいロジックは frontend ではなく Rust に置かれているか。
- 変更した state の source of truth は明確か。
- ドメインの規則（判断・計算・分類・検証・遷移）を domain が所有しているか。状態を持つ概念は集約が、持たない概念は値オブジェクトとドメインサービスが表現しているか。同じ概念が二つの場所で表現されていないか。domain の型と規則は実行経路にあるか（`docs/architecture/DOMAIN.md` モデルが実行を担う）。
- full-retention / full-recompute 経路を増やしていないか。
- 同じ backend-owned state を Tauri、WebSocket、将来の client surface で再利用できるか。
- artifact が workflow の判断材料として扱われているか。
