Implement Task Artifactに従ってコードを実装する。

## 入力

- `write_requirements` Artifactの`spec_dir`
- `create_detailed_design` Artifactの`tasks`

Taskの意味と各項目の扱いは`implement-task` Knowledgeに従う。

## 基本方針

- 全Taskを依存関係に従って実装する。
- TaskのOutputに記載された成果だけを作る。
- Taskに記載されていない範囲を変更しない。
- 実装中に新しい要求や観測可能な挙動を作らない。
- `Parallel`はTaskの独立性を示す情報であり、このNodeをFanoutする指示として扱わない。

## プロセス

1. `{{ write_requirements.spec_dir }}/requirements.md`、`{{ write_requirements.spec_dir }}/behavior.md`、`{{ write_requirements.spec_dir }}/design.md`を全文読む。
2. 全TaskのTask ID、記載済みのRequirement ID、依存関係、並列実行可否を把握する。
3. 依存関係から着手可能なTaskを特定する。
4. 対象TaskのFilesに記載された既存実装を実際に読む。
5. SpecとTaskに従い、Outputを満たす範囲だけを実装する。
6. Verifyの全`condition`について、条件に適した方法で実際に成立を確認する。
7. 成立しない条件がある場合は、そのTaskの範囲内で修正して再確認する。
8. Taskの完了後、次に着手可能になったTaskについて手順4から繰り返す。
9. 全TaskのOutputとVerifyが完了したことを確認する。

## 完了報告

Task IDごとに、実装したOutputと確認したVerifyを報告する。
