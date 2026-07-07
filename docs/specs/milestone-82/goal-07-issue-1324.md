# Goal: #1324 approval node を session.gate=approval に移行

まず `docs/specs/milestone-82/goal-common.md` を読み、そこに書かれた必読ドキュメント・設計判断・横断ルール・品質ゲートに従うこと。`gh issue view 1324 --repo siro33950/releash` で issue 本文を読むこと。

approval を node 種別から完全に消し、session の完了 gate に一本化する。#1322 で構文上は session.gate になっているため、本 goal は gate 必須化・typed command 化・reject/rerun 系の全削除・UI/CLI 整合が中心。

## 実装内容

1. **gate 必須化**: `session.gate`（auto | approval）を session の必須 field にする（省略は Diagnostic）。built-in 12 本に gate を明示する。
2. **gate 挙動**: `gate: auto` は turn 完了で自動的に node 完了判定へ、`gate: approval` は承認されるまで完了しない。承認しない間、人間は同じ session に追加指示を続けられる（既存 approval chat: `usecase/workflow/approval_chat.rs` / send_workflow_approval_chat_message を gate 語彙に接続・維持）。approval は状態としては WorkflowExecution.status = waiting_approval（P2）として観測される。
3. **approve typed command**: approve を WorkflowExecution / NodeExecution に対する typed command として検証する（既存 ApprovalCommand / UnauthorizedApprovalTarget / stale target validation を新語彙で維持・強化）。stale（対象 node が現在の承認待ちでない）/ unauthorized は拒否。
4. **reject / rerun の全削除**: 却下という別操作を持たない。修正依頼は同じ approval session への追加指示で表現する。workflow 全体を止める場合は abort（WorkflowExecution typed command）だけを使う。削除対象:
   - `domain/workflow/services/approval_rules.rs` の can_reject / reject_structured_output / validate_reject_reason_text
   - `transition.rs` decide_approval_action の reject 分岐、`match == 'reject'` rule の特別扱い
   - CLI `Reject` subcommand（cli/mod.rs / workflow.rs）、pending payload の Reject variant、ApprovalDecisionInput::Reject（Tauri command）、event の ApprovalResolved decision record の reject 表現
   - protocol の ApprovalOperationsView.can_reject、frontend の reject ボタン / reason 入力 / canReject（WorkflowView.tsx、workspace-tree.ts、useWorkspaceWorkflowStepDetail.ts）とそのテスト
   - rerun 相当の操作・rule（存在すれば）
5. **語彙の掃除**: built-in / UI / CLI / API の表示・引数から `approval` node 種別語彙を消す（gate: approval の session として表示する）。

## テスト

- `gate` 必須（省略 Diagnostic）、`gate: auto` の自動完了、`gate: approval` の待機。
- 承認待ち中に同じ session へ追加指示できること。approve 成功で node 完了 → 次へ遷移。
- stale / unauthorized approve の拒否。
- reject command / reject rule / rerun が CLI / API / pending 経路のどこでも受理されないこと（regression）。
- abort が WorkflowExecution typed command として通ること。
