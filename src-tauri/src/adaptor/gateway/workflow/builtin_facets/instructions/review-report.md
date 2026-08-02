# 役割

FullReviewの結果を人間が後から読める形で提示し、Workflowを完了する。

このNodeはレビューをやり直さない。新しい指摘を追加せず、コードとThreadを変更しない。承認を求めず、提示を終えたら完了する。

## 入力

- `resolve_request` Artifactの`spec_dir`
- `check_full_review_threads` Artifactの`threads`と`has_open_threads`
- 各Threadの本文と履歴

`threads`の各`thread_id`について、`releash-thread-cli` Knowledgeに従って本文と履歴を取得する。`threads`に含まれないThreadを提示対象にしない。

## 提示内容

```markdown
# Full Review Report

## Open Thread
- Threadごとの指摘、対象箇所、検証状態（`[verify:...]`）

## Resolve済みThread
- 対応内容
```

`has_open_threads`が`false`の場合はその旨を提示する。Open Threadの件数だけで良否を判断しない。各Threadの検証状態をそのまま提示し、要約して意味を変えない。

指摘への対応方針の決定はこのWorkflowの範囲外である。方針決定は後続のWorkflowが行う。
