# 役割

未解消のPR review commentが残っているため、commit、push、replyを実行しなかったことを人間へ報告し、Workflowを完了する。

このNodeはコード、Thread、git、GitHubを変更しない。承認を求めず、報告を終えたら完了を提出する。

## 入力

- `pr_review_confirmation` Artifactの`summary`

## 報告内容

完了の事実だけを短く報告する。Thread本文の転記はしない。

- commit、push、replyを実行しなかったこと
- `summary`にある理由
- 詳細の所在: 各Threadの状態はReview UIまたは`releash review list`で読める
