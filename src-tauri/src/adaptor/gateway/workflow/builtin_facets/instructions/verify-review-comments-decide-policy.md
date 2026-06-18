# 役割

`[PR_REVIEW_COMMENT_IMPORTED]` が付いた Releash Thread を人間と確認し、各 PR review comment の対応方針を確定する。

この Step は方針決定だけを行う。Task 作成、実装、GitHub への返信、commit、push は行わない。

# 入力

- 環境変数 `RELEASH_SESSION_ID`
- `[PR_REVIEW_COMMENT_IMPORTED]` が付いた Open Thread

# 完了条件

対象 Thread がすべて次のどちらかの状態になっていること。

- 修正する: `[FIX_POLICY_APPROVED]` Comment があり、Thread は Open のまま
- 修正しない: `[PR_REVIEW_REPLY_APPROVED]` Comment があり、Thread は Open のまま

GitHub への reply は最後の `commit_push_and_reply` Step でまとめて行うため、この Step では投稿しない。

# 手順

## 1. 対象 Thread を取得する

```sh
{{path_alias.releash}} review list --session-id "$RELEASH_SESSION_ID" --state open --json
{{path_alias.releash}} review get <thread-id> --session-id "$RELEASH_SESSION_ID" --json
{{path_alias.releash}} review history <thread-id> --session-id "$RELEASH_SESSION_ID" --json
```

対象は `[PR_REVIEW_COMMENT_IMPORTED]` があり、まだ `[FIX_POLICY_APPROVED]` も `[PR_REVIEW_REPLY_APPROVED]` もない Thread。

## 2. 1件ずつ人間と確認する

対象 Thread を1件ずつ提示し、以下を整理する。

- 元 PR review comment
- 対象ファイルと行
- 現在のコード
- 現在の差分
- 既存返信があればその内容
- 修正するべきか、修正しないで返信するべきか

複数 Thread をまとめて approve させない。1件の方針が確定してから次に進む。

## 3. 修正する場合

人間が修正方針を approve したら、Thread に次の Comment を投稿する。

```text
[FIX_POLICY_APPROVED]
修正方針: <何をどう直すか>
受入条件:
- <元 PR comment に対して満たすべき条件>
対応しない範囲: <この Thread では扱わないこと。なければ `なし`>
source:
  github_review_thread_id: <GitHub review thread id>
  database_id: <最初の comment databaseId>
  path: <path>
  line: <line>
```

Thread は Open のまま残す。

## 4. 修正しない場合

人間が修正しない判断と reply 文を approve したら、Thread に次の Comment を投稿する。

```text
[PR_REVIEW_REPLY_APPROVED]
classification: RESOLVED | NOT_VALID | NEEDS_CLARIFICATION
reason: <修正しない理由>
reply: <GitHub PR comment に最後に投稿する返信文>
source:
  github_review_thread_id: <GitHub review thread id>
  database_id: <最初の comment databaseId>
  path: <path>
  line: <line>
```

Thread は Open のまま残す。GitHub reply はまだ投稿しない。

# 出力

最後に次を報告する。

- 修正対象件数
- 修正しない件数
- 未決定 Thread がないこと
- GitHub reply は未投稿であること

# 禁止事項

- Task を作らない。
- 実装しない。
- Thread を resolve しない。
- GitHub PR comment に返信しない。
- commit / push しない。
- 未承認の方針を `[FIX_POLICY_APPROVED]` または `[PR_REVIEW_REPLY_APPROVED]` として投稿しない。
