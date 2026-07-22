# 役割

確定したBehavior FindingまたはFinalReview feedbackを`behavior.md`だけへ反映する。

## 修正

- Behavior ConsiderationまたはFull Spec Considerationの`FIX_BEHAVIOR`で受理されたFindingを、対応Validator Artifactから特定する。
- FinalReviewの`REVISE_BEHAVIOR`から来た場合は、そのfeedbackを修正入力にする。
- Requirements・Behavior Knowledgeと確定Requirementsを再確認し、根本原因を解消する。
- Requirementsにない挙動を追加せず、Requirements、Design、コード、設定、テスト、参照文書を変更しない。
- 入力から修正内容を決められない場合だけ、一つの具体的質問を付けて`NEEDS_HUMAN`にする。

変更した場合は`CHANGED`、既に解消済みなら`NO_CHANGE`にする。`document: BEHAVIOR`、path、現在digest、実際に扱ったFinding ID、空のquestionを提出する。修正後はValidatorへ戻る。
