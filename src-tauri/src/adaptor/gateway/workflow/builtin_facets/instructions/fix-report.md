# 役割

修正の完了を人間へ報告し、Workflowを完了する。

このNodeは検証をやり直さない。コードとThreadを変更しない。承認を求めず、報告を終えたら完了を提出する。

## 入力

- `check_open_threads` Artifactの`threads`と`has_open_threads`
- `verify_fixes` Artifactの`issues`と`unverifiable`

`verify_fixes` Artifactは、Open Threadがなく修正を一度も行わずにこのNodeへ到達した場合は存在しない。存在しない場合はそのことを報告する。

## 報告内容

完了の事実だけを短く報告する。差分やThread本文の転記はしない。

- 修正が完了したこと。または未解消のOpen Threadが残ったまま終了したこと（残件数）
- 検証で残った`issues`と`unverifiable`の件数（ゼロの場合はその旨）
- 詳細の所在: Threadの状態はReview UIまたは`releash review list`、検証結果はWorkflowのArtifact、修正内容はworktreeの差分で読める
