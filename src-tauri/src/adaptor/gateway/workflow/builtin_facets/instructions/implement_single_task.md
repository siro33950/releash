# 役割

担当として渡された一つのImplement Taskだけを実装する。

このNodeは実装だけを行う。Spec文書とTask Artifactを変更しない。

## 入力

- `resolve_request` Artifactの`spec_dir`と`directives`
- 担当Task: `{{ item }}`

`{{ resolve_request.spec_dir }}/requirements.md`、`{{ resolve_request.spec_dir }}/behavior.md`、`{{ resolve_request.spec_dir }}/design.md`を全文読む。担当Taskの`files`に記載された既存実装を実際に読む。`directives`を原文どおり遵守する。

Taskの意味と各項目の扱いは`implement-task` Knowledgeに従う。

## 手順

1. Spec 3文書と、担当TaskのTask ID、記載済みRequirement ID、Output、Verifyを読む。
2. SpecとTaskに従い、担当TaskのOutputを満たす範囲だけを実装する。
3. Verifyの全`condition`について、条件に適した方法で実際に成立を確認する。
4. 成立しない条件がある場合は、担当Taskの範囲内で修正して再確認する。

## 禁止事項

- 担当Task以外の範囲・ファイルを変更すること。他のTaskは別Sessionが同時に実装している。
- SpecとTaskにない要求や観測可能な挙動を追加すること。
- 担当TaskのOutputとVerifyを変更すること。

## 完了報告

実装したOutputと確認したVerifyをTask IDとともに報告する。
