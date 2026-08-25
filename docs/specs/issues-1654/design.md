# Design

対象: [requirements.md](requirements.md)（R-001〜R-007）/ [behavior.md](behavior.md)（B-001〜B-014）

## The actual design

### Architecture

#### NodeExecution の終端差分を durable commit 後の AgentSession 停止効果にする

`src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs` の `WorkflowExecution` が所有する NodeExecution status を正本とし、commit 前後の同一 NodeExecution を比較して、Session が active から `Succeeded` / `Failed` / `Aborted` へ初めて遷移した場合だけ停止対象を導出する。対象の identity は NodeExecution ID と、その NodeExecution が参照する AgentSession ID の組である。`current_session_id` や WorkflowExecution 全体の終端状態から対象を推定しないため、Sequence / Fanout、失敗処遇、承認を含む実行木の各 Session attempt を同じ規則で扱える。

`src-tauri/src/usecase/workflow/runtime_driver.rs` の durable transaction 境界が、この差分を `WorkflowRuntimeEffect` の AgentSession 停止効果として保持する。効果は canonical workflow fact の永続化に成功するまで取り出せず、永続化失敗時には実行しない。`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs` は durable transaction から解放された停止効果を、後続 Node の activation より先に実行する。これにより、終端した Session の枠を解放してから次の Session を起動する。

abort は `src-tauri/src/adaptor/gateway/workflow/workflow_host/lifecycle_commands.rs` に独立した required-event commit 経路を持つため、同じ before/after 差分から停止効果を作り、`ExecutionAborted` の durable append 後に実行する。command runtime の shutdown は従来どおり別に行う。workflow execution の stop は NodeExecution を `Paused` に留めるため停止効果を生成せず、既存の Ctrl-C による interrupt と resume 経路を維持する。

根拠は、workflow aggregate だけが NodeExecution の遷移を決め、NodeExecution は AgentSession を参照するが所有しないと定める [architecture/README.md](../../architecture/README.md)、[architecture/DOMAIN.md](../../architecture/DOMAIN.md)、[glossary/DOMAIN.md](../../glossary/DOMAIN.md) と、外部作用を durable commit 後に解放する `src-tauri/src/usecase/workflow/runtime_driver.rs` の既存 transaction 境界である。

#### AgentSession が checkpoint 保持停止を所有する

`src-tauri/src/adaptor/gateway/workflow/node_session_boundary.rs` の内部境界 `WorkflowAgentSessionPort` に、workflow-owned AgentSession の provider プロセスを terminal checkpoint 保持のまま停止する責務を追加する。`ProviderWorkflowAgentSessionPort` はこの操作を `src-tauri/src/usecase/agent_session/agent_session_lifecycle.rs` の `AgentSessionLifecycleUsecase` へ委譲する。

AgentSession 側は既存の session operation lock の内側で対象を読み、`tree_parent` の NodeExecution ID が停止要求と一致する workflow-owned session だけを受理する。これにより、workflow に紐づかない手動 AgentSession の lifecycle は変更しない。停止には `ProviderAgentTerminalGateway::stop_preserving_checkpoint` を使い、`src-tauri/src/usecase/terminal_surface/lifecycle_usecase.rs` の既存処理によって runtime の停止、output drain、surface / runtime の registry からの除去を行う一方、terminal checkpoint は削除しない。AgentSession の `provider_session_id` と transcript reference は変更しないため、既存の open / resume 経路が provider CLI の resume 機能を使用できる。

workflow 側は停止失敗を各効果の内側に閉じ、best-effort の warning を試みるだけで、Submit、承認、failure settlement、abort の結果には変換しない。warning の永続化や公開方法は追加せず、#1653 の可観測性改善を本件へ含めない。複数の停止対象がある場合も、一件の失敗で残りの停止を省略しない。NodeExecution の終端事実は既に durable であり、AgentSession 停止はその後の外部作用だからである。

#### provider Stop の post-commit 失敗を受理結果から分離する

provider Stop の transaction は、workflow の状態を確定する canonical workflow facts を先に append し、その後に provider lifecycle event を別 stream へ commit する。canonical facts の append が失敗した場合は Stop の受理自体を失敗として返す。一方、canonical facts が durable になった後の provider lifecycle commit 失敗は post-commit 失敗であり、確定済みの Stop を失敗へ戻さない。`src-tauri/src/adaptor/gateway/workflow/event_log_writer.rs` はこの状態を error ではなく専用 outcome として返し、`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs` は warning を記録したうえで `record_provider_stop` を成功として完了する。

provider lifecycle event はこの失敗時に欠落し得る。本変更では再配送を前提にせず、retry queue、起動時 reconciliation、診断情報の永続化を追加しない。

#### 停止後の AgentSession lifecycle と workflow-owned session の除去権限

停止は surface と runtime を registry から除去するため、`src-tauri/src/usecase/agent_session/agent_session_exit.rs` の exit 観測は停止済み session を対象として返さない。したがって停止操作自体が AgentSession の lifecycle settlement を所有する。`src-tauri/src/domain/agent_session/aggregates/agent_session.rs` の workflow 停止専用遷移が、対象 session を `Paused` へ遷移させ、意図的停止を異常終了として記録しない lifecycle event を同じ操作で生成する。usecase はその遷移を永続化した後、observed exit の非 GC 経路と同じ launch binding の解放と launch gateway の cleanup を行う。停止後の open は PTY 不在と `Paused` から停止前の terminal checkpoint を提示し、`provider_session_id` を持つ session は既存の resume 経路で provider CLI の再開機能を使う。

`provider_session_id` が確定する前に終端した node execution の session も停止対象に含める。R-001 が終端状態を `Succeeded` / `Failed` / `Aborted` の三つとも対象にしており、provider hook が `provider_session_id` を関連付ける前に終端する経路があるためである。この session を既存の除去規則（PTY 不在かつ `provider_session_id` 未確定なら garbage）へ落とすと、open と `src-tauri/src/usecase/agent_session/agent_session_read.rs` の読み取り時 reconciliation が terminal checkpoint ごと AgentSession を削除し、R-007 を満たせない。そこで `src-tauri/src/domain/agent_session/aggregates/agent_session.rs` の除去判定で、`tree_parent` を持つ workflow-owned session を garbage collection の対象外とし、上記の lifecycle settlement も `provider_session_id` の有無で分岐させない。既存 aggregate が archive と delete を workflow-owned に対して拒否しているのと同じく、除去権限を workflow 側に残す。

Resume の受理規則は AgentSession aggregate が所有し、`Paused` かつ `provider_session_id` が確定済みの場合だけ受理する。同じ判定から `AgentSessionOperations.can_resume` を作り、query service の read model へ透過させる。usecase と frontend は `provider_session_id` の有無から可否を再判定しない。

主要な変更対象は次のとおり。

| Path | 変更の要旨 |
| --- | --- |
| `src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs` | commit 前後から newly-terminal な Session と AgentSession 参照を導出する domain-owned 判定 |
| `src-tauri/src/usecase/workflow/runtime_driver.rs` | AgentSession 停止効果を durable commit より前に秘匿し、成功後だけ解放する transaction material |
| `src-tauri/src/adaptor/gateway/workflow/workflow_host.rs` | 通常の control-plane commit 後、後続 activation 前に停止効果を実行する共通境界 |
| `src-tauri/src/adaptor/gateway/workflow/workflow_host/lifecycle_commands.rs` | abort の durable append 後に同じ停止効果を実行し、停止失敗を abort result へ返さない経路 |
| `src-tauri/src/adaptor/gateway/workflow/event_log_writer.rs` | canonical workflow facts 確定後の provider lifecycle commit 失敗を post-commit outcome として返す境界 |
| `src-tauri/src/adaptor/gateway/workflow/node_session_boundary.rs` | workflow 内部 port から AgentSession lifecycle の checkpoint 保持停止へ変換する境界 |
| `src-tauri/src/domain/agent_session/aggregates/agent_session.rs` | workflow-owned session の停止遷移、Resume の受理規則、および workflow-owned session を garbage collection の対象外にする除去判定 |
| `src-tauri/src/usecase/agent_session/agent_session_query.rs` | domain が判定した Resume 可否を含む AgentSession read model |
| `src-tauri/src/adaptor/gateway/agent_session/agent_session_query_service.rs` | AgentSession の操作可否を read model へ透過する変換境界 |
| `src-tauri/src/usecase/agent_session/agent_session_lifecycle.rs` | session operation lock、受理判定、checkpoint 保持停止、停止後の lifecycle settlement を順序付ける usecase |
| `src-tauri/src/adaptor/gateway/workflow/runtime_command_gateway.rs` | 既存の AgentSession lifecycle usecase を workflow session port へ渡す composition |

### Interface

公開 Tauri command、local API、CLI、workflow definition、event schema の入出力は変更しない。Submit、承認、abort の成功・失敗表現も変更せず、post-commit の停止失敗を新しい公開 error にしない。

内部境界 `WorkflowAgentSessionPort` は、NodeExecution ID と AgentSession ID で特定した workflow-owned session を checkpoint 保持のまま停止する責務を追加する。provider process、Terminal Surface、Provider lifecycle の具体型やエラーは workflow usecase へ漏らさず、既存の `WorkflowRuntimeError` 境界で閉じる。

### Data Model

永続 record は追加しない。停止効果は一回の workflow transaction に属する一時データであり、NodeExecution ID と AgentSession ID を identity とする。既に終端していた NodeExecution、Session 以外の Node kind、AgentSession 参照を持たない NodeExecution からは生成しない。

AgentSession の既存 `provider_session_id`、transcript reference、`tree_parent`、Terminal Surface owner を再利用する。conversation 本文は保持せず、引き続き provider CLI / transcript を正本とする。terminal checkpoint の形式と保持先も変更しない。

### Database

該当なし。workflow fact、AgentSession lifecycle event、terminal checkpoint の schema は変更せず、停止効果の durable queue や起動時 reconciliation record は追加しない。

### UI/UX

AgentSession read model の `operations.canResume` が true の場合だけ、停止中の AgentSession に Resume を表示する。停止中の表示は provider プロセスが動作していないという事実だけを述べ、再開できない理由を frontend で断定しない。`canResume` が false になる状態は複数あり、read model はその理由を区別しないためである。frontend は lifecycle や provider identity から可否を再計算しない。

### Algorithm

停止対象の導出は、commit 前に active だった各 NodeExecution を ID で commit 後へ対応付け、commit 後が非 active、kind が Session、AgentSession ID が存在するものだけを選ぶ。同じ commit 内で新規に終端した対象だけを一度ずつ返す。active / terminal の分類は `RuntimeNodeExecutionStatus::is_active` を使い、状態集合を別実装へ複製しない。

通常経路の順序は「aggregate 遷移 → canonical fact 永続化 → live aggregate 公開 → AgentSession 停止効果 → 後続 Node activation / state broadcast」とする。永続化前に停止しないため、commit failure で active のまま残った NodeExecution の provider processを失わない。後続 activation 前に停止するため、直前に終端した Session の枠を次の spawn が利用できる。

abort 経路は activation cancellation を確定し、`ExecutionAborted` を durable append した後、active から `Aborted` になった全 Session の停止効果と command runtime shutdown を行ってから既存の terminal transition cleanup を続ける。停止効果の失敗は best-effort の warning 用に収集するが、処理を継続し、accepted abort を失敗へ戻さない。

必要な検証は次の種別で行う。

- workflow domain: `Running` / `Paused` / `WaitingApproval` から `Succeeded` / `Failed` / `Aborted` への Session 遷移だけが停止対象になり、active の維持、既終端、Command / composite は対象外になること。
- workflow transaction / host: persistence failure では停止せず、durable commit 後は後続 activation より前に停止すること。Submit、承認、failure settlement、abort の各終端経路と、stop / resume / WaitingApproval の非停止経路を扱う。
- AgentSession usecase / gateway: workflow ownershipを照合し、checkpoint を削除せず surface を枠から除外すること。停止後に既存 provider session identity で resume でき、手動 AgentSession には作用しないこと。`provider_session_id` が未確定のまま終端した session を停止しても、checkpoint と AgentSession が garbage collection されずに残り、read model が Resume 不可を返すこと。
- frontend: read model が Resume 可の場合だけ停止中の AgentSession に Resume を表示すること。
- workflow acceptance: 同一 worktree で終端済み Session を `per_worktree_cap` 以上累積させても、新しい workflow Session と手動 AgentSession が `WorktreeCapReached` にならないこと。停止失敗を注入しても Submit、承認、abort の確定結果が維持されること。

### Infra

該当なし。process 停止、Terminal output drain、checkpoint 保存には既存の Terminal Surface runtime と gateway を使い、新しい daemon、scheduler、background collector は追加しない。

## Alternatives Considered

- **execution 終端時または archive 時に一括停止する**: NodeExecution 終端から枠解放まで後続 Node や次の WorkflowExecution が起動でき、同一 execution 内でも cap を消費し続ける。R-001 の停止時点と Scope に一致しないため採らない。
- **Ctrl-C の interrupt を停止として使う**: process と Terminal Surface が残り枠を解放しないことが現障害の原因であり、R-001 / R-004を満たさないため採らない。interrupt は active な `Paused` Session の resume を保つ workflow stop 専用として残す。
- **Terminal Surface を delete / kill する**: process と枠は解放できるが checkpoint を破棄し、R-002 / B-004を満たさないため採らない。
- **durable commit 前に provider process を停止する**: workflow fact の永続化失敗時に NodeExecution が active のまま provider process だけを失い、R-003に反するため採らない。
- **後続 Node activation 後に停止する**: cap 境界では次の spawn が先に `WorktreeCapReached` となり、R-004を満たせないため採らない。
- **cap 引き上げ、idle 回収、起動時 reconciliation**: 残留速度を変えるか別時点で回収する案であり、NodeExecution 終端時の所有責務を解決せず、明示された Non-goals にも含まれるため採らない。

## Cross-cutting concerns

該当なし。

## Risks

- post-commit の process 停止は workflow fact と原子的ではない。停止失敗時は R-005 に従って NodeExecution の終端を維持するため、その process が枠を占有し続ける可能性がある。本変更は durable retry や restart reconciliation を追加しない。
- provider lifecycle commit は canonical workflow facts と原子的ではない。canonical facts 確定後の失敗は R-006 に従って Stop の受理を維持するため、provider lifecycle event が欠落し得る。本変更は retry や診断情報の永続化を追加しない。
