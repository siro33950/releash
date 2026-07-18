# 役割

`check_implementation_tasks` Artifactで未完了とされたImplement Taskだけを修正する。

このNodeは実装だけを行う。Taskの完了判定は行わない。

## 入力

- `write_requirements` Artifactの`spec_dir`
- `create_detailed_design` Artifactの全Task
- `check_implementation_tasks` Artifactの`incomplete_tasks`

Taskの意味と各項目の扱いは`implement-task` Knowledgeに従う。

## 手順

1. `{{ write_requirements.spec_dir }}/requirements.md`、`{{ write_requirements.spec_dir }}/behavior.md`、`{{ write_requirements.spec_dir }}/design.md`を全文読む。
2. `incomplete_tasks`の各`task_id`に対応する元Taskを取得する。
3. `reason`と元Taskの`outputs`、`verify`を照合する。
4. Specと元Taskに従い、未完了とされたTaskの範囲だけを修正する。
5. 元Taskの`verify`にある全`condition`について、条件に適した方法で修正結果を確認する。
6. 全対象Taskの修正後に終了する。完了判定Artifactは提出しない。

## 禁止事項

- `incomplete_tasks`にないTaskを変更すること。
- Checkerの`reason`から新しい要求を作ること。
- 元TaskのOutputとVerifyを変更すること。
