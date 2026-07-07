# Goal: #1331 WorkflowExecution / NodeExecution read model

まず `docs/specs/milestone-82/goal-common.md` を読み、そこに書かれた必読ドキュメント・設計判断・横断ルール・品質ゲートに従うこと。`gh issue view 1331 --repo siro33950/releash` で issue 本文を読むこと。

公開 read model を WorkflowExecution / NodeExecution / Artifact / Fanout 主語に統一し、run/step 語彙を外部から消す。

## 実装内容

1. **read model 型**: event log からの projection として WorkflowExecution（id / definition 名 / status / 現在 node / 起動元 / worktree / timestamps / 失敗理由 / total token usage）、NodeExecution（execution_id / node 名 / kind / status / 反復回 / session 参照 / Artifact / token usage / 失敗理由 / timestamps）、Fanout（親 NodeExecution と子 NodeExecution 群の束ね）、Artifact（node 名参照）を定義する。旧 `WorkflowStateSnapshot` / `StepHistoryEntry` / `StepOutput` / `ParallelStepState` の**公開語彙**を廃し、これらへ寄せる（`domain/value_objects/state.rs`・`step_output.rs`、`adaptor/gateway/workflow/state.rs`、`adaptor/protocol/workflow.rs` の WorkflowStateFieldsView を再構成）。
2. **event log の主語 rename**: WorkflowEvent の `run_id` → `execution_id`、RunStarted → ExecutionStarted 等、Fanout 語彙と合わせて event schema を新語彙に揃える（P4: NDJSON 在庫破棄・互換 reader なし）。保存パス（workflow_logs/ / workflow_runs/）も executions 語彙のディレクトリへ改名してよい。
3. **projection の一本化**: 現在状態は WorkflowExecution / NodeExecution から読み、履歴は event log projection で辿る。event_projection.rs / state_projection_repository.rs / step_detail_projection_repository.rs / runtime_view 系を新 read model に再編し、full-retention / full-recompute 経路を増やさない（既存の on-demand projection 方式を維持）。
4. **外部 API の語彙統一**: Tauri command を executions 語彙に rename する（list_workflow_runs → list_workflow_executions、get_workflow_run / get_workflow_run_state / get_workflow_run_log → get_workflow_execution(_state/_log)、resolve_active_run_by_worktree / resolve_worktree_by_run、restore_workspace_workflow_run / archive_workspace_workflow_run、get_workspace_workflow_step_detail → node detail、引数 runId → executionId、stepName → nodeName）。`run_id` / `WorkflowRun` / `runs` / step 語彙が外部 API（Tauri command 名・引数・戻り値 DTO）に残らないこと。domain の RunId / WorkflowRunRecord / RunStatus 等の内部型も WorkflowExecution 語彙に改名する（GLOSSARY: RunId → WorkflowExecution.id）。
5. **frontend 追随**: `src/types/workflow.ts`（WorkflowRunSummary → WorkflowExecutionSummary、runId → executionId、step* → node*）、`workspace-tree.ts`（WorkspaceWorkflowStepNode → node 語彙）、hooks（useWorkflowState / useWorkspaceWorkflowStepDetail / useAutomation）、WorkspaceList.tsx、WorkflowView.tsx、および `usecase/workflow/workspace_tree.rs`・`usecase/agent_session/status.rs` の結合部を新 read model 語彙へ更新する。frontend は mirror に徹し、domain decision を持ち込まない。
6. **境界テスト**: UI（Tauri command）と CLI（file-direct read）が同じ usecase query service / read model を読むことを境界テストで固定する（Remote surface は未実装のため、「同一 query service を複数 surface が共有する」構造の検証で満たす。ws/daemon は #77-79 の領分）。

## 削除対象

- 公開語彙としての `StepHistoryEntry` / `StepOutput` / `ParallelStepState` / `WorkflowStateSnapshot`
- 外部 API / frontend の `run_id` / `runId` / `WorkflowRun` / `runs` / `step` 語彙
- 旧 workflow state / 旧 NDJSON / 旧 YAML 互換 adapter（残存していれば）

## テスト

- event log から WorkflowExecution / NodeExecution / Artifact / Fanout が再構築できること（通常完走 / 失敗 / abort / fanout / approval 待ちの各シナリオ）。
- UI / CLI が同じ read model を読む境界テスト。
- Tauri command 名・引数・DTO・frontend 型に旧 step / run 語彙が残っていないこと（grep ベースの検査テスト可）。
