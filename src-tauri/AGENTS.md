# AGENTS.md - src-tauri (Releash バックエンド)

このディレクトリは Releash の Rust バックエンド（Tauri 2）。

## アーキテクチャ

クリーンアーキテクチャに基づくレイヤー構成を採用する。詳細は [`../docs/architecture/`](../docs/architecture/) を参照：

- [README.md](../docs/architecture/README.md) — 全体像、レイヤー構成、ドメイン一覧、移行計画
- [DOMAIN.md](../docs/architecture/DOMAIN.md) — ドメイン層規約
- [USECASE.md](../docs/architecture/USECASE.md) — ユースケース層規約
- [GATEWAY.md](../docs/architecture/GATEWAY.md) — ゲートウェイ層規約
- [CONTROLLER.md](../docs/architecture/CONTROLLER.md) — コントローラ層規約（Tauriコマンド／WebSocketハンドラ）
- [TEST.md](../docs/architecture/TEST.md) — テスト規約

### レイヤーの責務

```
infrastructure ← adaptor/gateway → domain ← usecase ← adaptor/controller
```

| レイヤー | 役割 |
|---|---|
| `domain/` | ビジネスロジック。外部依存を持たない |
| `usecase/` | アプリケーションの業務手順。ドメインのみに依存 |
| `adaptor/controller/` | Tauri コマンド（`command/`）と WebSocket ハンドラ（`handler/`） |
| `adaptor/gateway/` | Repository / Gateway trait の具体実装。外部世界の都合と内側の言語を相互に変換する |
| `adaptor/presenter/` | レスポンス整形 |
| `adaptor/protocol/` | WebSocket メッセージ・DTO |
| `infrastructure/` | 外部世界の都合をその形のまま扱う。変換せず、内側の層を知らない |
| `other/` | エラー型・ログ等の横断的関心事 |

### ロジック配置の原則

[../.claude/rules/rust-first-logic.md](../.claude/rules/rust-first-logic.md) の通り、全てのロジックは Rust に置く。フロントエンドはインターフェースに徹する。

## ビルド・テスト・Lint

```bash
# プロジェクトルートで実行（フロントエンド含む）
pnpm lint
pnpm test
pnpm build

# src-tauri/ で実行（Rust）
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

CI と同じコマンドを使う。詳細は [`../.github/workflows/ci.yml`](../.github/workflows/ci.yml) と root の [`../CLAUDE.md`](../CLAUDE.md) を参照。

## 既知の制約

- `git2` の `UnbornBranch`: `repo.head()` が `ErrorCode::UnbornBranch` を返す場合の特別処理が必要
- `git apply --cached`: パッチのベースはステージング状態にする（HEAD ベースだとコンテキスト不一致）
- worktree はリポジトリルート内に作らない（Biome のネスト設定エラー）
- `ignore::WalkBuilder` テストは `.git` ディレクトリが必要 → `git2::Repository::init()` で初期化

## リファクタリング進行中

このコードベースはクリーンアーキテクチャへの段階移行中。新規実装・修正時は新規約に従う。既存コードを触る場合、規約に従って移行する余裕があれば移行する（スコープは現タスクに閉じる）。

進行状況・移行計画は GitHub Issue を参照。
