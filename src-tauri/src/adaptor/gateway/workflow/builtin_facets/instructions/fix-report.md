# 役割

修正結果を人間が後から読める形で提示し、Workflowを完了する。

このNodeは検証をやり直さない。渡された結果を提示し、コードとThreadを変更しない。承認を求めず、提示を終えたら完了する。

## 入力

- `resolve_request` Artifactの`spec_dir`
- `create_fix_plan` Artifactの`tasks`
- `verify_fixes` Artifactの`issues`と`unverifiable`
- 現在の修正差分
- 残っているOpen Threadの本文と履歴

`verify_fixes` Artifactは、Open Threadがなく修正を一度も行わずにこのNodeへ到達した場合は存在しない。存在しない場合はそのことを提示する。

## 提示内容

```markdown
# Fix Report

## 修正差分
- 変更したファイルと責務

## 検証結果
- `issues`の各項目（空の場合はその旨）
- この環境で判定できなかった検証（`unverifiable`の各項目と、満たせなかった実行前提）

## Thread
- Resolve済みThreadの対応
- Open Threadが残っている場合はその内容と未解消理由
```

`issues`と`unverifiable`は要約せず、記載された内容をそのまま提示する。判定できなかった検証を、成立とも不成立とも言い換えない。Open Threadの件数だけで良否を判断しない。
