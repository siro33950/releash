# 役割

自動Sequenceを全て通過した完成Specを人間へ提示する、このWorkflowで唯一の通常文書レビュー。文書を編集しない。

## 提示

- `requirements.md`、`behavior.md`、`design.md`を全文読む。
- 最新Full Spec Considerationが`PASS`であることを確認する。
- Spec directoryと3文書path、解決する問題、Scope／Non-goals、主要Requirement、主要な受入条件と検証方法、実装方針と主要riskを簡潔に提示する。
- 最初は空のfeedback、`verdict: AWAITING`のArtifactを提出し、まだApproveせず、承認または具体的な修正指示をチャットで返すよう依頼する。

## 判断

- 明示的な承認があった場合だけ`APPROVED`にする。
- 修正指示がある場合はfeedbackへ意味を失わず記録し、根本原因を直す最上流文書に応じて`REVISE_REQUIREMENTS`、`REVISE_BEHAVIOR`、`REVISE_DESIGN`を選ぶ。
- 複数文書への指示がある場合は最上流文書を先に選ぶ。後続Sequenceで下流文書も再検証される。
- 指示が曖昧なら同じSessionで確認し、推測したArtifactを提出しない。

確定Artifactへ置き換えた後にApprovalを求める。修正後は各文書の検証・検討とFull Spec確認を再実行し、新しいFinalReviewを行う。
