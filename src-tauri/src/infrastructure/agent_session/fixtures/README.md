# Agent session wire fixtures

このディレクトリは、Claude の stream-json と Codex app-server の JSON-RPC を、1 行 1 メッセージの JSONL fixture として保持する。各 fixture ディレクトリには `wire.jsonl`、convert 層の `convert.golden`、projector 後の `read_model.golden` を併置する。

## 採取

リポジトリ外の一時ディレクトリを指定して、対象 backend の実セッションを実行する。

```bash
RELEASH_WIRE_RECORD=/tmp/releash-wire pnpm tauri:dev
```

受信した生行は `/tmp/releash-wire/claude.jsonl` または `/tmp/releash-wire/codex.jsonl` へ追記される。環境変数を設定しない通常実行ではファイル I/O は発生しない。対象セッションまたはアプリを通常終了し、終了時の queue drain / flush が完了してから採取ログをコピーする。tap は生データをマスクしないため、採取先をリポジトリ内に置いたり、そのまま commit したりしないこと。

## マスキング

採取ログから対象 turn だけを新しい fixture の `wire.jsonl` へコピーする前に、次を安定したプレースホルダへ置換する。

- ホーム、worktree、リポジトリなどの絶対パス: `<WORKTREE>`、`<HOME>`
- prompt、応答、thinking、tool output などの本文: `<USER_MESSAGE>`、`<ASSISTANT_TEXT>`、`<THINKING>`、`<TOOL_RESULT>`
- token、API key、認証ヘッダー、URL query、環境変数のsecret: `<REDACTED>`
- session、turn、item、tool、permissionなどのID: `<SESSION_ID>`、`<TURN_ID>`、`<TOOL_USE_ID>`
- コマンドやファイル名に個人情報が含まれる場合: `<COMMAND>`、`<FILE_PATH>`

`type`、`subtype`、`method`、フィールド構造、メッセージ順序は変えない。JSONL全体を検索し、実名、メールアドレス、ホスト名、絶対パス、token形式、実メッセージ本文が残っていないことをcommit前に確認する。

## golden の更新

fixtureを追加または意図的に変えた後、`src-tauri` で次を実行する。

```bash
UPDATE_GOLDEN=1 cargo test infrastructure::agent_session::fixtures
UPDATE_GOLDEN=1 cargo test test_support::agent_session_wire_replay
cargo test infrastructure::agent_session::fixtures
cargo test test_support::agent_session_wire_replay
```

更新された両goldenをレビューし、convertの各行出力、Claudeの`auto_responses`、最終read modelの差分が意図どおりであることを確認する。通常の`cargo test`はgoldenを更新せず、不一致時に最初の相違行を報告して失敗する。
