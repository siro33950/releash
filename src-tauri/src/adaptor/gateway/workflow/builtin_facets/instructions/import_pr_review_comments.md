# 役割

現在のブランチに紐づくGitHub PRの未解決review commentを取得し、Releash Threadへ取り込む。

このNodeは取得、重複確認、Thread作成、対象Thread一覧の確定だけを行う。対応方針の決定、コード変更、GitHubへの返信は行わない。

## 入力

- 環境変数`RELEASH_SESSION_ID`
- 現在のgitブランチ
- GitHub CLI `gh`

## 手順

1. `git branch --show-current`で現在のブランチを取得する。
2. `gh pr list --head <branch> --state open`で対象PRを一意に特定する。
3. `gh repo view`でownerとrepository名を取得する。
4. GraphQL APIでPRの`reviewThreads`を取得する。REST APIで代替しない。
5. `isResolved == false`のreview threadだけを対象にする。
6. Releash Threadの本文と履歴を確認し、同じ`github_review_thread_id`または`database_id`を持つcommentを重複登録しない。
7. 未登録のreview threadを一件ずつReleash Threadとして作成する。
8. 新規作成したThreadと、既に取り込み済みでOpenのThreadを合わせて今回の対象一覧にする。

取得する情報:

- GitHub review thread ID
- 最初のcommentのdatabase ID
- PR番号とタイトル
- path、line、startLine、diffSide
- isOutdated
- comment本文、author、createdAt
- thread内の返信
- diffHunk

## Thread本文

```text
[PR_REVIEW_COMMENT_IMPORTED]
github_review_thread_id: <GitHub review thread id>
database_id: <最初のcomment databaseId>
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
<既存返信。なければ「なし」>

Diff hunk:
<diffHunk>
```

可能なら`review create`へfileとlineを指定する。行番号がない場合は本文だけで作成する。

## 出力

対象PRが存在し、今回扱うOpen Threadがある場合:

```json
{
  "threads": [{"thread_id": "<Releash Thread ID>"}],
  "has_open_threads": true,
  "summary": "PR番号、取得件数、新規作成件数、重複スキップ件数"
}
```

対象PRがない、または対象となるOpen Threadがない場合:

```json
{
  "threads": [],
  "has_open_threads": false,
  "summary": "対象がない理由"
}
```

## 禁止事項

- 対応方針を決めない。
- コード、Spec、GitHub上の状態を変更しない。
- GitHubへreplyしない。
- Releash ThreadをResolveしない。
- commit、pushを行わない。
