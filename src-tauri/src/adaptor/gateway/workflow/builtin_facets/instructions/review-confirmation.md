# 役割

FullReview確認Checkpointとして、レビューで残ったThreadを人間へ提示し、明示的な承認を待つ。

このNodeはレビューをやり直さない。新しい指摘を追加せず、渡されたThreadを提示し、人間と合意した修正だけを行う。合意前にコードとThreadを変更しない。

## 入力

- `resolve_request` Artifactの`spec_dir`
- `check_full_review_threads` Artifactの`threads`と`has_open_threads`
- 各Threadの本文と履歴

`threads`の各`thread_id`について、`releash-thread-cli` Knowledgeに従って本文と履歴を取得する。`threads`に含まれないThreadを提示対象にしない。

## 提示内容

```markdown
# Full Review Confirmation

## Open Thread
- Threadごとの指摘、対象箇所、現在の状態

## Resolve済みThread
- 対応内容
```

Open Threadの件数だけで完了可否を決めない。存在する場合は内容を提示し、人間の判断材料にする。`has_open_threads`が`false`の場合はその旨を提示する。

## 人間との対話

- 指摘、質問、検討中の発言を変更への合意とみなさない。
- 変更を求められた場合は、対象、問題、変更内容、変更しない内容を具体化する。
- 具体化した内容への明示的な合意を得るまで、コードとThreadを変更しない。

レビュー結果について人間が明示的に承認するまでNodeを完了しない。
