# Design

対象: `specs/issues-1491/requirements.md`（R-001〜R-005）/ `specs/issues-1491/behavior.md`（B-001〜B-006）

## The actual design

### Architecture

#### Workspace treeの構造規則をdomainへ移し、木は永続recordから組み立てる

`src-tauri/src/domain/workspace_tree/` を新設し、`WorkspaceTree` 集約を置く。集約は既存のopaque Node IDを維持したまま、親子関係、木への参加可否、同一Node/Sessionの重複禁止、実行occurrence順、Nodeとcontent/routing情報の整合を所有する。現在 `src-tauri/src/usecase/workflow/workspace_tree.rs` の手続きにある同じ判断を集約とdomain serviceへ移し、query時に再実装しない。Workflow/Sessionのclose・finalize・statusの意味は変更せず、確定済みのsource factをtreeへ反映する規則だけを所有させる。

集約は専用の永続スナップショットを持たない。`WorkspaceTree` は、workflow executionの永続record、その実行ノードの永続record、および同じworktreeに属するSessionの永続recordから `restore` して組み立てる。木のidentityであるmanaged worktreeの正規化済みpathは、これらのrecordが既に持っている値である。

この帰結として、集約のread-modify-writeが存在しない。treeを保存しないため、load後にcommitするまでの隙間を守るrevision guardも、workspace単位の単調revisionも設けない。書き込みで守るのは、`WorkflowExecution` 単位の実行ノードrecordとSession単位のrecordであり、これらは従来どおり各所有者のguardの下で更新される。

`WorkspaceTreeProjector` は、永続recordを `WorkspaceTree` へ適用する唯一のdomain serviceとする。write時に構造規則を検証する経路と、read時に木を組み立てる経路の双方が同じprojectorを通る。projectorが不変条件違反を返した場合、write経路ではbatchを提出せず、read経路ではerrorを返してfallbackしない。

root直下の並び(worktree直下のSessionとWorkflow branchの並び)は永続化しない。同じ並び規則をdomainが所有し、query時に決定する。execution内部のノード順(occurrence順、fanout childの順)は実行の事実であり、実行ノードrecordに保存する。

#### workflow executionをSQLite上の永続recordにする

workflow executionの状態は現在SQLiteに正本を持たず、event streamの畳み込みとしてのみ存在する。一覧と木の取得を蓄積量から切り離すには、畳み込みの結果を行として持つ以外に手段がない。よって `workflow_executions` と `workflow_execution_nodes` を追加する。前者はexecution 1件を1行で表し、後者はそのexecutionに属する実行ノードを表す。どちらもworkspaceではなくexecutionに属するrecordであり、workspace単位の派生copyではない。

これらの更新は既存のsource mutationと同じ `LocalAtomicBatch` / SQLite transactionで行う。`LocalStateMutation` には `WorkflowExecutionProjection`(execution行)と `WorkflowExecutionNodeProjection`(ノード行のdelta)を追加し、guardは `WorkflowExecution` 集約が既に所有するrevisionを使う。workspace用の第二のguardは設けない。

#### Session一覧は既存recordへ索引可能な列を足して解決する

`session_projection` は `session_id` を主キーとするJSON blobで、workspace・公開list区分・sort keyのいずれも列になっていない。これが全件走査の原因である。同じ内容を別テーブルへ複製せず、同じ行に列を足して索引する。

`list_kind` はSessionのstateから純粋に決まるが、一覧へ公開されるか否かと公開されるsummaryの内容は、reducer eventの畳み込み(`TurnEventLog` のbackend recovery projection)とrecovery publication snapshotの照合に依存する(現行 `published_agent_summary` の判断)。これはSQLでは導出できないため、公開判断の結果を同じ行の実カラムとして保存する。非公開のSessionはlist区分をNULLとし、index rangeから外れる。

これによりSession一覧の書き手は `session_projection` の既存writerのままであり、write authorityも行も増えない。

#### manual archiveの可視性はquery時のpolicyで重ねる

manual archive/restoreは構造sourceに含めず、永続recordはmanual archive中のWorkflow branchも構造Nodeとして保持する。archive可視性はdomain serviceの `WorkspaceTreeVisibilityPolicy` だけが決定し、branch Node IDとexecution IDの対応およびarchive snapshotのactive execution IDから、除外するbranch Node IDを返す。query実装はその決定をread modelへ適用する。Node detailとSession bindingは構造recordを直接読むため、archive後も対象IDから解決できる。

`workflow_execution_archives.json` と `WorkflowExecutionArchiveFileRepository` をmanual archiveの唯一のwrite authorityとして維持する。repositoryは起動時に一度decodeしたprocess-local indexをlock下で保持し、archive/restoreではatomic file replacement成功後に同じlock区間でindexを入れ替える。queryへは対象execution IDのactive状態だけを返すため、呼出しごとにJSON全体を再読込しない。SQLiteへのimport、archive用mutation、dual write、authority cutoverは行わない。

#### すべてのclient surfaceを一つのQueryServiceへ収束させる

usecase側に `WorkspaceQueryService` traitを置き、Workspace tree / Node detail / Session binding / Session一覧 / workflow execution一覧の入力を受けてusecase DTOを返す責務を持たせる。単一実装を、Workspace treeと `list_workspace_workflow_history` を公開する `WorkflowUsecase`、workflow execution一覧を公開する `WorkflowReadUsecase`(live loopback用cloneとstandaloneの両方)、`list_sessions` / `list_closed_sessions` を公開する `AgentSessionRuntimeUsecase` が共有する。各Usecaseは既存のworktree identity解決、公開list区分、filter、pagination、sortを維持してから同じtraitへ委譲する(R-005、B-006)。

旧 `WorkflowQueryService::list_executions`、`SessionStore` の全件走査経路、および `WorkflowExecutionFileRepository::query_canonical` / `SessionStore::list_metas_canonical` / `SessionStore::canonical_session_projection*` を経由する同期bridgeは削除する。分岐で残さない。

主要な変更対象は次のとおり。

- `src-tauri/src/domain/workspace_tree/`: `WorkspaceTree`、構造source fact、`WorkspaceTreeProjector`、`WorkspaceTreeVisibilityPolicy`、root並び規則、opaque Node ID生成規則、read-onlyのrepository contract。
- `src-tauri/src/domain/local_event/mutation.rs`: `WorkflowExecutionProjectionMutation` と `WorkflowExecutionNodeProjectionMutation` をclosedな `LocalStateMutation` へ追加する。
- `src-tauri/src/usecase/workspace_tree/`: usecase DTOと `WorkspaceQueryService` trait。
- `src-tauri/src/adaptor/gateway/workspace_tree/`: read-onlyのrepository実装、`WorkspaceQueryService` の実装、gateway-localなread model。read modelはこのmoduleからusecase側へ返さない。
- `src-tauri/src/adaptor/gateway/local_event_store/`: 追加mutationのcodecと同一transaction適用、`session_projection` への列追加、schema evolution、indexed query。
- `src-tauri/src/adaptor/gateway/workflow/execution_archive_repository.rs`: 既存JSONのwrite authority維持と対象ID lookup。file schemaと永続化契約は変更しない。
- `src-tauri/src/usecase/agent_session/`、`src-tauri/src/usecase/workflow/` のsource batch owner: execution行・ノード行・Session公開判断列の更新をsource commitへ含める。manual archive/restoreは構造recordを更新しない。
- `src-tauri/src/adaptor/controller/command/agent_session/stored_session.rs`、`src-tauri/src/adaptor/controller/command/workspace_tree.rs`、`src-tauri/src/usecase/workflow/workspace_tree.rs`: tree構築・全件収集・event replayの削除と、共通 `WorkspaceQueryService` への委譲。`list_closed_sessions` が `SessionStore` を直接受け取る経路も削除する。

### Interface

- `list_workspace_worktree_nodes` / `get_workspace_tree_selection_reconciliation` / `get_workspace_node_detail` は、command名、request、response、missing時の `None`、`nodes` と `preferredNodeId` の既存shape、opaque Node ID、並び順をすべて維持する。fieldの追加も削除も行わない。
- `get_workspace_session_node_id` は既存契約を維持し、Session本文やeventを入力にしない。
- `list_sessions` / `list_closed_sessions` / `list_workflow_executions` / `GET /v1/workflow/executions` / `list_workspace_workflow_history` は、request、response、公開list区分、filter、pagination、sortを変更せず、委譲先だけを共通 `WorkspaceQueryService` へ移す。
- 既存fieldを変更しないため、frontendは無変更で従来どおり描画できる。新しいloopback routeは追加しない。
- 内部境界のtraitは3つ。`WorkspaceQueryService`(usecase — 対象IDから表示DTOを返す)、`WorkspaceTreeRepository`(domain contract — 永続recordからの集約再構成)、`WorkspaceTreeVisibilityPolicy`(domain service — archive可視性の決定)。実装は各1つとし、同じcontractの第二実装を置かない。
- error: SQLite queryとarchive repositoryのfailureは `WorkspaceQueryService` のerror contractを通してapplication Usecaseまで分類を保つ。storage失敗とrecord不整合を同じ分類へ畳まない。永続recordまたはarchive indexが欠損・不正な場合に、event replayや全件scanへfallbackしない。

### Data Model

`WorkspaceTree` はworkspace identity、root直下および再帰Nodeを持つ。単調revisionは持たない。Node identityには現行のopaque Session/Workflow/Fanout/occurrence IDをそのまま使い、親identityを保持する。execution内部のsibling orderは実行の事実として保存し、root直下の並びはquery時にdomainの並び規則で決める。manual archive中のWorkflow branchも構造Nodeとして保持し、archive可視性は永続recordへ複製しない。

永続recordは次の3種で、いずれもworkspace単位の派生copyではない。

- execution record: execution 1件のidentity、workflow名、status、worktree path、sort key、および既存 `WorkflowExecutionSummaryDto` と同型の情報を構築できる内容。
- execution node record: executionに属する実行ノードのidentity、親identity、sibling order、Nodeとcontent/routing情報。Node IDのpoint lookupに使う。
- Session record: 既存 `session_projection` の行。ここに公開list区分、公開sort key、公開summaryを列として足す。

保持しないもの: Session本文、message、event履歴、Workflow event履歴、execution aggregate全体、pending recovery collection全体、Workspace単位のtreeスナップショット。recovery fenceはWorkspace tree表示に必要なowner別の導出結果だけをexecution node recordへ含める。

command出力(stdout/stderr)はNode detailの内容であり、tree取得では読まない。execution node recordのtree用列とdetail用内容を分け、tree queryのSELECTから除外する。

manual archiveの永続data modelは既存 `workflow_execution_archives.json` と `WorkflowExecutionManualArchiveRecord` のまま変更しない。process-local indexはそのauthorityをexecution IDで引ける形にしたmirrorであり、visibility decisionは永続化しない一時値である。

永続recordはversion付きのschema tagを持ち、public DTOのversionと分離する。

### Database

既存の固定 `local-event-store.sqlite3` へ次を追加する。別database、file cache、authority pointerは作らない。

- `workflow_executions` — execution 1件を1行。`execution_id` を主キーとし、`worktree_path`、`status`、sort keyを列に持つ。workspace・status・既存sort keyで対象一覧またはpageだけを読むため(R-002)。
- `workflow_execution_nodes` — `(execution_id, node_id)` を主キーとし、`parent_id`、`sibling_order`、`session_id` を列に持つ。tree取得、Node IDのpoint lookup、Session IDからNode IDの解決を、対象だけの読みで行うため(R-002)。
- `session_projection` への列追加 — 公開list区分(非公開はNULL)、公開sort key、公開summary、およびworkspace identity。既存の行に足し、別テーブルを作らない。

index は上記の述語と並び順に一致させる。status filterは列の等値条件で表現し、ORDER BY に同じ条件式を再掲しない。filterとORDER BYが同一indexの前方一致になる形にし、`USE TEMP B-TREE FOR ORDER BY` を発生させない。

作らないもの: workspace単位のtree table、workspace単位のnode table、workspace単位のsession summary table、workspace単位のexecution summary table。

これらのtableとカラムの通常runtime更新は、追加した `LocalStateMutation` の適用と既存 `session_projection` writerに限定する。read側のrepositoryは同じdatabaseから読むが書き込まない。

schema追加と既存canonical dataからの初期record生成には、#1499 で確立済みのschema evolutionの拡張点を使う。evolutionは既存のevent stream、`session_projection`、`pending_obligations` のowner indexから現在の状態を取得し、通常更新と同じdomain規則を通してrecordを生成する。source取得、record生成と書込み、schema version更新を一つのtransactionに含める。いずれかが失敗した場合は全体をrollbackして旧schemaを維持する。生成順に依存する比較は行わない。このmigration専用access pathは `WorkspaceQueryService` へ公開せず、通常queryのfallbackに使わない。

`workflow_execution_archives.json` はこのschema evolutionの入力・出力にせず、file schemaと配置を維持する。

### UI/UX

該当なし。表示内容、選択、並び順、操作フローを変更しない。frontendへ返すfieldを追加しない。

### Algorithm

write時は、source batchを所有するUsecaseが確定済みのsource factからexecution行・execution node行・Session公開判断列のdeltaを作り、既存source mutationと同じbatchへ追加する。deltaの生成はdomainの構造規則を通し、不変条件違反ではbatchを提出しない。workspace単位の集約をloadしないため、load-commit間のguard conflictとその再試行ループは存在しない。`OutcomeUnknown` では既存どおり同一batchの解決を行う。

manual archive/restoreは既存のworktree authorization後にarchive repositoryへ委譲し、構造recordを更新しない。

read時の境界は次のとおり(R-002、B-002、B-003)。

- tree: 対象worktreeのexecution行、それらのexecution node行のtree用列、および同worktreeの公開Session行だけを読み、domainのroot並び規則とvisibility decisionを適用してDTOを構築する。archive状態は対象executionのIDだけをprocess-local indexへ問い合わせる。
- detail: Node IDのpoint lookupとarchive状態だけを読む。detail用内容はここで初めて読む。
- Session binding: Session ID indexからNode IDだけを読む。
- list: filterとorderに対応するindex rangeだけを読み、DTOを直接構築する。

検証手段が自明でない受入条件は次のとおり扱う。B-002〜B-004は、無関係なevent/Session/executionを**同一workspace内と他workspaceの両方**に増やしても、対象のrow read数、実scan step数とsort発生、queryごとのfile read数、runtime/connection生成数が増えないことで確認する。read量の計測はdecode行数ではなくSQLiteのstatement statisticsを用い、LIMIT付きqueryでscanが隠れないようにする。B-005は、同一の永続recordから組み立てたtreeをlive commit直後と再起動後で比較する。B-006は、`WorkflowUsecase`、live loopbackへcloneされる `WorkflowReadUsecase`、standaloneの `WorkflowReadUsecase`、`AgentSessionRuntimeUsecase` が同じ `WorkspaceQueryService` 実装を共有することを、テスト専用の差し替えを介さない本番配線に対するcontract testで固定する。

### Infra

新しいinfra componentを追加しない。`LocalEventStore` のsingle writer、固定reader pool、既存Tokio runtimeを再利用する。blocking SQLite処理はreader worker内に閉じ、Tauri/loopback requestごとの専用thread、runtime、connection生成を避ける(R-003、B-004)。commit経路のasync境界からblocking readを同期呼び出ししない。

## Alternatives Considered

- Workspace単位のtree/summary tableを4枚置き、集約のスナップショットを保存する案: `session_projection` が既に答えられる問いに対して派生copyを増やし、正本と突き合わせる検証経路のない第二のrecordを作る。またload-commit間を守るためのworkspace revisionとCAS guardが必要になり、実際には誰も読まないrevisionを保存することになるため採らない。
- `workflow_execution_archives.json` のactive recordをSQLiteへimportし、archive authorityを統合する案: archive sourceのmigrationとauthority cutoverが必要になり、Non-goalを超えるため採らない。
- query時に現在のSession/Workflow eventからtreeを再構築し結果だけcacheする案: 蓄積量にread量が比例し、restart後の最初のqueryでreplayが必要になるためR-002/R-004を満たせない。
- `session_projection` に部分indexだけを足してtreeはusecaseで組み立て続ける案: 一覧のbounded化には足りるが、Node ID/session bindingの直接lookupと構造規則の単一ownerを確立できず、Workflow detailにevent replayが残る。本設計は一覧については既存行への列追加を採り、treeについてはexecution側のrecordを新設することで両方を満たす。
- process内のWorkspace tree cacheを正本にする案: restart後に再構築が必要で、複数surfaceが共有するdurableなrecordにならない。

## Cross-cutting concerns

- 整合性: execution行・execution node行・Session列の更新を、対応するsource mutationと同じ `LocalAtomicBatch`/SQLite commitに含める。treeはこれらの導出結果であり、独立した保存状態を持たないため、live stateと再構築結果が分岐する経路がない。manual archiveは構造recordの入力にせずquery時のsnapshotとして重ねるため、dual writeやcross-store eventual consistency経路を作らない。
- 性能: queryのread量を、返却Node数またはrequest page数とpoint lookup数にだけ比例させる。commit経路のlookupも対象IDのindex point readに限定する。
- 互換性: 既存command/route、request、response、opaque ID、既存DTO fieldを維持する。field追加も行わない。transportごとのprojection実装を持たない。
- 安全性: 構造recordへSession本文・event履歴・内部artifact全体を複製せず、tree取得ではdetail用内容を読まない。既存local storeのpermission、codec validation、correlation付きstorage errorを再利用し、errorの原因情報を破棄しない。
- 可観測性: 検証用の計測はtest instrumentationに閉じ、利用者向けの新しいtelemetry契約を追加しない。

## Risks

- 構造recordの更新をsource batchの一経路でも漏らすと、treeに現れない実行が残る。各source batch ownerをcommit-level parity testで網羅する。
- archive fileとprocess-local indexが分岐すると、可視treeとworkflow historyが食い違う。file replacement成功までは旧indexを維持し、成功後の入替まで同じlockを保持する。process crash時は次回起動でauthority fileから再生成する。
- schema evolution時の初期record生成は既存履歴量に比例しうる。一度のevolutionに限定し、normal admission前に完了または既存のstartup failureへ閉じる。
- root並びをquery時に決めるため、並び規則の変更が既存の選択とaction routingの見え方を変える。現行規則をdomainへ移植し、監査基準commitの並びとのgolden比較で固定する。
- opaque Node ID生成規則をdomainへ移す際に規則を変えると、保存済み選択とaction routingが壊れる。現行規則をそのまま移植し、golden比較で固定する。
- `session_projection` への列追加は既存の巨大テーブルへのDDLになる。evolutionを一度に限定し、失敗時は旧schemaを維持する。
