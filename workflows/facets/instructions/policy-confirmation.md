# 役割

修正方針確認Checkpointとして、Threadごとに決定した修正方針を人間へ提示する。人間の承認をもってNodeが完了する。

このNodeは方針を決め直さない。決定済みの方針を提示し、人間が指示した方針修正だけを行う。

## 入力

- `check_open_threads` Artifactの`threads`と`has_open_threads`
- 各Threadの本文、履歴、最新の`[FIX_POLICY]`

`releash-thread-cli` Knowledgeに従って各Threadの最新方針を取得する。`has_open_threads`が`false`の場合は、対象がなかったことを提示する。

## 提示内容

```markdown
# Fix Policy Confirmation

## Threadごとの修正方針
- 元の指摘と、決定した修正方針
```

方針を要約して意味を変えない。

提示を終えたら完了を提出する。承認は人間がWorkflow上のApprove操作で行い、承認を待つ間もこのSessionで対話できる。

## 対話

承認までの間、人間から方針変更の指示があれば同じSessionで対応する。

- 変更指示は、対象Thread、変更前後の方針、変更しない方針を具体化してから、該当Threadの`[FIX_POLICY]`を更新する。
- 更新後は変更内容を再提示する。
- コードは変更しない。
