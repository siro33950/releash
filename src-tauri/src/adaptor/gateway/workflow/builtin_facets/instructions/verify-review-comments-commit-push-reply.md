# 役割

PR review comment 対応の最終段として、人間に差分・検証結果・返信予定を確認してもらい、approve 後に commit / push し、GitHub PR comment へまとめて返信する。

# 入力

- 環境変数 `RELEASH_SESSION_ID`
- 環境変数 `RELEASH_BASE_BRANCH`: PR 差分取得の基準 branch
- `[PR_REVIEW_COMMENT_IMPORTED]` が付いた Releash Thread
- `[FIX_POLICY_APPROVED]` Comment
- `[PR_REVIEW_REPLY_APPROVED]` Comment
- 現在の git 差分

# 手順

## 1. 対象 Thread と対応状況を取得する

`{{path_alias.releash}} review list --session-id "$RELEASH_SESSION_ID" --state open --json` で Open Thread を取得し、`[PR_REVIEW_COMMENT_IMPORTED]` が付いた Thread を対象にする。

各 Thread について `review get` / `review history` を確認し、次のいずれかを特定する。

- `[FIX_POLICY_APPROVED]`: 修正対象
- `[PR_REVIEW_REPLY_APPROVED]`: 修正しない対象

どちらもない Thread がある場合は、commit / push / reply を行わず、未決定 Thread として報告する。

## 2. 差分と検証結果を整理する

次を確認する。

```sh
git status --short
git diff --stat "$(git merge-base "$RELEASH_BASE_BRANCH" HEAD)"
git diff "$(git merge-base "$RELEASH_BASE_BRANCH" HEAD)"
```

必要なテスト・lint はプロジェクトの通常手順に従って実行する。実行できない場合は理由を明記する。

## 3. 返信予定を作成する

修正対象 Thread:

- push 後に「修正しました」と返信する
- 返信文には修正内容、確認したテスト、必要なら commit hash を含める

修正しない Thread:

- `[PR_REVIEW_REPLY_APPROVED]` の `reply` を返信する

返信には元 GitHub comment の `database_id` を使う。

## 4. 人間に approve を求める

次をまとめて提示し、approve を得る。

- commit 対象差分
- 実行したテスト・lint と結果
- GitHub PR comment への返信予定一覧
- Releash Thread の resolve 予定一覧

approve 前に commit / push / reply / resolve を行ってはならない。

## 5. approve 後に commit / push する

コード変更がある場合だけ commit / push する。

```sh
git add <変更ファイル>
git commit -m "<commit message>"
git push
```

コード変更がない場合は commit / push しない。

修正対象 Thread がある場合、push が成功するまで GitHub reply を投稿しない。

## 6. GitHub PR comment へ返信する

push 成功後、またはコード変更がないことを確認した後、各対象 comment に返信する。

```sh
gh api repos/{OWNER}/{NAME}/pulls/{PR_NUMBER}/comments \
  -f body="<返信本文>" \
  -F in_reply_to=<database_id>
```

## 7. Releash Thread を resolve する

GitHub reply が成功した Thread だけ resolve する。

```sh
{{path_alias.releash}} review resolve <thread-id> --session-id "$RELEASH_SESSION_ID" --outcome resolved --summary "<対応要約>" --json
```

# 出力

```markdown
## PR review comment 対応サマリ

### Commit / Push
- Commit: <hash またはなし>
- Push: <成功/未実施/失敗>

### GitHub replies
- 投稿成功: <件数>
- 投稿失敗: <件数>

### Releash Threads
- Resolve: <件数>
- 残 Open: <件数>
```

# 禁止事項

- approve 前に commit / push / GitHub reply / Thread resolve を行わない。
- push 前に修正対象 comment へ「修正済み」と返信しない。
- GitHub reply が失敗した Thread を resolve しない。
- `[PR_REVIEW_REPLY_APPROVED]` の reply 文を人間の承認なしに変更しない。
