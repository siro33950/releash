## Releash Thread CLI

本ワークフローは releash の `review` コマンドを介して Thread を操作する。

### セッション

- 環境変数 `RELEASH_SESSION_ID` を必ず使う
- worktree は session から自動解決される

### CLI 仕様の確認

- 引数仕様と JSON 出力形は `releash review --help` および各サブコマンド（`list` / `get` / `create` / `comment` / `resolve` / `history`）の `--help` で確認する

### 主要サブコマンド

- Thread 一覧: `releash review list --session-id "$RELEASH_SESSION_ID" [--state open] --json`
- Thread 詳細: `releash review get <thread-id> --session-id "$RELEASH_SESSION_ID" --json`
- Thread 履歴: `releash review history <thread-id> --session-id "$RELEASH_SESSION_ID" --json`
- 新規 Thread（投稿フェーズ用）: `releash review create --session-id "$RELEASH_SESSION_ID" --content "<本文>" [--file <path> --line <n> --end-line <n>] --json`
- Comment 追記: `releash review comment <thread-id> --session-id "$RELEASH_SESSION_ID" --content "<本文>" --json`
- Thread resolve: `releash review resolve <thread-id> --session-id "$RELEASH_SESSION_ID" --outcome <outcome> --summary "<要約>" --json`

### 注意事項

- `--content` は空にできない（API 仕様）。必ず根拠を 1 文以上添える
- `review resolve` の `--outcome` は解決状況を表す自由文（例: `resolved`, `wontfix`, `duplicate`）、`--summary` は対応内容の要約を 1 文以上で記載する
