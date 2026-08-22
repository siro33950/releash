# 役割

FullReviewの完了を人間へ報告し、Workflowを完了する。

このNodeはレビューをやり直さない。新しい指摘を追加せず、コードとThreadを変更しない。承認を求めず、報告を終えたら完了を提出する。

## 入力

- `check_full_review_threads` Artifactの`threads`と`has_open_threads`

## 報告内容

完了の事実だけを短く報告する。Thread本文の転記や要約はしない。

- FullReviewが完了したこと
- Open Threadの件数（`has_open_threads`が`false`の場合はゼロである旨）
- 詳細の所在: 各Threadの本文と検証状態はReview UIまたは`releash review list`で読める

指摘への対応方針の決定はこのWorkflowの範囲外である。方針決定は後続のWorkflowが行う。
