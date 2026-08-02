# Spec authoring governance

## 目的

Issue、Story、または自由文Requestから、`requirements.md`、`behavior.md`、`design.md`を作成する。3文書を上流から順に作成し、その後は3文書を1単位として「検証・検討・修正」を収束させ、最後に人間が完成文書をレビューする。

各文書に何を書くか、何を書かないか、書式と完成条件は、Nodeへ注入された既存のRequirements、Behavior、Design Knowledgeを正本とする。このPolicyやInstructionへ文書規約を複製しない。

## 文書の責務

- Requirementsは、背景、解決対象、現在状態、Scope、Non-goals、観測可能な要求を所有する。
- Behaviorは、Requirementsから導出できる観測可能な受入条件と検証方法を所有する。
- Designは、RequirementsとBehaviorを対象リポジトリで実現するために必要な内部設計判断を所有する。
- 下流文書で上流文書の不足を補完しない。問題は修正すべき最上流文書へ戻す。

## Sequence

1. Authorが`requirements.md`、`behavior.md`、`design.md`を上流から順に作成する。この段階では検証しない。
2. Validatorが3文書を1単位として、Knowledge、入力、文書間整合、対象コードへ照合し、Findingをすべて列挙する。各Findingには根本原因を持つ文書のownerを付ける。
3. FindingがなければFinalReviewへ進む。あればConsidererがFindingの成立を検討し、成立したものをownerで絞らずに確定する。
4. Repairが確定Findingをowner文書へ反映する。上流文書を変更した場合は、その周回のうちに下流文書を追随させる。
5. 修正後はValidatorへ戻り、Findingがなくなるまで繰り返す。

3文書は独立していない。下流文書を書いて初めて上流文書の不足が見えるため、検証・検討・修正は常に3文書を対象に行い、1文書だけを固めてから次へ進まない。

検証はKnowledgeと入力への適合確認であり、敵対的監査や要求外の改善提案ではない。好み、nit、将来の拡張、要求外のリファクタリングをFindingにしない。

## 実装可能性

Designの完了は、実装者が追加の仕様判断をせず、Design Knowledgeが定める各対象を特定できることを意味する。特定できる粒度もKnowledgeの粒度境界が定める。境界を超えた記述は、実装可能性を高めるものではなく、Knowledge違反として扱う。

## 人間への確認

- 自動処理中に人間へ確認できるのは、権威ある入力同士の矛盾、複数の観測可能な契約からの選択、必須資料・権限不足など、自動判断できない一点がある場合だけである。
- HumanDecisionは文書レビューではない。一度に一つの具体的な質問を行い、回答と解決内容をArtifactに記録して該当Sequenceを再開する。
- 通常の文書レビューは、全自動Sequenceが完了した後のFinalReviewだけで行う。

## 変更範囲

- 書き込める成果物は、検証済み`spec_dir`内の`requirements.md`、`behavior.md`、`design.md`だけである。
- コード、設定、テスト、参照文書、Knowledgeを変更しない。
- 文書本文をArtifactへ複製しない。Artifactには入力参照、Finding、判断、digest、要約だけを記録する。
- 既存文書を更新する場合は、意味が変わらないIDを維持する。
