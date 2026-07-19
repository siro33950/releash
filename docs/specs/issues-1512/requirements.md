# Context

- 変更要求の正本: [#1512 Workspace treeには実行開始済みNodeだけを表示する](https://github.com/siro33950/releash/issues/1512)（milestone [統一 Node モデル](https://github.com/siro33950/releash/milestone/86)）
- 入力文書:
  - [#1454](https://github.com/siro33950/releash/issues/1454) Workspace UIをNode中心の再帰ツリーに統一（closed。現行のqueued表示を意図的に導入した先行実装）
  - [#1468](https://github.com/siro33950/releash/issues/1468) 実行木UIの統一と文法正本化・最終cleanup（後続。詳細設計にqueued placeholder前提の記述が残る）
  - [#1329](https://github.com/siro33950/releash/issues/1329) / [#1331](https://github.com/siro33950/releash/issues/1331) / [#1461](https://github.com/siro33950/releash/issues/1461)（closed。fanout・read model・新構文の器の先行移行）
  - milestone [#82 Workflow Engine 新モデル移行](https://github.com/siro33950/releash/milestone/82)（closed）/ milestone #85（Session delegate + worktree隔離。入力の`issues/85`はこのmilestoneを指すとユーザー確認済み）
  - `specs/unified-node-model/decisions.md` / `syntax.md` / `examples/`（統一Nodeモデルの正本）
  - `docs/specs/milestone-82/design.md` / `goal-14-issue-1454.md` / `goal-common.md` / `plan.md`
  - `docs/workflow-yaml-syntax.md` / `docs/workflow-engine-evolution-plan.md` / `docs/workflow-engine-model-boundary.md` / `docs/architecture/` / `docs/domain-model/current-state.md` / `docs/examples/`
  - 既存実装: `src-tauri/src/usecase/workflow/workspace_tree.rs` ほか（詳細はCurrent Behavior）
- 確定済みの背景:
  - `specs/unified-node-model/decisions.md` §実行木は「WorkflowDefinitionはテンプレートであり木ではない。実行木には展開結果（実際に起きたこと）だけが載る」と定義している。本変更はWorkspace projectionをこの正本へ合わせる契約変更である。
  - 現行の「未開始Nodeはqueued表示」は#1454で意図的に導入され、`docs/specs/milestone-82/design.md` §13（行385）と`goal-14-issue-1454.md`（行20・行42）に正本として記載されている。本変更はこの正本記載の更新を含む。
  - #1468の詳細設計（queued rows / queued composite subtree等）は本契約確定前の記述であり、#1512の契約が優先する。#1468の再帰実行木projection（Sequence / Ref）でも本契約（definition child / expected child slotから未開始Nodeを再生成しない）を維持する。
  - 現行domainの`NodeExecutionStatus`は`Running / WaitingApproval / Succeeded / Failed / Aborted`のみで、`NotStarted` / `Skipped`は存在しない。未実行Nodeは、durableな`NodeStarted`イベントと`NodeExecution`が存在しないdefinition-only stateとして判別できる。
  - 表示可否の決定はRust-owned projectionが所有する（プロジェクト方針: ロジックはRustに置き、frontendはmirrorに徹する）。

# Outcome

- 対象者: Workspace treeでworkflow実行を監視する開発者。
- 現在の問題: 一度も実行されていないWorkflow Node（条件分岐で使われない候補、未開始のfanout expected slotを含む）が`queued`としてWorkspaceへ並ぶため、実際に発生した作業とWorkflow定義上の候補が混在し、実行中・確認待ち・完了済みのNodeを監視しにくい。
- 変更後の状態: Workspace treeは実行開始済み（durableな`NodeStarted`に由来する実在の`NodeExecution`を持つ）Nodeだけを表示し、Workspaceが「実際に発生した作業状態を扱うsurface」になる。Workflow定義のpreviewではなくなる。

# Current Behavior

- `src-tauri/src/usecase/workflow/workspace_tree.rs::project_workflow_children`（行702–822）: 実行済みNodeをevent順に投影した後、`NodeExecution`が一件も存在しない定義Nodeをdeclaration orderで末尾へ追加し（行789–819）、`node_execution = None`として`project_workflow_node`が`queued`表示へ変換する（行1012）。
- 同`project_fanout`（行832–967）+ `expected_fanout_slots`（行969–993）: 親fanoutがactiveな間、定義とitemsから期待slotを合成し、未実行slotを`queued` placeholderとして表示する（行897–938）。ArtifactField itemsが未確定の場合はchildごとに1つのplaceholderを合成する。
- queued placeholderと最初の実行occurrenceは同一のopaque IDを持つ仕様であり、テストで固定されている。
- definition-only Nodeも`project_workflow_node`（行1054–1098）でNode detail indexへ登録され、選択すると`get_workspace_node_detail`がqueued detailを返す。`NodeContentView.tsx`（行89–91）は`status === "queued"`で「This session has not started yet.」を表示する。
- `src/hooks/useWorkspaceTreeNodes.ts`はbackend snapshotを無加工で保持し、`src/components/workspace/WorkspaceList.tsx`は渡されたchildrenを無条件に描画する（frontend独自の表示可否判定はない）。
- `preferred_node_id`（行1315–1323）は全leaf（queued含む）から`running / waiting`を優先して選び、なければ先頭leafを返す。表示対象が皆無なら`None`。
- Node選択状態はfrontendのメモリ上（`src/App.tsx`の`centerStateByWorktree`、行44–63）のみで保持され、永続化されていない。選択Nodeがdetailから消えると`onNodeMissing`経由で未選択（`resolvedEmpty`）へ落とす（行242–259）。
- 再現手順と実際の出力: `src-tauri/`で次を実行すると、現在のqueued挙動を固定する既存テスト3件を含む21件が全て成功する（2026-07-19確認: `21 passed; 0 failed`）。

  ```bash
  cargo test -p releash --lib usecase::workflow::workspace_tree::tests
  ```

  - `declaration_order_and_queued_nodes_come_from_execution_snapshot`: 実行イベント0件のworkflowで全定義Nodeが`queued`としてdeclaration orderで並ぶこと。
  - `execution_occurrences_follow_event_order_and_unstarted_nodes_remain_queued`: 定義`[A,B,C,D]`・実行`A→B→A→C`で、4 occurrenceの後に未実行`D`が`queued`で並ぶこと。
  - `artifact_item_fanout_keeps_queued_child_id_for_its_first_occurrence`: queued placeholderのopaque IDが最初の実行childと一致すること。

# Scope / Non-goals

## Scope（変更するもの）

- Rust Workspace projection（`workspace_tree.rs`）の表示契約: definition-only Nodeの`queued`追加、fanout expected slotからのplaceholder合成、definition-only recordのdetail / session lookup indexへの登録を廃止し、実在するexecution occurrenceだけからstatus・updated time・children集約・`preferredNodeId`を導出する。
- queued placeholderと最初の実行occurrenceで同一IDを維持するという既存仕様・テストの廃止（placeholder自体を外部へ公開しない）。
- 保持中の選択Node IDが新snapshotの表示対象に存在しない場合のfallback挙動。
- frontend（`useWorkspaceTreeNodes` / `WorkspaceList`）: backend snapshotのmirrorを維持し、childrenが空のWorkflow branchを正常な状態として表示できるようにする。
- 正本文書の更新: `docs/specs/milestone-82/design.md` §13と`goal-14-issue-1454.md`の「未開始Nodeはqueued」要件を、実行開始済みNode限定の本契約へ更新する（`specs/unified-node-model/decisions.md`「実際に起きたことだけが実行木に載る」を受け入れ根拠として参照する）。
- 本Issueが指定する既存テスト3件の新契約への置き換えと、新契約（条件分岐未選択の非表示・空childrenのWorkflow・開始済みfanoutのactual child限定・stale selection fallback）の固定。

## Non-goals（変更しないもの）

- `NodeExecutionStatus`やWorkflow domainへの`NotStarted` / `Skipped`の追加。
- Workflow定義全体を閲覧・編集する新しいUI（実行前定義の確認機能はWorkspace treeの責務に含めない）。
- `queued` status語彙の他用途からの一括削除。
- 実行済み・完了済み・失敗・中断NodeのWorkspace履歴からの削除。
- 未実行Nodeを例外的に表示するpin等の新概念の追加。
- 選択Node IDの永続化機構の新設（fallback要求は保持中の選択に対するもの。ユーザー確認済み）。
- CLI / Local API / event logのWorkflowExecution / NodeExecution語彙・識別規則の変更。

# Requirements

- R-001: Workflow executionのbranch行は、Workflow executionが存在する間は表示される。Nodeが一件も開始されていない場合、branchは表示されchildrenは空である。
- R-002: Node leaf行は、durableな`NodeStarted`に由来する実在の`NodeExecution`が存在する場合だけ表示される。定義にのみ存在するNode（未実行・条件分岐で未選択・到達しなかったNode）は、実行中もWorkflow完了後も表示されない。
- R-003: fanout等の合成子branchは、対応する実在の`NodeExecution`がある場合だけ表示される。未開始fanoutはbranchごと表示されない。#1468で追加されるSequence / Refも同じ契約に従う。
- R-004: fanout childは実在するchild `NodeExecution`だけが表示され、定義・expected slot・items（literal / artifactのどちらでも）からplaceholderは合成されない。
- R-005: 開始済みNodeは`Running / WaitingApproval / Succeeded / Failed / Aborted`のいずれの状態でも従来どおり表示され続ける（互換性要件）。
- R-006: retry / loopで同じ定義Nodeが複数回開始された場合は、従来どおり実行occurrenceごとに別行となり、`NodeStarted`のevent順を維持し、相互に異なるopaque IDを持つ。実行済みoccurrenceのopaque ID・event順という既存保証は維持する（互換性要件）。一方、queued placeholderと最初の実行occurrenceで同一IDを維持する既存仕様は廃止する。
- R-007: Nodeが実際に開始された時点でlive snapshotへ行が追加され、reload後も同じ行が表示される。live更新とreload後のWorkspace treeは一致する。
- R-008: `preferredNodeId`は表示対象の実在Nodeだけを指す。表示対象がなければ`null`になる。
- R-009: Node detail / session lookupは未実行Node（definition-only record）を参照・返却しない。
- R-010: 保持中の選択Node IDが新snapshotに存在しない場合、表示対象の`preferredNodeId`へ安全にfallbackし、表示対象がなければ未選択になる。
- R-011: 表示対象の決定はRust projectionが所有する。frontendに`status === "queued"`等を使った除外判定やWorkflow定義・分岐結果を使った表示可否ロジックを追加しない（安全性・アーキテクチャ要件）。
- R-012: `docs/specs/milestone-82/design.md` §13と`docs/specs/milestone-82/goal-14-issue-1454.md`の「未開始Nodeはqueued」記載が、本契約（実行開始済みNode限定）へ更新されている。

# Assumptions / Open Questions

## Assumptions

- 入力documentsの`https://github.com/siro33950/releash/issues/85`はmilestone #85（Session delegate + worktree隔離）を指す参照であり、GitHub Issue(PR) #85（セキュリティ修正）は本件と無関係（ユーザー確認済み 2026-07-19）。
- R-010のfallback要求は保持中の選択Node IDに対するものであり、選択の永続化機構の新設は含まない（ユーザー確認済み 2026-07-19）。

## Open Questions

なし。
