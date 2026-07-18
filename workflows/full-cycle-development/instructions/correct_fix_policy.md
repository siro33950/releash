# 役割

渡された一つの方針修正Taskに従い、対象Threadの`[FIX_POLICY]`を相互に整合する内容へ修正する。

このNodeは方針の修正だけを行う。コード、Spec文書、Task対象外のThreadを変更しない。

## 入力

- Task ID: `{{ item.task_id }}`
- 対象Thread: `{{ item.thread_ids }}`
- 問題: `{{ item.problem }}`
- 必要な修正: `{{ item.required_changes }}`
- `{{ write_requirements.spec_dir }}/requirements.md`
- `{{ write_requirements.spec_dir }}/behavior.md`
- `{{ write_requirements.spec_dir }}/design.md`

## 手順

1. Taskに含まれる全Threadの本文と全履歴を読む。
2. 各Threadの元指摘と最新`[FIX_POLICY]`を確認する。
3. Taskの問題と必要な修正を、Specと現在の実装に照らして確認する。
4. Task内のすべての問題を同時に解消する方針を作る。
5. 対象Threadそれぞれへ、更新後の完全な`[FIX_POLICY]`をCommentする。

## Comment形式

```text
[FIX_POLICY]
修正Task: <task_id>
置換理由: <以前の方針の問題と今回の修正>
妥当性: <元指摘が成立する根拠>
Spec根拠: <Requirement ID、Behavior ID、Designの該当箇所>
修正方針: <他方針と両立する完全な方針>
受入条件: <修正後に確認できる結果>
変更しない範囲: <扱わない内容。なければ「なし」>
```

新しいCommentを、そのThreadの有効な最新方針として扱える完全な内容にする。以前のCommentとの差分だけを書かない。

## 禁止事項

- Taskにない問題を追加すること。
- Specから導出できない要求を追加すること。
- Taskに含まれないThreadを変更すること。
- ThreadをResolveすること。
- 実装すること。
