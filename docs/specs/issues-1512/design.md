# Design

対象: `docs/specs/issues-1512/requirements.md`（R-001〜R-012）/ `docs/specs/issues-1512/behavior.md`（B-001〜B-013）

## The actual design

### Architecture

#### 表示可否の決定はRust projectionの1箇所に閉じる

表示契約の変更は`src-tauri/src/usecase/workflow/workspace_tree.rs`のprojection関数群だけで行う。queued行を生成している経路は次の3つであり、すべて削除する（R-002 / R-003 / R-004 / R-011、B-001〜B-006）。

1. `project_workflow_children`の定義Node追記ブロック（現行789–819行）: `NodeExecution`が存在しない定義Nodeをdeclaration orderで末尾へ追加している。削除により、childrenは`node_executions`（`fanout_parent`なし）の`NodeStarted` append順の投影だけになる。定義由来のフィルタ用`child_names`集合と`executed_node_names`集合も不要になる。
2. `project_fanout`の`keep_queued_slots`ブロック（現行897–938行）と`expected_fanout_slots`（現行969–993行）: 定義・items（Literal / ArtifactField）から期待slotを合成している。両方削除し、childrenは`actual_children`（実在するchild `NodeExecution`）の投影だけにする。
3. 定義-onlyのfanout branch投影: 1.の削除に伴い、`project_fanout`が`parent: None`で呼ばれる経路が消える。未開始fanoutはbranchごと投影されない（B-004）。

frontend（`useWorkspaceTreeNodes` / `WorkspaceList`）はbackend snapshotのmirrorを維持し、表示可否判定を追加しない（R-011、B-012）。`WorkspaceBranchRow`は既に`item.children.map(...)`で空配列を正常に描画するため、空children表示のための描画変更は不要であり、テストで固定するだけにする。

#### projection関数のシグネチャ整理

definition-only経路の削除により、次のOptionを外す。

- `project_workflow_node`の`node_execution: Option<&NodeExecution>` → `&NodeExecution`。`RepresentativeStatus::Queued`へのfallback（現行1012行）と`summary.started_at`への`updated_at` fallback（現行1022行）は到達不能になるため削除する。
- `project_fanout`の`parent: Option<&NodeExecution>` → `&NodeExecution`。status集約の`unwrap_or(RepresentativeStatus::Queued)`（現行944行）はparent statusが常に供給されるため到達不能になる。

`project_workflow_children`が`definition`を参照する残りの用途は、fanout childのkind解決等の表示補助に限られる。definitionは「何を表示するか」の決定には使わない。この原則は#1468で追加されるSequence / Refの再帰projectionにも適用する: definition childやexpected child slotから未開始Nodeの行を再生成しない（R-003）。

#### stale selectionはArchive専用のbackend-owned reconciliationで判定する

Archive済みWorkflowのNode detail / session lookupは参照可能なままなので、detail lookupのmissingだけでは「detailは有効だが新snapshotには表示されない」選択を検出できない。Archive成功後だけ、専用`WorkspaceTreeQueryService`がmanaged worktree、workspace session、archive record、`WorkflowQueryService`のread依存を用い、`WorkspaceProjectionTarget::Snapshot`を一度生成する。同じprojectionからsnapshot membershipを判定し、`{ snapshot, reconciliation: { selectionInSnapshot } }`を返す（R-010 / R-011、B-011 / B-012）。Command側`WorkflowUsecase`はこのreadを公開せず、Archive / Restore mutationだけを所有する。

frontendで選択IDを常時loaderへ渡したり、snapshotのleafを走査したりしない。`useWorkspaceTreeNodes`がArchive成功時だけ`{ worktreePath, selectedNodeId, reconciliationGeneration }`をrequest contextとして登録し、combined reconciliation readを直ちに試す。通常の初期load、event / websocket、dropdown、Archive以外のactionによるrefreshは`list_workspace_worktree_nodes`を使う。最初のcombined readが失敗した場合は既存snapshot・選択・request contextを保持し、次に既存契機でrefreshされたとき、同じcontextが有効な場合だけcombined readを再試行する。自動retry timerは追加しない。

各refreshには既存の単調増加`refreshSeq`を割り当てる。最新sequenceで、現在のgeneration・worktree・selectionと一致するresponseだけを受理し、snapshotと`{ refreshSeq, requestContext, selectionInSnapshot }`をhookの一つのstate commitで公開する。成功、選択変更、worktree変更で必要contextを解除するため、遅延response、後続refreshに追い越されたresponse、選択のA→B→Aで旧generationに属するresponseはno-opになる。

`selectionInSnapshot=false`の受理eventは、同じcommitで適用された`snapshot.preferredNodeId`を唯一のfallback候補として扱う。`WorkspaceList`は現在contextとの一致をeffect直前に再確認し、`refreshSeq`ごとに一度だけAppへ当該Nodeの失効を通知する。Appはcallback到着時にもfunctional updateで現在選択が同じNode IDか確認して`awaitingInitial`へ戻す。既存auto-select effectが同snapshotの`preferredNodeId`を選び、`null`なら未選択を維持する。`selectionInSnapshot=true`なら通知せず現在選択を維持する。Archive以外のdetail-missingは、従来どおり`NodeContentView`の`onNodeMissing`から同じ`awaitingInitial` / auto-select経路へ入る。

#### 正本文書の更新

R-012 / B-013として次を更新する。

- `docs/specs/milestone-82/design.md` §13（現行385行）:「未開始Nodeはqueuedで表示する」を、実行開始済み（durableな`NodeStarted`に由来する`NodeExecution`が存在する）Nodeだけを表示する契約へ書き換える。
- `docs/specs/milestone-82/goal-14-issue-1454.md`（現行20行「未開始Nodeはqueued」・43行「queued Node」）: 同様に本契約へ書き換える。

両更新とも、受け入れ根拠として`specs/unified-node-model/decisions.md` §実行木の「WorkflowDefinitionはテンプレートであり木ではない。実行木には展開結果（実際に起きたこと）だけが載る」を参照する。

### Interface

- 通常refreshは既存`list_workspace_worktree_nodes(worktreePath) -> WorkspaceTreeSnapshotDto`を使う。`children: Vec<WorkspaceTreeItemDto>`が空になりうることは既に型上許容されている（R-001、B-001 / B-006）。
- Archive後の必要contextが有効な間だけ、Tauri command `get_workspace_tree_selection_reconciliation(worktreePath, selectedNodeId) -> WorkspaceTreeSelectionSnapshotDto`を使う。controllerは独立した`State<Arc<WorkspaceTreeQueryService>>`を呼ぶ。
- responseは`{ snapshot: WorkspaceTreeSnapshotDto, reconciliation: { selectionInSnapshot: boolean } }`である。`selectedNodeId`はrequestとfrontend request contextだけ、`preferredNodeId`は同梱snapshotだけを正とし、responseのreconciliationへ複製しない。membershipとfallback候補は必ず同じprojectionに対応する。
- `get_workspace_node_detail` / `get_workspace_session_node_id`等の既存command、およびWebSocket protocolの型・シグネチャは変更しない。
- opaque ID規則は変更しない: `workflow_node_occurrence_key` / `fanout_branch_occurrence_key` / `fanout_child_occurrence_key` / `fanout_dynamic_child_occurrence_key` / `opaque_workflow_node_id`は現行のまま。実行済みoccurrenceのID・event順の既存保証はこれで維持される（R-006、B-002）。queued placeholderとの同一ID仕様はplaceholder自体が公開されなくなることで消滅する。`fanout_dynamic_child_occurrence_key`のplaceholder前提コメント（現行1153–1155行）は実態に合わせて更新する。
- `preferred_node_id`（現行1315–1323行）はコード変更しない。queued行が投影されなくなることで、走査対象が表示対象の実在leafだけになり、leafが皆無なら`None`を返す既存実装がそのままR-008 / B-009を満たす。

### Data Model

- `WorkspaceProjectionIndex`（`records` / `session_node_ids`）の構造は変更しない。登録はすべて`project_workflow_node`内で行われるため、definition-only行の投影廃止により、未実行Nodeのdetail record / session lookup entryは自然に登録されなくなる（R-009、B-010）。
- `WorkspaceSelectionReconciliationDto`が持つ独立したread factは`selection_in_snapshot`だけである。fallbackは同梱`WorkspaceTreeSnapshotDto.preferred_node_id`から得る。
- frontendのArchive再調停状態は世代付きrequest contextだけを保持し、server requestのechoやsnapshot preferred値の複製を持たない。受理eventは同じstate commitのsnapshotと`refreshSeq`で相関する。
- domain（`NodeExecutionStatus`等）は変更しない（Non-goal）。

### Database

該当なし（イベントストア・永続化スキーマの変更はない。live snapshotとreload後のsnapshotは同一のprojection関数がイベントストア由来の同一stateから導出するため、一致はprojectionの純粋性で保証される。R-007、B-008）。

### UI/UX

- Workspace tree: 未開始Node・未開始fanout・fanout placeholderの行が表示されなくなる。Node未開始のWorkflowはbranch行のみ（children空）で表示される。開始済みNodeは`Running / WaitingApproval / Succeeded / Failed / Aborted`の全statusで従来どおり表示される（R-005、B-007）。
- 選択中のNodeが新snapshotの表示対象から消えた場合、選択は`preferredNodeId`のNodeへ移り、表示対象がなければ中央は未選択状態（"Select a Node from the Workspace tree."）になる（B-011）。
- Archive後のreconciliation readが一時失敗した間は、古いsnapshotと選択を表示したままにする。次の既存refreshで成功したauthoritative snapshotを受理してからfallbackする。
- `NodeContentView`の`status === "queued"`分岐（現行89–91行）は変更しない。workflow nodeのdetailは常に実行済みstatusを持つため到達しなくなるが、`queued`語彙の一括削除はNon-goalであり、frontend変更のScopeは`useWorkspaceTreeNodes` / `WorkspaceList`に限る。

### Algorithm

- `project_workflow_children`（変更後）: `node_executions`のうち`fanout_parent`なしのものを`NodeStarted` append順に走査し、occurrence番号を採番して`project_fanout` / `project_workflow_node`へ投影するだけになる。ソート・定義追記は行わない。
- `project_fanout`（変更後）: `actual_children`だけをsemantic key採番（現行851–895行のまま）で投影する。branch statusは`aggregate_representative_statuses(children ∪ parent status)`で、childrenが空でもparent statusが常に供給される。`updated_at`はchildren / parentのmaxで、既存ロジックのまま。
- frontend refresh: Archive再調停contextがなければlist commandを呼ぶ。contextがあればcombined commandを呼び、失敗時はcontextを保持する。最新`refreshSeq`かつ現在のgeneration / worktree / selectionに一致する成功だけがsnapshotとreconciliation eventを同時にcommitし、contextを解除する。選択またはworktree変更もgenerationを進めて解除する。
- frontend notification: commit後のeffectでeventのsequence / generation / worktree / selectionを再確認する。`selectionInSnapshot=false`かつ未通知の`refreshSeq`だけをAppへ通知し、fallback値はcommit済みsnapshotの`preferredNodeId`だけを使う。
- テスト設計（検証対応表に従う。Rust側は`workspace_tree.rs`の`#[cfg(test)] mod tests`、frontend側は対象ファイル隣の`*.test.ts(x)`）:
  - 廃止仕様を固定する既存テスト3件（`declaration_order_and_queued_nodes_come_from_execution_snapshot` / `execution_occurrences_follow_event_order_and_unstarted_nodes_remain_queued` / `artifact_item_fanout_keeps_queued_child_id_for_its_first_occurrence`）を新契約のテストへ置き換える（B-001 / B-002 / B-005相当）。
  - 追加: 条件分岐で未開始Nodeを持つ完了済みexecution（B-003）、未開始fanout非表示（B-004）、ArtifactField items未確定の開始済みfanout空children（B-006）、5 status全表示（B-007）、`NodeStarted`追記前後のsnapshot比較（B-008）、`preferredNodeId`の実在leaf限定と`None`（B-009）、detail / session lookupの未実行Node不在（B-010）。
  - 既存テストのうちqueued slot前提の箇所（`literal_fanout_items_expand_to_distinct_leaf_nodes_in_item_then_child_order`等、置き換え対象3件以外）も新契約の期待値へ更新する。
  - frontend: stale selection→`preferredNodeId` fallbackと`null`時の未選択（B-011）、空childrenのWorkflow branchを含むsnapshotの無加工描画（B-012）。
  - frontend競合: Archive直後read失敗→次refresh成功、`selectionInSnapshot=true`、選択移動、worktree変更、後続refresh、A→B→A、重複render、Appへの遅延旧Node通知を固定する。通常選択変更とoccurrence更新がreconciliation / closed sessions / workflow historyのfull refetchを起こさないことも計数する。

### Infra

該当なし。

## Alternatives Considered

- **`NotStarted` / `Skipped` statusのdomain追加**: 未実行Nodeをstatus付きで表現する案。requirementsのNon-goalであり、未実行Nodeは`NodeExecution`不在というdefinition-only stateとして判別できるため採らない。
- **frontendでのqueued行フィルタリング**: backendはqueued行を返し続け、frontendで除外する案。表示可否の所有者がfrontendへ移りR-011に反するため採らない。
- **stale selection fallbackでのfrontend snapshot走査**: `WorkspaceList`がleaf集合に選択IDが含まれるか自前で走査する案。表示可否・membershipの所有者がfrontendへ移りR-011に反するため採らない。採用案は専用QueryServiceが同じbackend projectionからmembershipを導出する。
- **選択中の全refreshでreconciliation commandを使う案**: Archiveと無関係なrefreshでもmembershipを再計算し、Node選択だけでtree / closed sessions / historyのfull refetchを起こすため採らない。Archive後の未完了contextがある期間だけcombined commandを使う。
- **Archive handler内の一回だけのread**: 単純だが一時失敗後にB-011を完了できないため採らない。必要contextを成功まで保持し、次の既存refreshで再試行する。
- **responseへ`selectedNodeId` / `preferredNodeId`を複製する案**: request echoとsnapshot cloneであり、不一致という不正状態を表現できるため採らない。request contextと同梱snapshotを唯一値にする。
- **`preferredNodeId`をAppへ渡して直接選択する案**: Appへsnapshot値を別配管すると相関対象が増えるため採らない。Appには失効したNode IDだけを通知し、同じsnapshotを表示するWorkspaceListの既存auto-select経路を再利用する。

## Cross-cutting concerns

- **性能**: queued行・expected slotの合成と、そのdetail index登録が削除されるため、projectionの計算量・snapshotサイズは縮小方向。reconciliation projectionはArchive後の未完了contextだけで実行し、通常refreshや選択変更には追加しない。
- **互換性**: 実行済みoccurrenceのopaque ID・event順・status表示・detail取得は既存規則のまま維持する（R-005 / R-006）。CLI / Local API / event logの語彙・識別規則は変更しない。
- **アーキテクチャ**: 表示契約とmembershipはRust projectionに閉じる。新規read modelは専用`WorkspaceTreeQueryService`が所有し、controllerは独立したTauri Stateを読む。Command側`WorkflowUsecase`はArchive / Restore mutationを維持する。frontendはsnapshotのmirrorと世代付きrequest相関だけを担い、WebSocket protocol・domain・永続化は変更しない。

## Risks

- queued placeholderを前提にしたテスト・補助関数の残存: 置き換え対象として指定された3件以外にも、fanout系テストがexpected slotを前提にしている（`literal_fanout_items_expand_to_distinct_leaf_nodes_in_item_then_child_order`等）。テスト更新範囲は3件に限らないことを実装時に前提とする。
- 一時read failure: Archive成功後も必要contextを解除せず、次の既存refreshでだけ再試行する。timer retryを追加しないため、外部refreshが来るまで古いsnapshot / selectionを維持する。
- 非同期競合: 遅延responseや選択のABAが現在選択を失効させる危険がある。`refreshSeq`、generation、worktree、selected Node IDをすべて照合し、通知済みsequenceを記録する。Appもcallback到着時に現在Node IDを再確認する。
- fallback後の再選択タイミング: `awaitingInitial`復帰方式では、表示対象が皆無で未選択になった後、新たにNodeが開始されると初回auto-selectと同じ経路で選択される。これは初回表示時の既存挙動と同一経路の再利用である。
- `NodeContentView`のqueued分岐・`RepresentativeStatus::Queued`・frontend status語彙の`"queued"`は本変更でworkflow node経路から到達しなくなるが、削除はNon-goal（`queued`語彙の一括削除をしない）のため残す。dead code整理は別Issueの候補として実装時に報告する。
