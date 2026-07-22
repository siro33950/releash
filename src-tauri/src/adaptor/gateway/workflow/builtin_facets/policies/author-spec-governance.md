# Spec authoring governance

## 目的

Issue、Story、または自由文Requestから、`requirements.md`、`behavior.md`、`design.md`を作成する。各文書を個別に「作成・検証・検討・修正」し、3文書全体の整合確認を通した後、最後に人間が完成文書をレビューする。

各文書に何を書くか、何を書かないか、書式と完成条件は、Nodeへ注入された既存のRequirements、Behavior、Design Knowledgeを正本とする。このPolicyやInstructionへ文書規約を複製しない。

## 文書の責務

- Requirementsは、背景、解決対象、現在状態、Scope、Non-goals、観測可能な要求を所有する。
- Behaviorは、Requirementsから導出できる観測可能な受入条件と検証方法を所有する。
- Designは、RequirementsとBehaviorを対象リポジトリで実現するために必要な内部設計判断を所有する。
- 下流文書で上流文書の不足を補完しない。問題は修正すべき最上流文書へ戻す。

## Sequence

Requirements、Behavior、Designはそれぞれ独立した次のSequenceで管理する。

1. Authorが対象文書だけを作成する。
2. ValidatorがKnowledge、入力、上流文書へ照合してFindingを列挙する。
3. ConsidererがFindingの成立、重複、修正ownerを検討する。
4. FindingがあればRepairがowner文書だけを修正する。
5. 修正後はValidatorへ戻り、Findingがなくなるまで繰り返す。

3文書が個別に通過した後、Full Specでも同じく検証・検討し、問題があればowner文書のRepairへ戻す。

検証はKnowledgeと入力への適合確認であり、敵対的監査や要求外の改善提案ではない。好み、nit、将来の拡張、要求外のリファクタリングをFindingにしない。

## 実装可能性

Designの完了は、実装者が追加の仕様判断をせず、責務owner、主要な変更対象、interface、data、control flow、error処理、互換境界、必要な検証を特定できることを意味する。既存規約から安全に決められるprivateな細部まで固定する必要はない。

## 人間への確認

- 自動処理中に人間へ確認できるのは、権威ある入力同士の矛盾、複数の観測可能な契約からの選択、必須資料・権限不足など、自動判断できない一点がある場合だけである。
- HumanDecisionは文書レビューではない。一度に一つの具体的な質問を行い、回答と解決内容をArtifactに記録して該当Sequenceを再開する。
- 通常の文書レビューは、全自動Sequenceが完了した後のFinalReviewだけで行う。

## 変更範囲

- 書き込める成果物は、検証済み`spec_dir`内の`requirements.md`、`behavior.md`、`design.md`だけである。
- コード、設定、テスト、参照文書、Knowledgeを変更しない。
- 文書本文をArtifactへ複製しない。Artifactには入力参照、Finding、判断、digest、要約だけを記録する。
- 既存文書を更新する場合は、意味が変わらないIDを維持し、人間のFinalReview feedbackを該当文書の修正入力として扱う。
