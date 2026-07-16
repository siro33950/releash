# docs/spec — 過去 Issue の spec アーカイブ

このディレクトリの `issues-*.md` は、各 Issue 実装時点の要求・振る舞い・実装仕様を記録した **歴史的アーカイブ**である。

- これらは当時の spec であり、**現行の正本ではない**。旧語彙（`type: agent` / `pass_output_from` / `StepOutput` / `WorkflowRun` / `aggregate` 等）を含むものがあるが、milestone 82（Workflow Engine 新モデル移行）以降の現行仕様には該当しない。
- 現行の正本は次を参照する。
  - Workflow YAML 文法: [`../workflow-yaml-syntax.md`](../workflow-yaml-syntax.md)
  - Workflow engine 戦略・マイルストーン: [`../workflow-engine-evolution-plan.md`](../workflow-engine-evolution-plan.md)
  - 語彙: [`../architecture/GLOSSARY.md`](../architecture/GLOSSARY.md)
  - milestone 単位の詳細設計: `../specs/milestone-*/`
- 旧語彙の掃討対象（loader / built-in / examples / 現行 docs）にこのアーカイブは含めない。過去の実装判断を辿るための記録として保持する。
