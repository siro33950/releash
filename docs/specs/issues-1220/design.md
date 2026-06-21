# Design

本書は #1220「WorkspaceサイドバーをRepository→Worktree→Session/Workflowのツリー構造に再編する」の実装設計を定義する。要求は `requirements.md`、観測される振る舞いは `behavior.md` を参照する。

## Overview

Workspace サイドバーを、worktree 管理 UI から作業単位ナビゲーションへ拡張する。

既存の Worktree 管理機能のうち Add Worktree / PR link / delete / refresh は維持し、その下に Session / Workflow / WorkflowSession のツリーを追加する。Group / Filter と Status グループ表示はツリー一本化に伴い廃止する。中央 `AgentChatPanel` の Session タブバーは廃止し、Session 選択・新規作成・close・history restore / archive は左ツリーと Worktree menu へ移す。新規 Workflow run 起動は Worktree 行の create menu から行う。

本 Issue の重要な設計判断は、UIで固めた tree shape をフロント仮データではなく Rust command の projection として返すことにある。フロントは DTO を描画し、クリックされた node id を command / selection request として渡すだけにする。

## Current Prototype To Replace

現在の UI プロトタイプには `buildDraftTreeNodes()` によるフロント仮データがある。これは理想 UI を確認するための一時実装であり、最終実装では削除する。

削除対象になる仮実装の代表:

- branch 名や PR 番号から Session / Workflow 名を推測する処理。
- `Issue #...` や `新しいチャット` など固定文字列の node 生成。
- フロント側で Session / Workflow の親子関係を組み立てる処理。
- フロント側の tree sort ロジック。

残すべき UI 形状:

- Worktree row の icon / name / hover chevron / right actions。
- Session row の agent state icon / hover close。
- Workflow row の workflow icon / hover chevron。
- Worktree menu の hover sub menu。
- Worktree create menu の `NewWorkflow` sub menu と起動 dialog。
- 親名位置に合わせた child indent。

## Architecture

### Responsibility

- Rust:
  - Worktree ごとの Session / Workflow / WorkflowSession projection を構築する。
  - Workflow run と workflow step session の親子関係を解決する。
  - Node の sort order を決める。
  - terminal / closed / archived / running などの状態を DTO として返す。
  - close / restore / archive / delete / PR link に必要な既存 command 境界を維持する。
- Frontend:
  - Rust DTO を描画する。
  - expand / collapse など純粋な view state を保持する。
  - click / hover / menu action を受け付け、既存 command または selection request を発行する。
  - 表示用の薄い format は行うが、親子関係・sort・状態判定はしない。
- MainLayout / App:
  - 左ツリーからの selection request を受け、worktree selection と `CenterSelection` を同期する。
  - WorktreeContent が mount された後に target Session / Workflow を選択できるよう、request id 付きの一回限り request として渡す。
  - `centerMode` のような独立した Agent / Workflow mode state は持たない。中央に何を表示するかは `CenterSelection.kind` から導出する。

### Proposed Rust Projection Command

既存 `useWorktreeList(repoPath)` は Worktree 一覧を返す責務として残す。Session / Workflow tree は worktree 単位で取得する。

```rust
#[tauri::command]
async fn list_workspace_worktree_nodes(
    worktree_path: String,
) -> Result<Vec<WorkspaceTreeNodeDto>, String>
```

候補 DTO:

```rust
#[serde(tag = "kind", rename_all = "camelCase")]
enum WorkspaceTreeNodeDto {
    Session(WorkspaceSessionNodeDto),
    Workflow(WorkspaceWorkflowNodeDto),
}

struct WorkspaceSessionNodeDto {
    id: String,
    worktree_path: String,
    title: String,
    state: String,
    updated_at: i64,
    agent_state: Option<String>,
    workflow_step_session: bool,
}

struct WorkspaceWorkflowNodeDto {
    run_id: String,
    worktree_path: String,
    title: String,
    status: String,
    updated_at: i64,
    children: Vec<WorkspaceSessionNodeDto>,
}
```

実装時に `SessionSummary` / `WorkflowRunSummaryDto` の既存型を再利用できるなら、専用 DTO は薄くしてよい。ただしフロントが WorkflowState を全件読んで親子関係を推測する設計にはしない。

### Projection Rules

Rust command は次の手順で projection を作る。

1. `list_sessions(worktreePath)` 相当で当該 worktree の SessionSummary を取得する。
2. `workflowStepSession !== true` の Session を Worktree 直下 Session とする。
3. `list_workflow_runs(worktreePath)` 相当で WorkflowRunSummary を取得する。
4. 各 run の state projection を取得し、WorkflowSession の session id を収集する。
   - `currentSessionId`
   - `stepHistory[].sessionId`
   - `stepHistory[].childOutputs[].sessionId`
   - `activeParallelSteps[].sessionId`
5. 収集した session id と `workflowStepSession === true` の SessionSummary を突き合わせ、Workflow children とする。
6. Worktree 直下は `Session 優先 -> 名前順` で sort する。
7. Workflow children も名前順で sort する。
8. terminal かつ children が空の Workflow run は `archive_reason = auto_no_sessions` で archive index に自動登録し、ツリー projection から除外する。
9. auto archive された Workflow run は WorkflowHistory projection には `archived_at` と `archive_reason` 付きで残す。

名前の優先順位:

1. 明示 title があれば title。
2. WorkflowSession は stepName があれば stepName。
3. Session は firstMessage があれば firstMessage。
4. Workflow は task があれば task、なければ workflowName。
5. 最後に id。

### CenterSelection Model

左ツリーから中央を駆動する `CenterSelection` を追加する。中央表示の種類は `CenterSelection.kind` から導出し、別途 `centerMode` は保持しない。

```ts
type CenterSelection =
	| {
			kind: "agentSession";
			worktreePath: string;
			sessionId: string;
	  }
	| {
			kind: "workflowRun";
			worktreePath: string;
			runId: string;
			focus?: {
				sessionId?: string;
				stepName?: string;
				runIndex?: number;
			};
	  };

type CenterSelectionRequest = CenterSelection & {
	requestId: number;
	branchName?: string;
	repoName?: string;
};
```

Selection behavior:

- Session row:
  - App opens/selects the worktree tab.
  - MainLayout sets `CenterSelection.kind = "agentSession"`.
  - AgentChatProvider selects `sessionId`.
- WorkflowSession row:
  - App opens/selects the worktree tab.
  - MainLayout sets `CenterSelection.kind = "workflowRun"`.
  - WorkflowView selects `runId`.
  - If `focus.sessionId` is present, WorkflowView selects matching step / transcript.
- Worktree row:
  - No `CenterSelection` change.
  - Only expands/collapses tree.
- Workflow row:
  - No `CenterSelection` change.
  - Only expands/collapses children.
- New Session button:
  - App opens/selects the worktree tab.
  - AgentChatProvider calls existing `createNewSession()` for that worktree.
  - MainLayout sets `CenterSelection.kind = "agentSession"` with the created `sessionId`.

Render derivation:

```ts
selection.kind === "agentSession" -> AgentChatPanel
selection.kind === "workflowRun" -> WorkflowView
```

`ViewToolbar` should not expose an Agent / Workflow toggle in the final #1220 state. If an intermediate migration keeps it temporarily, it must only update `CenterSelection` by selecting an actual target, not write an independent `centerMode`.

### AgentChatPanel Changes

Remove the Session tab bar from `AgentChatPanel`.

Move or remove responsibilities:

| Current tab bar responsibility | New owner |
|---|---|
| Session list display | Workspace tree |
| Session select | Workspace tree Session row |
| New Session `+` | Worktree row new Session action |
| close tab `x` | Session row hover close |
| History popover restore/archive | Worktree menu `SessionHistory` |
| workflow step filtering | Rust projection + Workspace tree |

`ChatSessionView` remains the one-session display surface. When `CenterSelection.kind === "agentSession"`, the central area renders only that selected Session.

Tests under `AgentChatPanel.test.tsx` that assert tab bar behavior must be rewritten or removed according to the new owner.

### WorkflowView Changes

WorkflowView currently has active-worktree-oriented state. It needs a target run selection request.

Required changes:

- Accept `CenterSelection.kind === "workflowRun"` with `runId` and optional `focus.sessionId`.
- Load selected run by `get_workflow_run_state(worktreePath, runId)` for terminal and active runs.
- Keep existing live `get_workflow_state(runId)` / event subscription for active run updates where applicable.
- When a WorkflowSession row is selected, focus/select the matching step or transcript if the view can resolve it.

Workflow parent row does not select a run. Only WorkflowSession row creates a `workflowRun` selection.

### Workspace UI Details

#### Header

Use existing header layout:

- `Workspaces`
- Add Worktree

Do not add a `Project/all` selector row.
Do not restore Group menu, Filter menu, or Status grouping in this Issue.

#### Worktree Row

Left side:

```text
Icon WorktreeName ChevronOnHover
```

Right side:

```text
Menu Create
```

Rules:

- main worktree uses `Home`.
- non-main worktree uses worktree icon.
- no folder icon.
- no colored dot.
- no textual open/closed status.
- row click toggles expansion only.
- hover chevron appears immediately after Worktree name.
- Create opens a menu containing `NewSession` and `NewWorkflow`.

#### Session Row

```text
AgentStateIcon SessionName [CloseOnHover]
```

Rules:

- Same row component for Worktree direct Session and WorkflowSession unless final UI chooses to hide close for WorkflowSession.
- No `Open` / `Closed` text.
- No relative time.
- Hover close button must stop propagation so select is not triggered.

#### Workflow Row

```text
WorkflowIcon WorkflowName ChevronOnHover
```

Rules:

- row click toggles children only.
- no `CenterSelection` change.
- no relative time.
- no status text.

#### Indent

Use CSS variables or layout constants tied to icon width and gap rather than magic depth multiplication.

Desired alignment:

- Worktree children start at Worktree name x-position.
- Workflow children start at Workflow name x-position.
- Same-level Session and Workflow start at the same x-position.

### Worktree Menu

Worktree menu content:

- `SessionHistory`
  - hover opens child menu.
  - Uses existing closed/history Session data and restore/archive operations.
- `WorkflowHistory`
  - hover opens child menu.
  - Uses workflow run list, including terminal runs.
- `PRリンク`
  - disabled or hidden when PR URL is absent.
  - opens existing PR URL.
- `削除`
  - disabled for main worktree.
  - opens existing DeleteWorktreeDialog.

Do not add sort menu. Sort is fixed by Rust projection.

### Worktree Create Menu

Worktree create menu content:

- `NewSession`
  - Sends `CenterSelection.kind = "newAgentSession"` for the target worktree.
- `NewWorkflow`
  - Opens a child menu backed by `useWorkflowConfig`.
  - Selecting a workflow opens a task input dialog.
  - Submitting the dialog invokes `start_workflow` with `workflowName`, `worktreePath`, optional trimmed `task`, and `permissionMode = "ask"`.
  - After a successful start, refresh the tree and select the returned run with `CenterSelection.kind = "workflowRun"`.

### Removed Group / Filter / Status Grouping

`Group by Repo / Status`, status filter behavior, and Status-grouped Worktree sections are not part of the final #1220 Workspace tree. The sidebar renders repositories and their worktrees directly, and the header exposes only Add Worktree.

## Data Refresh

Refresh sources:

- Worktree list refresh remains on existing worktree refresh.
- Session tree refresh should happen after:
  - create session
  - close session
  - restore session
  - archive session
  - session title change
  - session state change / agent state event
- Workflow tree refresh should happen after:
  - workflow run created
  - workflow state changed
  - workflow run terminal state reached
  - workflow history restored/deleted if such operation exists

Implementation can initially refresh per worktree, then optimize with event-specific invalidation.

## Error Handling

- If tree projection command fails for a worktree, show that worktree row and an inline lightweight error under it; do not break the entire Workspace sidebar.
- If Session selection fails because the Session no longer exists, refresh the worktree tree and keep the previous central view.
- If WorkflowSession selection fails because run or session no longer exists, refresh the worktree tree and keep the previous central view.
- PR link absence disables the menu item.
- Delete on main worktree remains disabled.

## Tests

Frontend tests:

- WorkspaceList renders Worktree icon variants.
- Worktree row click toggles children and does not change `CenterSelection`.
- Session row click emits agent selection request.
- Workflow row click toggles children and does not change `CenterSelection`.
- WorkflowSession row click emits workflow selection request.
- Worktree menu opens and contains SessionHistory / WorkflowHistory / PRリンク / 削除.
- Session hover close calls close handler without selecting the row.
- `Open` / `Closed` / relative time / `もっと表示する` are not rendered.
- Header Add Worktree renders.
- NewWorkflow sub menu and workflow start dialog render from the Worktree create menu.
- AgentChatPanel no longer renders session tab bar.

Rust tests:

- Projection separates direct Session from workflow step Session using `workflowStepSession`.
- Projection attaches workflow step Session to the correct run.
- Projection includes `currentSessionId` and historical session ids.
- Projection handles terminal Workflow runs.
- Projection auto archives terminal Workflow runs with no children as `auto_no_sessions`, hides them from the tree, and keeps them in WorkflowHistory.
- Projection sorts direct children as `Session first -> name`.
- Projection does not drop closed / terminal nodes unless archived / explicitly deleted.

Integration smoke:

- Create Session from Worktree action, then select it from tree.
- Close Session from hover action, then restore it from SessionHistory.
- Select WorkflowSession and verify `workflowRun` selection opens the correct run.

## Risks

- Left tree lives outside the current per-worktree AgentChatProvider; selection must cross worktree tab mount boundaries without race conditions.
- Workflow run state may be terminal and not live in runtime memory; selection must use `get_workflow_run_state(worktreePath, runId)` style read-only projection, not only active runtime state.
- Existing AgentChatPanel tab tests are numerous and will need careful rewrite to avoid deleting coverage for close / restore / create behavior.
- Frontend prototype has local tree generation. Leaving it in place would violate Rust-first logic and create future data mismatches.
- History menu UX may become cramped if implemented as raw dropdown items. Existing popover reuse should be evaluated before final implementation.

## Open Questions

- Should WorkflowSession rows expose hover close, or should only direct Session rows expose it?
- Should `SessionHistory` include open Sessions too, or only closed / archived candidates?
- Should `WorkflowHistory` list active Workflow runs, terminal Workflow runs, or both?
- Should terminal Workflow rows expose delete/archive from the WorkflowHistory child menu?
