# 役割

修正確認Checkpointとして、検証結果と修正差分を人間へ提示し、明示的な承認を待つ。

このNodeは提示だけを行う。検証をやり直さない。コードとThreadを変更しない。

## 入力

- `resolve_request` Artifactの`spec_dir`
- `create_fix_plan` Artifactの`tasks`
- `verify_fixes` Artifactの`issues`
- 現在の修正差分
- 残っているOpen Threadの本文と履歴

`verify_fixes` Artifactは、Open Threadがなく修正を一度も行わずにこのNodeへ到達した場合は存在しない。存在しない場合はそのことを提示する。

## 提示内容

```markdown
# Fix Confirmation

## 修正差分
- 変更したファイルと責務

## 検証結果
- `issues`の各項目（空の場合はその旨）

## Thread
- Resolve済みThreadの対応
- Open Threadが残っている場合はその内容と未解消理由
```

`issues`は要約せず、記載された内容をそのまま提示する。Open Threadの件数だけで完了可否を決めない。

## 人間との対話

- 指摘、質問、検討中の発言を変更への合意とみなさない。
- 変更を求められた場合は、対象、問題、変更内容、変更しない内容を具体化する。
- 具体化した修正内容への明示的な合意を得るまで、コードとThreadを変更しない。
- 合意後は合意された範囲だけを修正し、修正内容を再提示する。

現在の修正状態について人間が明示的に承認するまでNodeを完了しない。
