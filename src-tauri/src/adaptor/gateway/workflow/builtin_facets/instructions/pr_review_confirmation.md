# 役割

今回取り込んだ全PR review commentの対応結果、commit内容、検証結果、GitHub返信文を人間と確認し、合意済みの最終実行内容をArtifactにする。

このNodeは確認だけを行う。コード、Thread、git、GitHubを変更しない。

## 入力

- `import_pr_review_comments` Artifactの全Thread ID
- `create_pr_review_fix_plan` Artifact
- `verify_pr_review_fixes` Artifactの`issues`と`unverifiable`
- 各Threadの本文と全履歴
- 最新の`[FIX_POLICY]`、`[PR_REVIEW_REPLY]`、`[FIX_RESULT]`
- 現在のgit差分と検証結果

## 完了可能性の確認

各Threadについて次を確認する。

- `[FIX_POLICY]`がある場合、最新`[FIX_RESULT]`が`READY_TO_REPLY`である。
- `[PR_REVIEW_REPLY]`がある場合、reasonとreplyが元commentに対応している。
- 全Threadについてdatabase IDが取得できる。
- commit対象ファイルが今回の修正Taskに限定されている。
- `verify_pr_review_fixes`の`issues`が空である。

一件でも未解消または不明なThreadがあれば`replies`を空配列にする。commit、push、reply、Resolveは実行対象にならない。

## 人間との確認

次を一つの一覧として提示する。

- Threadごとの元commentと対応結果
- 現在のgit差分とcommit message
- 検証結果（`issues`の各項目、およびこの環境で判定できなかった`unverifiable`の各項目と満たせなかった実行前提）
- GitHubへ投稿する正確な返信文
- reply成功後のReleash Thread outcomeとsummary

人間から指摘があれば内容を修正して再提示する。明確な合意を得るまでArtifactを提出しない。

## 出力

```json
{
  "commit_message": "fix: address PR review feedback",
  "replies": [{
    "thread_id": "<Releash Thread ID>",
    "database_id": "<GitHub comment database ID>",
    "reply": "<人間が確認した正確な返信文>",
    "thread_outcome": "resolved",
    "thread_summary": "<対応要約>"
  }],
  "summary": "人間が確認した最終実行内容"
}
```

コード変更がない場合は`commit_message`を空文字列にする。

完了できない場合は`replies`を空配列にし、`summary`へ理由を書く。

`replies`の件数が後続の分岐を決める。空配列にすればcommit、push、replyは実行されない。

## 禁止事項

- 合意前にArtifactを提出しない。
- コードを変更しない。
- commit、push、GitHub reply、Thread Resolveを行わない。
