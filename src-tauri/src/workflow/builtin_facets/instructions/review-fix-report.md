# 役割

{{project_name}} のフルレビュー後修正ワークフロー終端で、各 Open Thread の対応状況を人間に提示し、approve された Thread を resolve する。

# 入力

- 環境変数 `RELEASH_SESSION_ID`
- 各 Open Thread の `[FIX_POLICY_APPROVED]` Comment

# プロセス

## 1. Open Thread の取得

- `{{path_alias.releash}} review list --session-id "$RELEASH_SESSION_ID" --state open --json` で Open Thread を取得
- 各 Thread の `[FIX_POLICY_APPROVED]` を `{{path_alias.releash}} review get <thread-id>` / `{{path_alias.releash}} review history <thread-id>` で確認

## 2. 対応状況の提示

各 Thread について次の形式で人間に提示する。

```text
Thread <thread-id> [<観点>] <file>:<line-range>
修正方針: <[FIX_POLICY_APPROVED] の修正方針>
受入条件: <[FIX_POLICY_APPROVED] の受入条件>
実装対応: <該当 Thread に対する実コードの変更要約>
resolve 案: resolved
```

## 3. 一括 approve の確認

人間に次のいずれかを求める。

- 一括 approve: 全 Thread を resolve する
- Thread 単位の reject: 該当 Thread だけ resolve しない（後続の判断は人間に委ねる）
- 修正指示: 提示内容に問題がある場合は、再提示する

## 4. resolve の実行

approve された Thread だけ、次のコマンドで resolve する。

```sh
{{path_alias.releash}} review resolve <thread-id> --session-id "$RELEASH_SESSION_ID" --outcome resolved --summary "<実装対応の要約>" --json
```

reject された Thread は resolve せず、Open のまま残す。

## 5. 完了報告

```markdown
## レビュー後修正 完了サマリ

### 対応 Thread
- Resolve 件数: <件数>
- Reject 件数: <件数>
- 残 Open Thread: <件数。なければ「なし」>
  - `<thread-id>`: <概要>（残っている場合のみ列挙）

### 実装内容（要約）
- <変更したファイル・モジュールの要点>
```

# 禁止事項

- 新たなコード変更は行わない
- approve されていない Thread を resolve しない
- `[FIX_POLICY_APPROVED]` の方針自体を変更しない
