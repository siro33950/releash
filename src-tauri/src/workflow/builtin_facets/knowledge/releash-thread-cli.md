## Releash Thread CLI

本ワークフローは {{path_alias.releash}} の `review` コマンドを介して Thread と Stance を操作する。

### セッション

- 環境変数 `RELEASH_SESSION_ID` を必ず使う
- worktree は session から自動解決される

### CLI 仕様の確認

- 引数仕様と JSON 出力形は `{{path_alias.releash}} review --help` および各サブコマンド（`list` / `get` / `create` / `comment`）の `--help` で確認する

### 主要サブコマンド

- Thread 一覧: `{{path_alias.releash}} review list --session-id "$RELEASH_SESSION_ID" [--state open] [--stance none] --json`
- Thread 詳細: `{{path_alias.releash}} review get <thread-id> --session-id "$RELEASH_SESSION_ID" --json`
- 新規 Thread + Stance 表明（投稿フェーズ用）: `{{path_alias.releash}} review create --session-id "$RELEASH_SESSION_ID" --content "<本文>" --stance agree [--file <path> --line <n> --end-line <n>] --json`
- Comment 追記（任意で Stance 更新）: `{{path_alias.releash}} review comment <thread-id> --session-id "$RELEASH_SESSION_ID" --content "<本文>" [--stance <agree|disagree>] --json`

### 注意事項

- `--content` は空にできない（API 仕様）。Stance のみの表明は不可で、必ず根拠を 1 文以上添える
- `review create` に `--stance agree` を付けると Thread 作成と Stance 表明を atomic に確定できる
- `review comment` に `--stance` を付けると Comment 追記と Stance 表明を atomic に行える
