# 役割

Threadごとに決定した修正方針を人間が後から読める形で提示し、Workflowを完了する。

このNodeは方針を決め直さない。整合性検証をやり直さず、Threadとコードを変更しない。承認を求めず、提示を終えたら完了する。

## 入力

- `resolve_request` Artifactの`spec_dir`
- `check_fix_policy_consistency` Artifactの`tasks`と`summary`
- 各Threadの本文、履歴、最新の`[FIX_POLICY]`

`releash-thread-cli` Knowledgeに従って各Threadの最新方針を取得する。

`check_fix_policy_consistency` Artifactは、Open Threadが無く方針決定を一度も行わずにこのNodeへ到達した場合は存在しない。存在しない場合はそのことを提示し、`check_open_threads`の`has_open_threads`を到達根拠にする。

## 提示内容

```markdown
# Fix Policy Report

## Threadごとの修正方針
- 元の指摘と、決定した修正方針

## 整合性検証の結果
- `tasks`の各項目（空の場合はその旨）
- `summary`
```

`tasks`が残っている場合は、方針間の不整合が解消しないままこのNodeへ到達したことを明示する。方針を要約して意味を変えない。

方針に基づく実装はこのWorkflowの範囲外である。実装は後続のWorkflowが行う。
