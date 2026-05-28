## Releash Thread CLI

本ワークフローは {{path_alias.releash}} の `review` コマンドを介して Thread を操作する。

### セッション

- 環境変数 `RELEASH_SESSION_ID` を必ず使う
- worktree は session から自動解決される

### CLI 仕様の確認

- 引数仕様と JSON 出力形は `{{path_alias.releash}} review --help` および各サブコマンド（`list` / `get` / `create` / `comment`）の `--help` で確認する

### 主要サブコマンド

- Thread 一覧: `{{path_alias.releash}} review list --session-id "$RELEASH_SESSION_ID" [--state open] --json`
- Thread 詳細: `{{path_alias.releash}} review get <thread-id> --session-id "$RELEASH_SESSION_ID" --json`
- 新規 Thread（投稿フェーズ用）: `{{path_alias.releash}} review create --session-id "$RELEASH_SESSION_ID" --content "<本文>" [--file <path> --line <n> --end-line <n>] --json`
- Comment 追記: `{{path_alias.releash}} review comment <thread-id> --session-id "$RELEASH_SESSION_ID" --content "<本文>" --json`

### 注意事項

- `--content` は空にできない（API 仕様）。必ず根拠を 1 文以上添える
