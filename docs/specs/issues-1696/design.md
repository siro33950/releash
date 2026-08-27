# Design

## The actual design

### Architecture

#### 実行木の root の事実を1種類にし、木の起こされ方を値として持つ

`TreeRootFact::Workflow(WorkflowRootFact)` / `TreeRootFact::Session(SessionRootFact)`（`domain/workflow/value_objects/node_fact.rs:98-128`）という variant 分けを廃し、root の事実を1種類にする。単独 Session も Session node 1個の `WorkflowDefinition` を持つ木として書き側が記録する。

統合後の root の事実が持つもののうち、現行のどちらか一方にしか無い、または新設するものは次の3つ。他（`worktree_path` / `created_from` / `request`）は現行 `WorkflowRootFact` と同じ。

- `workspace_identity`: 現行 `SessionRootFact` だけが持つ。terminal surface の owner 鍵として呼び出し側の指定値を往復させる必要があるため（`node_fact.rs:119-121`）保持する。workflow の実行として起こす木では、現行 `derive_session` と同じく `WorkspaceIdentity::new(worktree_path)` の値を入れる。
- `definition`: 現行 `WorkflowRootFact` だけが持つ。Session の起動として起こす木では、書き側が Session node 1個の定義を組み立てて記録する。node の `SessionSpec` が現行 `SessionRootFact.session` の役割を引き継ぐ。
- `launched_as`: 新設。値オブジェクト `ExecutionTreeLaunch { Workflow, Session }`（`domain/workflow`）。その木が workflow の実行として起こされたか、Session の起動として起こされたかを表す。木の生成時に確定し、以後変わらない。

`ExecutionTreeLaunch` は木の種別ではなく、木がどう起こされたかの記録である。木の構造（root node の kind、child の有無）にも定義の内容にも依存せず、これらから導出しない。`main` が Session node の workflow は定義でき（`domain/workflow/services/validation.rs:1288,1628` は entry の kind を検査しない）、Session node 1個という構造を単独 Session と共有するが、`launched_as` では区別できる。

読み側から次が消える。

- `standalone_session_definition`（`domain/workflow/services/fact_replay.rs:202-219`）と `restore_aggregate` の root 分岐（同 `169-185`）。定義は root の事実にあり、合成しない。
- `locate_session`（`agent_session_repository.rs:760-777`）の「root started が `TreeRootFact::Session` なら root を location とする」特例分岐。`find_session_attachment` だけで解決する。
- `derive_session`（同 `803-843`）の root 種別による分岐。
- `domain/workspace_tree/projection.rs:82-88` の構造 fallback と `RuntimeSnapshotNodeProjection.standalone_session_id` field、および `adaptor/gateway/workspace_tree/repository.rs:107` の受け渡し。
- `adaptor/gateway/workspace_tree/repository.rs:193-210`（`node_id_for_session`）の attachment 不在時の構造 fallback。

`adaptor/gateway/workspace_tree/query_service.rs:65,120,138` の読み分けは残すが、述語を `TreeRootFact` の variant から `launched_as` に替える。workflow 実行一覧は `Workflow`、workspace の session 一覧は `Session` を対象にする（R-008 / B-012）。

#### workflow が Session の lifecycle を所有するかを木の起こされ方で判定する

`AgentSession` の `tree_parent: Option<AgentSessionTreeParent>`（`domain/agent_session/aggregates/agent_session.rs`）を廃止し、必須の値オブジェクト `AgentSessionTreeLocation { tree_id, node_execution_id, launched_as }` に置き換える。`launched_as` の正本は木の root の事実であり、AgentSession は復元時にそこから読んだ導出値として持つ。

現行 `tree_parent` を lifecycle 所有の述語にしている箇所は `launched_as == Workflow` に替える。実行木の node 終端に伴う停止は lifecycle 所有の判定ではないため、node execution の一致だけで認可する。

| 箇所 | 現行の述語 |
| --- | --- |
| `operations`（archive / restore / delete の可否、`agent_session.rs:283`） | `tree_parent.is_none()` |
| `archive` / `authorize_delete` / `authorize_archive_fallback_delete` / `authorize_gc` | `tree_parent.is_some()` |
| `admit_initial_instruction` | `tree_parent.is_none()` |
| `authorize_execution_tree_node_stop`（旧 `authorize_workflow_stop`） | `node_execution_id` の一致のみ |
| `authorize_workflow_launch_rollback` | `tree_parent.is_none()` |
| `open_action`（`Open` かつ provider session 未確定のときの GC 判定、同 `374-378`） | `tree_parent.is_some()` |
| `observe_provider_process_exit`（`GcRequired` の判定、同 `389-391`） | `tree_parent.is_none()` |

親の有無を述語にしてはならない。`prepare_workflow_node`（`usecase/agent_session/agent_session_launch.rs:317`）は node の親の有無に関係なく workflow 所有として起動し、workflow 木の root node execution は `parent = None` で記録される（`adaptor/gateway/workflow/fact_log.rs:158-160`）。`main` が Session node の workflow ではその Session が親を持たない workflow 所有 Session になるため、親の有無を述語にすると R-006 / B-008 が破れる。

#### provider completion signal の受理を実行木 1 本の経路に統一する

`usecase/provider_lifecycle/ingress.rs:177` の `session.tree_parent().is_some()` 分岐を削除し、`StopObserved` は常に AgentSession の `tree_location` を使って control plane の Stop transaction へ送る。木の形にも `launched_as` にも分岐しない。`ProviderLifecycleUsecase::receive`（実行木へ記録しない経路）は Stop に対して使わない。

これに伴い、境界の語彙を workflow 固有から実行木の語彙へ改める。`ProviderWorkflowStopTransaction` / `ProviderWorkflowStopCommand`（`usecase/provider_lifecycle/ingress.rs:35-49`）は木を指す名前に改め、`workflow_execution_id` field は `tree_id` にする。実装側（`usecase/workflow/runtime_command.rs:167`、`usecase/workflow/control_plane.rs:425`）の判断と domain 呼び出し（`WorkflowExecution::record_provider_stop` → `apply_completion_handshake`）は変更しない。単独 Session は Stop だけを受けるため handshake は `AwaitingSignal` に留まり、node は完了しない（B-009 と同一経路）。

node execution を宛先にする既存 command のうち submit / approve は `launched_as` で分岐させず、Session node に対する既存規則がそのまま受理を決める。explicit retry だけは木の起こされ方を規則に含め、`launched_as == Workflow` の木でのみ受理する。Workspace ツリーの `can_retry` と command の受理は workflow aggregate の同じ述語を使うため、UI を経由しない要求でも Session 起動由来の木に新しい attempt は作られない。

#### 実行木を start 時に engine の作業キャッシュへ載せる

`WorkflowRuntimeHost.executions`（in-memory・非永続の作業キャッシュ）に、`launched_as` に関わらず実行木を載せる。載せる契機は2つ。

1. Session の起動。`AgentSessionLaunchUsecase` が create commit 前に tree id を予約し、commit 成功後・provider process 起動前に木を本登録してから予約を解放する。予約は本登録の成否にかかわらず解放し、commit 失敗時も既存 rollback 経路へ合流する前に解放する。`prepare_new_session` 経由の起動と provider history からの resume 起動の両方が同じ予約点と登録点を通る。
2. startup reconciliation。`reconcile_startup`（`adaptor/gateway/workflow/workflow_host.rs:625`）の `TreeRootFact::Workflow` filter を、登録に関しては外す。

launch 時登録は表示のためではなく、`reconcile_tree_pass`（`adaptor/gateway/workflow/fact_log.rs:680-735`）が依存する不変条件のために要る。同 pass は「attach / spawn は記録済みだが engine に載っていない Running leaf」をプロセス喪失と見なして `ProcessExited` を追記する。`reconcile_startup` は本登録済みの木に加えて予約中の tree id も対象外にする。予約を engine の作業状態に含めることで、create commit の瞬間から「engine に載っている = 生きている」が成立し、commit と本登録の間に喪失判定の窓が生じない。本登録後にだけ予約を外すため、両状態の間にも隙間はない。予約は `executions` と同じく in-memory・非永続であり、プロセスが消滅した場合は次回起動の reconciliation が通常どおり喪失を観測する。

登録に失敗した場合は commit 後の launch 失敗として扱い、既存の launch rollback で create 済み Session と起動資源を巻き戻す。provider history からの resume も、登録前に create した Session を同様に削除して一次の登録失敗を返す。

起動済みの Session の leaf を engine が起動し直すことはない。これは木の起こされ方による分岐ではなく、attach の事実が root started と同じ batch に記録されること（後述）の帰結である。`reconcile_tree_pass` は attach 済みの leaf を `pending_leaf_ids` に入れない（`fact_log.rs:711-714`）。

#### workflow 実行 registry は workflow として起こされた木だけを保持する

`ExecutionStore`（`adaptor/gateway/workflow/execution_store.rs`）は active workflow execution の registry であり、`worktree_path` ごとに active execution を 1 本に制限し（`register_active_execution`）、execution id が UUID であることを要求する（`is_valid_execution_id`）。Session の起動として起こした木の id は AgentSession id（`agent-session-<hex>`、`domain/agent_session/launch_identity.rs`）であり、かつ同一 worktree で複数の Session と workflow が同時に生きうる。したがってこの registry には `launched_as == Workflow` の木だけを入れる。

`commit_control_plane_candidate` の commit 後同期（`sync_state_after_required_event_commit`）も `launched_as == Workflow` のときだけ行う。判定材料として、作業キャッシュのエントリに `launched_as` を持たせる（登録時に root の事実から決まる）。commit 経路で事実ログを読み直さないためである。

#### node と AgentSession の紐づけを root started と同じ durable batch で記録する

Session の起動として起こす木の生成（`agent_session_repository.rs:436-466`）で、root の `Started` と同じ batch に `SessionAttached { session_id, provider_session_id: None, transcript_ref: None }` を追記する。以後 `ProviderSessionAssociated` が起きたときに provider session id 付きの `SessionAttached` を追記する現行の挙動は変えない（`derive_session_facts` は `provider_session_id` が `None` の attach で既存値を上書きしない、`fact_replay.rs:387-394`）。

これにより node と AgentSession の紐づけが記録された事実だけで決まり、構造条件からの推測を全て除去できる（R-004）。`record_provider_stop` の所有確認（node の `session_id` と受信 session の一致、`workflow_execution/mod.rs:2611-2634`）も、起動直後から成立する。

### Interface

外部契約の変更は AgentSession の読み取り 1 箇所。`AgentSessionItemDto`（`usecase/agent_session/agent_session_query.rs`）の

```
tree_parent: Option<AgentSessionTreeParentDto>   // treeParent?: {...} | null
```

を必須 field に置き換える。

```
tree_location: AgentSessionTreeLocationDto { tree_id, node_execution_id }   // treeLocation: { treeId, nodeExecutionId }
```

- 公開経路: Tauri command `get_agent_session`（`adaptor/controller/command/agent_session/provider_tui.rs:353`）と workspace tree snapshot の `archivedSessions`。
- `launched_as` は DTO に出さない。現行の frontend / CLI に読み手がなく、操作可否は既存の `operations` が表す。
- 互換: 破壊的変更として入れる。凍結前 prototype であり、`treeParent` の production 読み手は存在しない（`src/types/agent-session.ts` の型宣言のみ）。移行手段は設けない。

内部境界:

- 実行木登録の port（trait 1つ、`usecase/agent_session/agent_session_launch.rs` に消費側の言語で定義）— 「開始済みの実行木を engine の作業キャッシュへ載せる」責務のみを持つ。実装は `WorkflowRuntimeUsecase`（`usecase/workflow/runtime_command.rs`）が `WorkflowControlPlaneRuntime` へ委譲する。
- 配線は composition root（`adaptor/controller/agent_session_wiring.rs`）で行い、workflow composition が agent_session composition に依存する既存の向きを保つため、`DeferredProviderWorkflowStopTransaction` と同じ遅延 bind を使う。

### Data Model

- 新しい事実の種別を追加しない。node と AgentSession の紐づけは既存の `SessionAttachedFact`、completion signal は既存の `StopReceivedFact` を使う。
- root の事実は variant を持たない1種類の record になる。identity は木の `tree_id`。持つものは Architecture に挙げたとおりで、`SessionRootFact.session`（`SessionSpec`）は定義の Session node へ移すため record からは無くなる。
- `ExecutionTreeLaunch` は `domain/workflow` の値オブジェクト。木がどう起こされたかだけを持ち、定義や worktree を保持しない。
- `AgentSessionTreeLocation` が `AgentSessionTreeParent` を置き換える。identity は `tree_id` + `node_execution_id`（現行 `AgentSessionTreeParent` と同じ 2 値）に `ExecutionTreeLaunch` を加えたもの。`AgentSessionLifecycleEvent::Created` が持つ値も同じ型に変わる。
- engine 作業キャッシュのエントリが `ExecutionTreeLaunch` を併せ持つ（in-memory のみ、永続化しない）。
- versioning は行わない。root の事実の schema は非互換に変わるが、Assumption により本変更以前の記録は対象外とする。

### Database

- スキーマ変更なし。
- session から実行木上の所在を引く access path を `node_events` の `session_id` 列に対する `latest_session_attachment`（`local_event_store/node_events.rs:172`）1 本に統一する。すべての Session で attach 事実が必ず存在するようになるため、`locate_session` と `node_id_for_session` が持っていた「木の先頭行を読む」second path が不要になる。

### UI/UX

新しい画面・操作・API は追加しない。単独 Session の行の色は既存の `classify_own_status`（`domain/workspace_tree/value_objects/mod.rs:159-176`）が `Running` + `StopReceived` を `Attention` に分類した結果として変わる。更新契機も既存のまま（Stop commit 後の `workflow-execution-changed` broadcast が worktree 一致で workspace tree を再取得する）。

workflow 実行一覧と workspace の session 一覧の内容は現行と同じで、述語だけが `launched_as` に替わる。

### Algorithm

該当なし。completion signal の累積規則、completion 条件の判定、事実列の fold はいずれも既存のものを変更しない。

### Infra

該当なし。

## Alternatives Considered

- **root の事実の種別（`TreeRootFact::Session`）を維持し、木の種別を AgentSession の必須属性として持たせる。** 書き側の Stop 経路だけは統一できる。しかし fact log・読み経路・engine 登録判定に木の種別が残り、定義の合成（`standalone_session_definition`）と読み分けも残るため、Outcome の「実行木の表現から単独 Session という区分が無くなる」を満たさない。採らない。
- **所有権規則の述語を「その Session が結びつく node execution が木の root か」にする。** 新しい値を足さずに済むが、`main` が Session node の workflow から起動された Session が archive / delete / GC の対象になり、R-006 / B-008 を満たさない。採らない。
- **ingress の分岐を「親の有無」から「木の起こされ方」に変えるだけにする。** 単独 Session の Stop は AgentSession repository が木へ直接追記する。書き経路が 2 本のまま残り、Scope が変更対象とした乖離（同じ概念を 2 箇所で表現する）が消えないため採らない。
- **completion signal を AgentSession repository から追記する。** 既に同 repository は `ProcessExited` / `ArchiveRequested` を木へ追記しており最短ではある。しかし `docs/glossary/DOMAIN.md` は NodeExecution の completion signal の transition を workflow aggregate だけが決めると定めており、completion handshake（Submit と Stop の合流、`completion: Approval` の分岐）の第二実装が必要になる。採らない。
- **launch 時に登録せず、control plane が未登録の木を事実から fold して遅延採用する。** 登録用の配線は不要になるが、`reconcile_tree_pass` が「engine に載っていない activated leaf = プロセス喪失」を推論するため、生きている Session が喪失として記録される。また workflow 木を reconcile なしで採用すると、その後の `reconcile_startup` が `contains_key` で skip して喪失観測と leaf 再起動が行われなくなる。採らない。
- **Session の起動自体を engine の node 起動経路（`prepare_workflow_node` / `activate_workflow_node`）へ寄せる。** 統一度は最も高いが、terminal の rows/cols、provider history からの resume、provider session 所有権の CAS、冪等 create（rearm）を engine 側へ移す必要があり、本 ISSUE の範囲を大きく超える。採らない。
- **`tree_parent: Option` を残したまま、必須の tree location を別 field として追加する。** 同じ概念が 2 箇所で表現されたままになり、「親がない = 実行木がない」の読み替えを構造として除去できない。採らない。

## Cross-cutting concerns

**作業キャッシュの位置づけと full-recompute の回避.** engine の `executions` は判断のための非永続の作業キャッシュであり、正本は事実ログである。Session の起動として起こした木は AgentSession repository（`ProcessExited` / `ArchiveRequested` / `ResumeRequested` / `RestoreRequested`）と control plane（`StopReceived`）の 2 つの書き手を持つため、キャッシュは repository 側の追記に対して stale になりうる。これに対して commit ごとに木を fold し直す（full-recompute 経路の追加）ことはしない。事実の fold が順序に耐えるためである（`fact_replay.rs:272-295`: Stop 受信済みの Session node は `ProcessExited` で中断しない／`ProcessExited` 後の Stop は Paused の node に signal を記録するだけで色は Idle のまま）。stale なキャッシュ上の判断が生む追記は、durable な投影の結果を変えない。

**Stop 受理経路の lock 規律.** Stop の commit は同一 AgentSession の operation lock と provider lifecycle slot を保持したまま走る。#1695 はこの経路で Session 停止 effect を同じ call stack で実行すると provider の Stop 受理が deadlock することを示し、`spawn_committed_runtime_effects` で切り離した。Session 起動由来の木の Stop も同じ経路に載るため、この経路に同期の session 停止 effect を追加しない。

**検証手段.** 自明でないものだけ挙げる。

- B-002（木の形と起こされ方に非依存）: control plane usecase の境界で、同一の Stop 入力を `launched_as` の異なる 2 本の木に対して流し、`StopReceived` への遷移が一致することを 1 つの test で対にして確認する。
- B-003（再起動後も黄）: gateway の事実ログ test で、`StopReceived` の Session node を持つ木に対する reconciliation pass が `ProcessExited` を追記しないこと、および再 fold 後の分類が `Attention` のままであることを確認する。
- B-008（workflow が起こした木の Session の拒否）: domain 集約 test に、`launched_as == Workflow` かつ Session node が木の root（親なし）である場合を含める。親の有無を述語にした実装ではここが通らない。
- B-012（一覧の分離）: gateway の query service test で、同一 worktree に `launched_as` の異なる木を置き、両一覧の内容が分かれることを確認する。
- B-013（Session 起動由来の木の retry 拒否）: workflow aggregate の test で `accepts_explicit_retry` と attempt 不生成を確認し、local API / Tauri command を通る acceptance test で拒否後の木と AgentSession の不変性を確認する。Session 起動由来 retry の prepare 経路は削除するため、削除前の `prepare_session_tree_retry` が持っていた predecessor の origin / tree id / provider の guard は検証対象にせず、その関数を直接呼ぶ test も置かない。

## Risks

- **Session 起動由来の木への retry 到達.** `launched_as == Session` の木は explicit retry を受理せず、Workspace ツリーの `can_retry` も false にする。外部インターフェースから直接要求されても aggregate が拒否するため、新しい attempt や provider process は作られない。
- **workflow retry の回帰.** `launched_as == Workflow` の木では既存の retry 受理、past attempt の表示、`retry_predecessors` の記録、失敗時 cleanup を変更しない。Session 起動由来 retry の prepare / activate / rollback 経路は削除するため、その失敗時 cleanup を別途定義する未決事項は残らない。
