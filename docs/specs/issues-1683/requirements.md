# Context

- 要求の正本は GitHub Issue #1683「[workspace tree] ノードの状態がアイコンの色から判別できず、実態とも一致しない」（https://github.com/siro33950/releash/issues/1683 、state: OPEN、label: bug、milestone なし、comment なし）。追加の自由文指示はない。
- Workspace ツリーは Tauri command 経路でだけ提供される。local API と CLI は Workspace ツリーを返していない。
- ツリー行のアイコン形状は行の種類で固定されており、状態は色にしか出ない。leaf 行は `contentKind` に応じて Bot / Terminal（`src/components/workspace/WorkspaceList.tsx:183`）、Sequence 行は ListTree（`src/components/workspace/WorkspaceList.tsx:406`）、Fanout 行は GitFork（`src/components/workspace/FanoutRowStatusIcon.tsx:12`）。
- アプリケーションロジックは Rust が所有し、frontend に許すのは表示とレイアウト制御である（`AGENTS.md` アーキテクチャ原則）。ノード状態の分類規則を frontend に置くことはできない。
- 現在状態の確認は、リポジトリ内コードの読解によって行った。アプリを起動しての画面確認は行っていない。

# Outcome

Releash の Workspace ツリーを見る開発者が対象である。

現在、ツリー行の色はノードの状態を判別する用をなしていない。production で一度も出ない色があり、意味の異なる状態が同じ色になり、介入が必要な状態や復元できない状態が「実行中」と同じ色で表示される。親行の集約規則も Sequence / Fanout / Workflow root で 3 種類に割れている。そのため、開発者はツリーを見ても、どのノードが自分の介入を待っているのか、どのノードが失敗して止まっているのかを判断できず、詳細ペインを一つずつ開くまで実態が分からない。

変更後は、ツリー行の色が「実行中」「介入が必要」「失敗」「動いていない」の 4 つの意味だけを表す。どの行についても色が一意に決まり、その色が実際のノードの状態と一致する。親行の色は配下を含めた最も重い状態を表し、Sequence と Fanout で規則が変わらない。開発者はツリーを見るだけで、介入が必要な箇所と失敗している箇所を特定できる。

# Current Behavior

## ツリー行の状態表示

ツリー行の状態色は `workflowNodeIconClasses`（`src/components/workspace/WorkflowNodeStatusIcon.tsx:12-20`）が状態値ごとに定める 7 色である。branch 行も同じ色マップを共有する（`src/components/workspace/WorkspaceBranchStatusIcon.tsx:23-24`）。`running` / `waiting` のときだけアイコンが pulse する（`src/components/workspace/WorkflowNodeStatusIcon.tsx:22-27`）。pulse を適用しているのはツリー行だけで、leaf 行は `src/components/workspace/WorkspaceList.tsx:184-185`、branch 行は `src/components/workspace/WorkspaceBranchStatusIcon.tsx:25-26` である。詳細ペインのノード状態アイコンは pulse せず、`running` は Loader2 の spin で表す（`src/components/panels/NodeContentView/NodeContentView.tsx:152-153`、`src/components/workspace/WorkflowNodeStatusIcon.tsx:36-58`）。

状態値は `WorkspaceNodeStatus`（`src-tauri/src/domain/workspace_tree/value_objects/mod.rs:35-43`）の `running` / `paused` / `failed` / `waiting` / `interrupted` / `aborted` / `completed` の 7 値で、`status` として文字列のままツリー DTO に載る（`src-tauri/src/usecase/workflow/workspace_tree.rs:54`、`src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs:318,391,403`）。frontend 側の型も同じ 7 値である（`src/types/workspace-tree.ts:22-29`）。

## 到達不能な色がある

`interrupted`（オレンジ、`src/components/workspace/WorkflowNodeStatusIcon.tsx:17`）を設定するのは `workflow_status()`（`src-tauri/src/domain/workspace_tree/entities/mod.rs:1004-1014`）だけで、その分岐は `#[cfg(test)]` である。元になる `ExecutionStatus::Interrupted` 自体が `#[cfg(test)]` であり（`src-tauri/src/domain/workflow/value_objects/execution.rs:11-12`）、この variant を生成する経路（`src-tauri/src/adaptor/gateway/workflow/state_notification_gateway.rs:68`、`src-tauri/src/adaptor/gateway/workflow/execution_store.rs:206`）もすべて `#[cfg(test)]` で閉じている。production ビルドでこの色は一度も出ない。

加えて `workflow_status()` が付く Workflow ノードは DTO 化されず子へ平坦化されるため（`src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs:383-385`）、Workflow root の status は表示に一切使われない。

`WorkspaceNodeStatus::Interrupted` は表示以外に、resume 可否と recovery 理由を算出する `recompute_workflow_recovery_capabilities()` の条件にも読まれている（`src-tauri/src/domain/workspace_tree/entities/mod.rs:629,659`）。到達不能なため、この条件は production では常に false である。

## 意味の異なる状態が同色になる

`paused` と `aborted` はどちらも `text-muted-foreground` である（`src/components/workspace/WorkflowNodeStatusIcon.tsx:14,18`）。`paused` はユーザーが Stop した再開可能なノードで、`stop_workflow_execution` が実行中の Node Attempt を pause する経路で付く（`src-tauri/src/adaptor/gateway/workflow/workflow_host/lifecycle_commands.rs:152`、`src-tauri/src/domain/workspace_tree/projection.rs:189-191`）。`aborted` は中止されて終わったノードで、`NodeFailed` の `failure_kind` が `UserAbort` のときに付き `can_retry` は false になる（`src-tauri/src/domain/workspace_tree/entities/mod.rs:856-881`）。再開できるものとできないものが同じ見た目になる。`text-muted-foreground` は行の既定テキスト色と同じなので、状態が付いていないようにしか見えない。

## 状態に載っていない情報がある

- crash 後の recovery fence は `RecoveryFenceProjected` により `recovery_owner_reason` へ入るだけで（`src-tauri/src/domain/workspace_tree/projection.rs:58-61`）、ノードの `status` は変わらない。ツリー DTO（`src-tauri/src/usecase/workflow/workspace_tree.rs:51-66` の `WorkspaceNodeDto`）に recovery 情報がないため、復元できないノードが `running` として青 + pulse で「実行中」に見える。詳細ペイン（`src/components/panels/NodeContentView/NodeContentView.tsx:173-177`）を開かない限り分からない。
- submit / stop の片方だけを受け取った状態（`NodeCompletionSignalState::SubmitReceived` / `StopReceived`、`src-tauri/src/domain/workflow/value_objects/node_execution.rs:4-10`）も `status` に出ず、`running` と同じ青になる。Stop だけ来て Submit がないノードは人が retry を判断するしかないが（`can_retry()` が partial signal を条件に含む、`src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs:201-207`）、ツリーでは通常の実行中と区別できない。

これらの詳細は detail 側にだけ載っている。`WorkspaceNodeDetailDto` は `submit_received` / `stop_received` / `recovery_reason` を持つ（`src-tauri/src/usecase/workflow/workspace_tree.rs:125-126,133`、`src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs:469`）。

## 親行の集約規則が 3 種に割れている

- Fanout: `aggregate_status()`（`src-tauri/src/domain/workspace_tree/entities/mod.rs:1016-1034`）で子と自分の最小ランクを採る。ランクは `running`=1 が最優先で、子が失敗していても `running` の子が 1 つあれば親は `running` になる。ランクには欠番 4 が残っている。
- Sequence: 子から集約せず自分の Node イベントだけを正とする（`src-tauri/src/domain/workspace_tree/entities/mod.rs:537-539`）。子からは `updated_at` だけを伝播する。
- Workflow root: 最後の子から導出するロジックがあるが（`src-tauri/src/domain/workspace_tree/entities/mod.rs:542-550`）、平坦化されて表示されないため効かない。

## 再現手順

1. Workflow を実行し、実行中のノードを含むツリーを表示する。
2. 実行中の Workflow を Stop する。ツリー行は `text-muted-foreground` になる（`paused`）。
3. 別の Workflow のノードを abort する。ツリー行は同じ `text-muted-foreground` になる（`aborted`）。
4. どちらの行も既定テキスト色と同じ灰色で表示され、再開できるノードと終わったノードを見分けられない。

# Scope / Non-goals

## Scope

- Workspace ツリーの行が表す状態色の意味と、その色を決める分類規則。
- 親行（Sequence / Fanout）の状態集約規則。
- Workspace ツリー取得および Workspace ノード詳細取得の外部インターフェースが返す状態表現。
- 詳細ペインのノード状態アイコンの色と形状。
- 到達不能な `interrupted` 状態の除去。

## Non-goals

- ツリー行のアイコン形状（Bot / Terminal / ListTree / GitFork）の変更。
- local API と CLI の応答。Workspace ツリーを返していないため対象外である。
- Workflow 実行そのものの状態遷移規則の変更。`ExecutionStatus` および Node Attempt の実行状態は対象外である。
- recovery fence および completion signal の生成規則の変更。
- approve / retry / stop / resume / abort / archive の操作可否判定の変更。
- Workflow root ノードをツリーへ表示すること。引き続き子へ平坦化する。
- 過去 attempt 履歴（`pastAttempts`）の表示規則の変更。

# Requirements

- R-001: Workspace ツリーの行が表すノード状態の色は 4 つとし、それぞれの意味を次に固定する。緑は完了ではなく「動いていない」を表す。

  | 色 | 意味 |
  |---|---|
  | 青 | 実行中 |
  | 黄 | 介入が必要 |
  | 赤 | 失敗 |
  | 緑 | 動いていない |

- R-002: leaf ノードの分類は次の順で評価し、最初に当てはまったものを採る。
  1. recovery fence を持つ → 赤
  2. 状態が `failed` → 赤
  3. approval 待ち → 黄
  4. 状態が `running` かつ Stop 信号だけを受領（Submit 未受領）→ 黄
  5. 状態が `running` → 青
  6. 状態が `paused` / `completed` / `aborted` → 緑

  `paused` かつ Stop 信号受領は 4 に当たらず 6 で緑になる。`paused` かつ recovery fence ありは 1 で赤になる。
- R-003: 親行（Sequence / Fanout）は、自分自身の分類結果と配下の子の分類結果を合わせ、赤 > 黄 > 青 > 緑 の重大度順で最も重いものを採る。Sequence と Fanout で規則を分けない。
- R-004: Workspace ツリー取得の外部インターフェースが返すノード状態は R-001 の 4 分類だけとし、`running` / `paused` / `failed` / `waiting` / `aborted` / `completed` といった詳細な状態値をツリーに返さない。分類の判定は呼び出し側に委ねない。
- R-005: Workspace ノード詳細取得の外部インターフェースは、詳細な状態と R-001 の 4 分類の両方を返す。詳細ペインが 4 分類を自分で導出する必要がない。
- R-006: 詳細ペインのノード状態アイコンは、色を R-001 の 4 分類に従わせ、`paused` / `completed` / `aborted` などの詳細はアイコン形状で区別できるようにする。
- R-007: `interrupted` は Workspace ツリーおよび Workspace ノード詳細のいずれの状態表現にも現れない。
- R-008: Workspace ツリー行のアイコンを pulse させる対象は、現在 pulse している状態（`running` / `waiting`）を新しい分類へ移した結果として、青と黄の 2 分類とする。赤と緑は pulse しない。詳細ペインのノード状態アイコンは、現在どおり pulse させない。
- R-009: approve / retry / stop / resume / abort / archive の操作可否、および resume 不能理由の内容は、この変更の前後で変わらない。
- R-010: local API と CLI の応答は、この変更の前後で変わらない。

# Assumptions / Open Questions

なし。
