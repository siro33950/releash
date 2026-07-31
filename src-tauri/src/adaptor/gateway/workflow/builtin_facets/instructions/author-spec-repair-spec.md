# 役割

受理されたFindingまたはFinalReview feedbackを、owner文書へ反映する。

## 修正

- `spec_consider`が受理したFindingを、対応するValidator Artifactから特定する。受理されたFindingをすべて扱い、次の周回へ持ち越さない。
- FinalReviewの`REVISE`から来た場合は、そのfeedbackを修正入力にする。
- Findingのownerが`REQUIREMENTS`、`BEHAVIOR`、`DESIGN`のいずれであっても、この周回で修正する。
- 上流から下流の順に修正する。Requirementsを変更した場合は、その変更から導出されるBehaviorとDesignを同じ周回で追随させる。Behaviorを変更した場合は、Designを追随させる。
- 各文書の修正は、対応するKnowledgeの規約と粒度境界の内側で行う。粒度超過のFindingは該当記述の削除で解消し、加筆で補わない。
- 意味が変わらないR-ID、B-IDを維持する。
- Findingが指す根本原因を解消する最小の意味変更を行い、要求外の改善を加えない。
- `spec_dir`内の`requirements.md`、`behavior.md`、`design.md`以外を変更しない。コード、設定、テスト、参照文書、Knowledgeを変更しない。
- 入力から修正内容を決められない場合だけ、一つの具体的質問を付けて`NEEDS_HUMAN`にする。

いずれかの文書を変更した場合は`CHANGED`、受理Findingがすべて解消済みで変更が不要だった場合は`NO_CHANGE`にし、空の`question`を提出する。`NEEDS_HUMAN`の場合は`question`に一つの具体的質問を入れる。いずれも実際に変更した文書のpathを`changed_documents`へ、3文書の現在combined digestを`digest`へ、実際に扱ったFinding IDを提出する。修正後はValidatorへ戻るため、検証を省略しない。
