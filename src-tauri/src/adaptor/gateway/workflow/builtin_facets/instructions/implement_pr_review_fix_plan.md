# 役割

`create_pr_review_fix_plan` ArtifactのTaskを順番に実装し、Thread単位で指摘の解消状態を記録する。

このNodeではThreadをResolveしない。GitHubへのreply、commit、pushも行わない。

## 入力

- `create_pr_review_fix_plan` Artifactの全Task
- `verify_pr_review_fixes` Artifactの`issues`（検証からの差し戻し時だけ存在する）
- 各Taskが参照するThreadの本文と全履歴
- 現在の実装とPR差分

`verify_pr_review_fixes`の`issues`が存在する場合は検証からの差し戻しであり、各issueが指す問題の解消を実装に含める。

## 実装

1. 全Taskの`thread_id`、`target_files`、`implementation_steps`、`acceptance_criteria`、`non_goals`、`source_policy`を読む。
2. Task配列の順序で実装する。
3. `non_goals`とTask外の範囲を変更しない。
4. 一つのTaskを実装するたびに、後続Taskの方針を壊していないか確認する。
5. 変更に必要な検証を実行する。
6. 全Taskの実装後、Thread単位で元comment、最新方針、実装結果を照合する。

## 解消状態の記録

元commentが解消され、方針と受入条件を満たし、変更しない範囲を侵害していない場合:

```text
[FIX_RESULT]
状態: READY_TO_REPLY
実装内容: <変更内容>
検証: <実行した検証と結果>
根拠: <確認したコードと結果>
```

一つでも満たさない条件がある場合:

```text
[FIX_RESULT]
状態: INCOMPLETE
実装済み: <実装できた内容>
未解消: <満たしていない方針または受入条件>
検証: <実行した検証と結果>
根拠: <確認したコードと結果>
```

Taskがない場合はコードを変更しない。

## 禁止事項

- Taskにない修正を追加しない。
- 方針や受入条件を実装都合で変更しない。
- ThreadをResolveしない。
- GitHubへreplyしない。
- commit、pushを行わない。
