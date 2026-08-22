# 役割

受理されたFindingをowner文書へ反映する。

## 修正

- `spec_consider`が受理したFindingを、対応するValidator Artifactから特定する。受理されたFindingをすべて扱う。
- Findingのownerが`REQUIREMENTS`、`BEHAVIOR`、`DESIGN`のいずれであっても、このNodeで修正する。
- 上流から下流の順に修正する。Requirementsを変更した場合は、その変更から導出されるBehaviorとDesignを同時に追随させる。Behaviorを変更した場合は、Designを追随させる。
- 各文書の修正は、対応するKnowledgeの規約と粒度境界の内側で行う。粒度超過のFindingは該当記述の削除で解消し、加筆で補わない。
- 意味が変わらないR-ID、B-IDを維持する。
- Findingが指す根本原因を解消する最小の意味変更を行い、要求外の改善を加えない。
- `spec_dir`内の`requirements.md`、`behavior.md`、`design.md`以外を変更しない。コード、設定、テスト、参照文書、Knowledgeを変更しない。
- 入力から修正内容を決められない場合は推測せず、具体的な質問を提示して人間の回答を待つ。回答を得るまで完了を提出しない。

実際に変更した文書のpathを`changed_documents`へ、実際に扱ったFinding IDを`addressed_finding_ids`へ提出する。

修正後は人間の最終レビューへ進む。修正の確認と残る問題の裁定は最終レビューの人間が行う。
