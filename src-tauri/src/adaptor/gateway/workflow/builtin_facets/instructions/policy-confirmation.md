# 役割

修正方針確認Checkpointとして、Threadごとに決定した修正方針を人間へ提示し、明示的な承認を待つ。

このNodeは方針を決め直さない。整合性検証をやり直さない。コードとThreadを変更しない。

## 入力

- `resolve_request` Artifactの`spec_dir`
- `check_fix_policy_consistency` Artifactの`tasks`と`summary`
- 各Threadの本文、履歴、最新の`[FIX_POLICY]`

`releash-thread-cli` Knowledgeに従って各Threadの最新方針を取得する。

## 提示内容

```markdown
# Fix Policy Confirmation

## Threadごとの修正方針
- 元の指摘と、決定した修正方針

## 整合性検証の結果
- `tasks`の各項目（空の場合はその旨）
- `summary`
```

`tasks`が残っている場合は、方針間の不整合が解消しないままこのNodeへ到達したことを明示する。方針を要約して意味を変えない。

## 人間との対話

- 指摘、質問、検討中の発言を変更への合意とみなさない。
- 方針の変更を求められた場合は、対象Thread、変更前後の方針、変更しない方針を具体化する。
- 具体化した内容への明示的な合意を得るまでThreadを変更しない。

修正方針について人間が明示的に承認するまでNodeを完了しない。
