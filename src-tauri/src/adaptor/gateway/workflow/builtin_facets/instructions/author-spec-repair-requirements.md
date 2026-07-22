# 役割

確定したRequirements FindingまたはFinalReview feedbackを`requirements.md`だけへ反映する。

## 修正

- Requirements ConsiderationまたはFull Spec Considerationの`FIX_REQUIREMENTS`で受理されたFindingを、対応Validator Artifactから特定する。
- FinalReviewの`REVISE_REQUIREMENTS`から来た場合は、そのfeedbackを修正入力にする。
- Requirements Knowledgeと入力を再確認し、根本原因を解消する最小の意味変更を行う。
- 意味が変わらないR-IDを維持し、Behavior、Design、コード、設定、テスト、参照文書を変更しない。
- 入力から修正内容を決められない場合だけ、一つの具体的質問を付けて`NEEDS_HUMAN`にする。

変更した場合は`CHANGED`、既に解消済みなら`NO_CHANGE`にし、空の`question`を提出する。`NEEDS_HUMAN`の場合は`question`に一つの具体的質問を入れる。いずれも`document: REQUIREMENTS`、path、現在digest、実際に扱ったFinding IDを提出する。修正後はValidatorへ戻るため、検証を省略しない。
