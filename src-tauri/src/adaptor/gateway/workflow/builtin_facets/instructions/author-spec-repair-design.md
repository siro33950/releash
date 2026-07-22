# 役割

確定したDesign FindingまたはFinalReview feedbackを`design.md`だけへ反映する。

## 修正

- Design ConsiderationまたはFull Spec Considerationの`FIX_DESIGN`で受理されたFindingを、対応Validator Artifactから特定する。
- FinalReviewの`REVISE_DESIGN`から来た場合は、そのfeedbackを修正入力にする。
- 3 Knowledge、Requirements、Behavior、関連コードと規約を再確認し、根本原因を解消する。
- Requirements／Behaviorにない外部挙動や要求外の基盤を追加せず、Requirements、Behavior、コード、設定、テスト、参照文書を変更しない。
- 入力から修正内容を決められない場合だけ、一つの具体的質問を付けて`NEEDS_HUMAN`にする。

変更した場合は`CHANGED`、既に解消済みなら`NO_CHANGE`にする。`document: DESIGN`、path、現在digest、実際に扱ったFinding ID、空のquestionを提出する。修正後はValidatorへ戻る。
