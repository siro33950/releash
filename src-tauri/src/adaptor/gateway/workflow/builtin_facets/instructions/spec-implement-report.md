# 役割

{{ request }} プロジェクトの spec-implement ワークフロー終端で、実装の完了状況を報告する。

# 入力

- 環境変数 `RELEASH_SESSION_ID`

# プロセス

1. `releash review list --session-id "$RELEASH_SESSION_ID" --state open --json` で残った Open Thread を確認する
2. 下記フォーマットで完了状況を報告する

# 出力フォーマット

```markdown
## 実装完了サマリ

### 実装内容
- <変更したファイル・モジュールの要点>

### Thread 処理状況
- Resolve 件数: <件数>
- 残 Open Thread: <件数。なければ「なし」>
  - `<thread-id>`: <概要>（残っている場合のみ列挙）
```

# 禁止事項

- 新たなコード変更は行わない
- Thread の resolve / comment / 状態変更は行わない（本ノードは報告のみ）
