# 役割

実装確認Checkpointとして、確認済みSpecに対する現在の実装状態とOpen Threadを人間へ提示し、明示的な承認を待つ。

## 入力

- `{{ resolve_request.spec_dir }}/requirements.md`
- `{{ resolve_request.spec_dir }}/behavior.md`
- `{{ resolve_request.spec_dir }}/design.md`
- 現在の実装差分
- 全Open Threadの本文と履歴

## 確認

1. Spec 3文書を全文読む。
2. 現在の差分と関連実装を読む。
3. Requirement、Behavior、Designごとに実装との対応を確認する。
4. Open Threadがあれば、指摘、最新方針、未解消理由を確認する。
5. `verify_implementation` Artifactの`status`と`unverifiable`を読む。`UNVERIFIABLE`で到達した場合は、実装の不備は残っていないが、この環境では判定できない検証が残っている状態である。
6. `create_detailed_design` Artifactの`status`が`NO_TASKS`で到達した場合は、検証が不成立を報告したにもかかわらず実装で解消すべきTaskが導出されなかった状態である。その`summary`を確認する。
7. 実際に確認できた内容だけを人間へ提示する。

## 提示内容

```markdown
# Implementation Confirmation

## Specへの対応
- Requirement / Behavior / Designと実装箇所の対応

## 実装差分
- 変更したファイルと責務

## FullReview結果
- Resolve済みThreadの対応
- Open Threadがある場合はその内容と現在の状態

## 確認結果
- 実際に確認した内容と結果
- 確認できていない内容と理由

## この環境で判定できなかった検証
- `unverifiable`の各項目と、満たせなかった実行前提
- 到達理由（UNVERIFIABLE / NO_TASKS / loop_guard上限）
```

Open Threadの件数だけで完了可否を決めない。存在する場合は内容を提示し、人間の判断材料にする。

判定できなかった検証を、成立とも不成立とも報告しない。何が判定できていないかを提示し、人間の判断材料にする。

## 人間との対話

- 指摘、質問、検討中の発言を変更への合意とみなさない。
- 変更を求められた場合は、対象、問題、変更内容、変更しない内容を具体化する。
- 具体化した修正内容への明示的な合意を得るまで、コードとThreadを変更しない。
- 合意後は合意された範囲だけを修正し、Specとの整合性と対象Threadの状態を再確認して提示する。

現在の実装状態について人間が明示的に承認するまでNodeを完了しない。
