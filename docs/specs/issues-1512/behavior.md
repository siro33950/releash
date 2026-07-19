# Behavior — issues-1512 Workspace treeには実行開始済みNodeだけを表示する

対象Requirements: `docs/specs/issues-1512/requirements.md`

## B-001: Node未開始のWorkflow branchは空childrenで表示される

GIVEN Workflow定義に複数のNodeを持つWorkflow executionが存在し、`NodeStarted`イベントが一件も記録されていない
WHEN Workspace treeのsnapshotを取得する
THEN そのWorkflow executionのbranch行は表示される
AND branchのchildrenは空である
AND 定義上のNodeはleaf行として一件も表示されない

最小再現入力: 実行イベント0件のWorkflow execution（既存テスト`declaration_order_and_queued_nodes_come_from_execution_snapshot`と同じ入力）。現在の実際の結果は、全定義Nodeが`queued`としてdeclaration orderで並ぶ（2026-07-19に`cargo test -p releash --lib usecase::workflow::workspace_tree::tests`で確認、21 passed）。

## B-002: 実行済みoccurrenceだけがevent順で表示され、未実行定義Nodeは表示されない

GIVEN Workflow定義が`[A, B, C, D]`のWorkflow executionが存在し、`NodeStarted`が`A → B → A → C`の順に記録されている
WHEN Workspace treeのsnapshotを取得する
THEN Workflow branchのchildrenは`A, B, A, C`の4 occurrenceだけが`NodeStarted`のevent順で並ぶ
AND `D`のleaf行は表示されない
AND `A`の2つのoccurrenceは別行であり、相互に異なるopaque IDを持つ

最小再現入力: 既存テスト`execution_occurrences_follow_event_order_and_unstarted_nodes_remain_queued`と同じ入力。現在の実際の結果は、4 occurrenceの後に未実行`D`が`queued`で並ぶ。

## B-003: 条件分岐で未選択のNodeはWorkflow完了後も表示されない

GIVEN 条件分岐により一部の定義Nodeが一度も開始されないままWorkflow executionが完了状態（Completed / Failed / Aborted のいずれか）に達している
WHEN Workspace treeのsnapshotを取得する
THEN 開始されなかった定義NodeのleafおよびbranchはWorkspace treeに表示されない
AND 開始済みNodeのoccurrence行は表示され続ける

## B-004: 未開始fanoutはbranchごと表示されない

GIVEN Workflow定義にfanout Nodeが含まれ、そのfanoutに対応する`NodeExecution`が存在しないWorkflow executionが存在する
WHEN Workspace treeのsnapshotを取得する
THEN そのfanoutのbranch行はWorkspace treeに表示されない
AND fanout childのplaceholder行も表示されない

## B-005: 開始済みfanoutは実在するchild NodeExecutionだけを表示する（literal items）

GIVEN literal itemsを持つfanoutが開始済みで、一部のitem×childの組だけがchild `NodeExecution`として開始されている
WHEN Workspace treeのsnapshotを取得する
THEN fanout branchのchildrenは実在するchild `NodeExecution`の行だけである
AND 未開始のitem×childの組に対するplaceholder行は合成されない

最小再現入力: 既存テスト`artifact_item_fanout_keeps_queued_child_id_for_its_first_occurrence`が固定する「queued placeholderのopaque IDが最初の実行childと一致する」仕様は本契約で廃止され、placeholder自体が公開されない。

## B-006: 開始済みfanoutは実在するchild NodeExecutionだけを表示する（artifact items未確定）

GIVEN ArtifactField itemsを持つfanoutが開始済みで、itemsが未確定のためchild `NodeExecution`が一件も存在しない
WHEN Workspace treeのsnapshotを取得する
THEN fanout branchは表示され、childrenは空である
AND childごとのplaceholder行は合成されない

## B-007: 開始済みNodeは全statusで表示され続ける

GIVEN `Running / WaitingApproval / Succeeded / Failed / Aborted`の各statusを持つ開始済み`NodeExecution`がWorkflow executionに存在する
WHEN Workspace treeのsnapshotを取得する
THEN 5つのstatusすべての`NodeExecution`がleaf行として表示される
AND 各行のstatus表示は従来と同じstatusを示す

## B-008: Node開始時のlive反映とreload後の一致

GIVEN Workflow executionが実行中で、あるNodeがまだ開始されていない
WHEN そのNodeが実際に開始される（durableな`NodeStarted`が記録される）
THEN live snapshotにそのNodeのleaf行が追加される
AND アプリreload後に取得したWorkspace treeにも同じ行が表示される
AND live更新後のtreeとreload後のtreeは一致する

## B-009: preferredNodeIdは表示対象の実在Nodeだけを指す

GIVEN Workspace treeのsnapshotが存在する
WHEN `preferredNodeId`を取得する
THEN `preferredNodeId`は表示対象のleaf行に存在するNode IDである
AND 表示対象のleaf行が一件もない場合（Node未開始のWorkflowのみ等）、`preferredNodeId`は`null`である

## B-010: Node detail / session lookupは未実行Nodeを返さない

GIVEN 定義にのみ存在し`NodeExecution`を持たないNodeがあるWorkflow executionが存在する
WHEN Node detailおよびsession lookupを照会する
THEN 未実行Nodeに対応するdetail recordは登録されておらず、参照・返却されない
AND 開始済みNodeのdetailは従来どおり取得できる

## B-011: stale selectionはpreferredNodeIdへfallbackする

GIVEN frontendが保持中の選択Node IDを持ち、新しく取得したsnapshotの表示対象にそのIDが存在しない
WHEN snapshotが更新される
THEN 選択はsnapshotの`preferredNodeId`が指すNodeへfallbackする
AND `preferredNodeId`が`null`（表示対象なし）の場合、選択は未選択状態になる

## B-012: frontendはbackend snapshotのmirrorに徹する

GIVEN backendがchildrenが空のWorkflow branchを含むsnapshotを返す
WHEN frontend（`useWorkspaceTreeNodes` / `WorkspaceList`）がsnapshotを描画する
THEN frontendはsnapshotの内容を加工せずそのまま描画し、空childrenのWorkflow branchを正常な状態として表示する
AND frontendに`status === "queued"`等のstatus値やWorkflow定義・分岐結果に基づく表示可否の除外判定は存在しない

## B-013: 正本文書が本契約へ更新されている

GIVEN `docs/specs/milestone-82/design.md` §13と`docs/specs/milestone-82/goal-14-issue-1454.md`に「未開始Nodeはqueued」の記載が存在する
WHEN 本変更を適用する
THEN 両文書の該当記載が「実行開始済みNode限定」の本契約へ更新されている
AND 更新後の記載は`specs/unified-node-model/decisions.md`の「実際に起きたことだけが実行木に載る」を受け入れ根拠として参照している

## 要件IDと検証方法の対応表

| Requirement ID | Behavior ID | Verification Method |
| --- | --- | --- |
| R-001 | B-001 | 実行イベント0件のWorkflow executionを入力に`src-tauri/`で`cargo test -p releash --lib usecase::workflow::workspace_tree::tests`を実行し、projectionの出力でWorkflow branch行が存在しchildrenが空配列であること、定義Nodeのleaf行が0件であることを確認する。 |
| R-002 | B-002, B-003 | 定義`[A,B,C,D]`・実行`A→B→A→C`のsnapshotをprojectionテストで検証し、childrenが4 occurrence（event順）のみで`D`の行がないことを確認する。条件分岐で未開始のNodeを持つ完了済み（Completed / Failed / Aborted）executionでも、未開始Nodeの行がないことをテストで確認する。 |
| R-003 | B-004 | fanout定義があり対応する`NodeExecution`がないexecutionのprojection出力に、fanout branch行およびそのchild行が0件であることをテストで確認する。 |
| R-004 | B-005, B-006 | literal items fanoutで一部childのみ開始したsnapshot、およびArtifactField items未確定で開始済みのfanoutのsnapshotをprojectionテストで検証し、childrenが実在するchild `NodeExecution`の行だけであり、placeholder行が合成されないことを確認する。 |
| R-005 | B-007 | `Running / WaitingApproval / Succeeded / Failed / Aborted`の`NodeExecution`を各1件持つexecutionのprojection出力に、5行すべてが従来のstatus表示で存在することをテストで確認する。 |
| R-006 | B-002 | `A→B→A→C`のsnapshotで`A`の2 occurrenceが別行・event順・相互に異なるopaque IDであることをテストで確認する。併せて、queued placeholderとの同一ID仕様を固定していた既存テスト（`artifact_item_fanout_keeps_queued_child_id_for_its_first_occurrence`等3件）が新契約のテストへ置き換えられていることを確認する。 |
| R-007 | B-008 | 実行中のexecutionで`NodeStarted`を追記した前後のlive snapshot、およびイベントストアからの再構築（reload相当）snapshotを比較し、新Nodeの行が両方に存在しtreeが一致することをテストで確認する。 |
| R-008 | B-009 | 開始済みNodeを持つsnapshotで`preferredNodeId`が表示対象leafのIDであること、Node未開始のWorkflowのみのsnapshotで`preferredNodeId`が`None`であることをprojectionテストで確認する。 |
| R-009 | B-010 | 未実行Nodeを含むexecutionでNode detail index / session lookupを照会し、未実行Nodeのrecordが存在しないこと、開始済みNodeのdetailが取得できることをテストで確認する。 |
| R-010 | B-011 | 保持中の選択Node IDが含まれないsnapshotへ更新するfrontendテストで、選択が`preferredNodeId`のNodeへ遷移すること、`preferredNodeId`が`null`のとき未選択状態になることを確認する。 |
| R-011 | B-012 | 空childrenのWorkflow branchを含むsnapshotを入力とする`useWorkspaceTreeNodes` / `WorkspaceList`のテストでsnapshotが無加工で描画されることを確認し、frontend（`src/`）に`queued`比較やWorkflow定義・分岐結果による表示可否判定が追加されていないことをコード確認する。 |
| R-012 | B-013 | `docs/specs/milestone-82/design.md` §13と`docs/specs/milestone-82/goal-14-issue-1454.md`を読み、「未開始Nodeはqueued」記載が実行開始済みNode限定の契約へ更新され、`specs/unified-node-model/decisions.md`への参照があることを確認する。 |
