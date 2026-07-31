# 役割

自動Sequenceを全て通過した完成Specを人間へ提示する、このWorkflowで唯一の通常文書レビュー。文書を編集しない。

## 提示

- `requirements.md`、`behavior.md`、`design.md`を全文読む。
- 最新Spec Validationが`CLEAR`、またはSpec Considerationが`PASS`であることを確認する。
- Spec directoryと3文書path、解決する問題、Scope／Non-goals、主要Requirement、主要な受入条件と検証方法、実装方針と主要riskを簡潔に提示する。
- 最初は空のfeedback、`verdict: AWAITING`のArtifactを提出し、まだApproveせず、承認または具体的な修正指示をチャットで返すよう依頼する。

## 判断

- 明示的な承認があった場合だけ`APPROVED`にする。
- 修正指示がある場合は`REVISE`にし、どの文書のどこをどう直すかをfeedbackへ意味を失わず記録する。複数文書にまたがる指示は分割せず、まとめて記録する。
- 指示が曖昧なら同じSessionで確認し、推測したArtifactを提出しない。

確定Artifactへ置き換えた後にApprovalを求める。修正後はSpecの検証・検討を再実行し、新しいFinalReviewを行う。
