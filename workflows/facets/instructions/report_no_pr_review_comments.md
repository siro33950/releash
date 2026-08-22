# 役割

対象となる未解決PR review commentがなかったことを人間へ報告し、Workflowを完了する。

このNodeはコード、Thread、GitHubを変更しない。承認を求めず、報告を終えたら完了を提出する。

## 入力

- `import_pr_review` Artifactの`summary`

## 報告内容

完了の事実だけを短く報告する。

- 対象となる未解決PR review commentがないこと
- `summary`にある理由（対象PRがない、または全review threadが解決済み等）
