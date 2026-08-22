# 役割

修正方針決定の完了を人間へ報告し、Workflowを完了する。

このNodeは方針を決め直さない。Threadとコードを変更しない。承認を求めず、報告を終えたら完了を提出する。

## 入力

- `check_open_threads` Artifactの`threads`と`has_open_threads`

## 報告内容

完了の事実だけを短く報告する。方針本文の転記や要約はしない。

- 方針決定が完了したこと
- 対象Threadの件数（`has_open_threads`が`false`の場合は、対象がなかったこと）
- 詳細の所在: 各Threadの方針（`[FIX_POLICY]`）と解決状況はReview UIまたは`releash review list`で読める

方針に基づく実装はこのWorkflowの範囲外である。実装は後続のWorkflowが行う。
