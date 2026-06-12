# 役割

現在のブランチに紐づく GitHub PR の unresolved review comment を取得し、各 comment を Releash Thread として投稿する。

この Step は取得と Thread 投稿だけを行う。対応方針の決定、実装、GitHub への返信は行わない。

# 入力

- 環境変数 `RELEASH_SESSION_ID`
- 現在の git ブランチ
- GitHub CLI `gh`

# 手順

## 1. PR を特定する

```sh
branch="$(git branch --show-current)"
gh pr list --head "$branch" --state open --json number,title --jq '.[0]'
gh repo view --json owner,name --jq '"\(.owner.login)/\(.name)"'
```

現在のブランチに紐づく open PR が見つからない場合は、その旨を報告して終了する。

## 2. unresolved review comment を取得する

GraphQL API で `reviewThreads` を取得する。REST API では `isResolved` を取得できないため、必ず GraphQL を使う。

取得する項目:

- review thread ID
- `isResolved`
- `isOutdated`
- `path`
- `line`
- `startLine`
- `diffSide`
- 最初の comment の `databaseId`
- comment body
- author
- createdAt
- diffHunk
- thread 内の返信

対象は `isResolved == false` の thread のみ。

## 3. 既存 Releash Thread との重複確認

`{{path_alias.releash}} review list --session-id "$RELEASH_SESSION_ID" --json` で既存 Thread を取得し、本文・履歴に同じ `database_id` または `github_review_thread_id` を持つ `[PR_REVIEW_COMMENT_IMPORTED]` がある場合は新規投稿しない。

## 4. Releash Thread 投稿

未取り込みの GitHub review comment ごとに、`{{path_alias.releash}} review create` で Thread を作成する。

Thread 本文は次の形式にする。

```text
[PR_REVIEW_COMMENT_IMPORTED]
github_review_thread_id: <GitHub review thread id>
database_id: <最初の comment databaseId>
pr_number: <PR number>
pr_title: <PR title>
path: <path>
line: <line>
start_line: <startLine or null>
diff_side: <diffSide>
is_outdated: <true|false>
author: <author>
created_at: <createdAt>

PR review comment:
<body>

Thread replies:
<既存返信。なければ `なし`>

Diff hunk:
<diffHunk>
```

可能なら `--file <path> --line <line>` を付ける。行番号がない場合は本文だけで投稿する。

# 出力

次を簡潔に報告する。

- PR 番号とタイトル
- unresolved review comment 件数
- 新規作成した Releash Thread 件数
- 既に取り込み済みとしてスキップした件数
- 作成した Thread ID と元 GitHub comment の対応

# 禁止事項

- 対応方針を決めない。
- `[FIX_POLICY_APPROVED]` / `[PR_REVIEW_REPLY_APPROVED]` を投稿しない。
- GitHub PR comment に返信しない。
- コード変更、commit、push を行わない。
