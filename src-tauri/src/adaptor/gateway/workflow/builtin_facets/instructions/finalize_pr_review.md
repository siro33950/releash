# 役割

`pr_review_confirmation` Artifactで人間が確認した内容だけを実行し、commit、push、GitHub reply、Releash Thread Resolveを完了する。

## 入力

- `pr_review_confirmation` Artifact
- 現在のgit状態
- 対象PRとrepository情報

## 事前確認

1. `ready`が`true`であることを確認する。
2. 各replyのThread ID、database ID、返信文、outcome、summaryが欠落していないことを確認する。

入力が人間の確認内容と一致しない場合は、何も実行せず報告する。

## CommitとPush

`commit_required: true`の場合だけ、現在の作業ツリーの変更をstageし、`commit_message`をそのまま使ってcommitする。

commit成功後にpushする。commitまたはpushが失敗した場合はGitHubへreplyせず、ThreadもResolveしない。

`commit_required: false`の場合はcommitとpushを行わない。

## GitHub reply

Artifactの`replies`を順番に処理し、各`database_id`へ`reply`を一字一句変更せず投稿する。

```sh
gh api repos/{OWNER}/{NAME}/pulls/{PR_NUMBER}/comments \
  -f body="<reply>" \
  -F in_reply_to=<database_id>
```

## Releash Thread Resolve

GitHub replyが成功した項目だけ、Artifactの`thread_outcome`と`thread_summary`を使ってResolveする。

GitHub replyが失敗したThreadはOpenのまま残す。

## 出力

- commit hashまたは未実施理由
- push結果
- GitHub replyの成功・失敗件数
- ResolveしたThread ID
- Openのまま残したThread IDと理由

## 禁止事項

- commit message、reply、Thread outcome、summaryを変更しない。
- push成功前に修正済みreplyを投稿しない。
- reply失敗時にThreadをResolveしない。
