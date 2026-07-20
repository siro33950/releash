# Agent チャット表示（presentation）の理想形

作成日: 2026-07-07
更新日: 2026-07-19

milestone 84「Agentチャット安定化」のドキュメント群:

- [agent-chat-instability-audit.md](agent-chat-instability-audit.md) — 問題点インベントリ（全 66 件、要求リスト）
- [agent-chat-ideal-vocabulary.md](agent-chat-ideal-vocabulary.md) — 正規化語彙・データ構造の理想形
- [agent-chat-ideal-lifecycle.md](agent-chat-ideal-lifecycle.md) — ライフサイクルの理想形（不変条件）
- **agent-chat-ideal-presentation.md（本書）** — UI 表示の理想形
- [d3-durable-event-store-design.md](d3-durable-event-store-design.md) — durable event store の物理設計 gate
- [close-quit-decision-table.md](close-quit-decision-table.md) — close / quit surface の表示結果正本

本書は「backend が持つ情報を、どの surface に、どう表示するか」の正本を定義する。監査で確定した presentation 問題群（FE 群）と、語彙拡張で新たに表示対象になる要素（thinking / plan / notice / stop reason / usage 等）の表示先を確定する。frontend は Rust backend が所有する read model の mirror であり（CLAUDE.md 原則 / ST-8）、本書は「何を映すか」を決める文書であって frontend にロジックを置く根拠にはしない。

## 表示原則

- **P1 (live / reload 等価)**: 画面は read model のみから描画できる。transient event は read model 更新を早く届ける手段であり、live と reload 後で表示が変わってはならない。streaming delta は seq で順序・欠落を検出し、欠落時は snapshot 再取得で自己修復する（FE-3、seq 契約は語彙文書 §11）。durable workflow/session stateの唯一の例外は `Notice(PersistFailure)` — 永続化自体の故障を知らせるため transient 表示を許容する（lifecycle I8）。state transition前のtyped `SessionOperationFeedback`は履歴/read modelではなくcommand feedbackであり、別のtransient契約として明示する。
- **P2 (表示先の一意性)**: 各語彙要素に primary surface を 1 つ定める。同一情報の二重描画を禁止し、補助 surface は要約・導出値のみを表示する。
- **P3 (無言遷移の禁止)**: ユーザーに観測可能な状態変化（turn 終了・失敗・中断・permission 失効・queue 変化・Agent 設定変更）は、必ず画面上の変化を伴う。「スピナーが消えただけ」「reload したら突然エラーが現れる」（FE-2）を許さない。
- **P4 (スコープの一致)**: バナー・エラー表示は対象スコープ（session / turn / app）の surface にのみ表示する。session を跨いだグローバル状態での表示を禁止する（FE-5）。
- **P5 (入力の保全)**: 送信失敗・queue 操作・stall のいずれでも、入力欄の内容と添付は消えない（lifecycle I6 の表示面）。通常sendでは送信開始時にcomposer snapshotとcaller生成のstable opaque `operation_id`を対応付け、既存の`send_agent_message`へ現行6 target field＋content / images / mentions / editor context / active-turn policyのexact payloadを渡す。immutable Accepted receiptを受け取った場合だけ、そのoperation IDに対応するsnapshotをclearする。`RejectedBeforeCommit`、`OutcomeUnknown`、typed `PayloadConflict` application errorでは保持し、待機中に追加入力された新しいsnapshotを消さない。response喪失後は同じoperation IDと同じpayloadで再照会または再実行し、別operation IDを自動生成しない。Accepted後に`SendExecutionStatus`が`ReconciliationRequired`へ進んでも受理済みsnapshotを復活・再送せず、receipt後のquery / emit failureをsend失敗へ読み替えない。active-turn steerもdurable `TurnSteerRequested` receipt後だけclearし、ResultUnknownを再クリックで暗黙再送しない。
- **P6 (監査可能性)**: 後からセッションを開いた読者が「何が実行され、どの mode / Goal / 推論レベルだったか、なぜ止まり、何が拒否され、いくら使ったか」を durable read model と履歴から読み取れる（RG-4 / FE-7 / CL-1 effective / RG-9 / #1445〜#1451）。

## Surface 定義

| ID | surface | 実体（現行） | 責務 |
|---|---|---|---|
| S1 | transcript | `ChatSessionView` | parts の時系列描画。会話の正本表示 |
| S2 | activity log | `ActivityLog` | 実行中 turn の tool 進行の要約ストリーム |
| S3 | todo フッター | todo 表示部 | 最新 `TodoListSnapshot` の常時表示 |
| S4 | permission UI | `PermissionDialog` ＋ transcript 内 permission part | 回答の受付と履歴表示 |
| S5 | 入力エリア | `MessageInput` ＋ queue chips | 入力・送信・queue 操作 |
| S6 | session バナー | チャットパネル上部（session-scoped） | セッション単位の警告・エラー |
| S7 | usage indicator | 入力エリア上部（`MessageInput` 上縁、新設） | token / context / cost の常時要約 |
| S8 | session バッジ | セッション一覧・タブ | セッション状態（Running / Waiting / Error）の要約 |
| S9a | launch configuration | New Agent dialog（新設） | preflight→prepared reservation/challenge→durable `AgentLaunchAttempt`。provider-ack済みSession stateとは分離 |
| S9b | Session 設定 strip | `MessageInput` 上部（新設） | selected/effective configuration、current Goal、各pending/reconciliation、protocol compatibilityのprimary surface |
| S9c | workflow Agent 設定 | workflow step editor（新設） | model/modeのInherit/Setとeffort/initial GoalのInherit/Set/Clearを編集。Session stateとは別artifact |
| S10 | application shutdown overlay | 全window共通のbackend-owned progress surface | Application planのPreparing / Prepared / Activated / Quiescing / Completed / Failed / Cancelled / ReconciliationRequired、safe summaryを表示。専用`ApplicationShutdownAction::RetryQuit`はsame-boot pre-activation effect-0 Failed＋durable fence＋admission Open＋store Healthyだけ |

## 語彙 × 表示先マトリクス

vocabulary 文書の語彙要素ごとに primary / 補助 surface と表示規則を定める。

| 語彙要素 | primary | 補助 | 表示規則 |
|---|---|---|---|
| `Text` | S1 | — | 現行通り。streaming 追記 |
| `Thinking` | S1 | — | 折りたたみブロック（streaming 中は自動展開、完了で自動折畳）。**backend 共通**（CX-3/RG-1: Codex でも表示）。`redacted` は「非公開の思考」プレースホルダ。Task 配下（`parent_tool_use_id` あり）も Task 展開内に描画する（FE-6） |
| `ToolCall` | S1 | S2 | kind 別アイコン、`status` バッジ（Running spinner / Succeeded / Failed / **Denied / TimedOut / Interrupted を色・文言で区別**（RG-4））、`exit_code` バッジ（RG-8）、output は text ＋ **image**（CL-6/RG-7）を描画。Running 中の出力は追記表示（SD-5）。WebSearch は query と結果要約を表示（CX-11） |
| `TodoListSnapshot` | S3 | S1 | `in_progress` 項目をハイライト（スピナー付き）、priority 表示（RG-5）。Claude / Codex 共通（CX-5/RG-2） |
| `Notice` | kind による（下表） | — | 下記「Notice 振り分け」 |
| `SessionOperationFeedbackSnapshot` | S6 | — | Rustが返す未解決failureだけのidentity-keyed collectionを`GetSessionOperationFeedbackRequest { session_id, cursor, limit: 1..=32 }`で描画し、successをentry化しない。process全体512件の未解決identityをcoalesce / evictせず、別session、同kind別attempt、古いsuccessではclearしない。`resolves_feedback_id`一致または`feedback_id + expected_revision` dismissだけで対象entryを消す。Loadを含むdomain read / mutationのslot予約不能による`FeedbackCapacityExceeded`は対象へ作用前、mutation effect 0件としてRust-owned解決操作と共に表示する。`GetOperationFeedback / DismissOperationFeedback / RetryOperationFeedbackResolution`はcapacity gateから除外されたTauri / WebSocket共通control planeなので、512件飽和時にも操作可能にする。dismiss成功、retry成功、retry再失敗は`Applied { snapshot }`の更新後snapshotを表示し、unknown / stale / action不正の`Rejected { NotFound / RevisionConflict / ActionUnavailable }`では現表示を維持する。control自身のstorage failureは`Failed { failure }`として表示し、新feedback entryを合成しない。dismiss成功は1件減、retry再失敗は同じidentityのattempt / failure / actions / revision更新として表示し件数を増やさない。close / archiveだけで未解決entryを消さない。frontendでraw error分類・clear規則・capacity policyを実装せず、durable transcriptへ自動変換しない |
| `AgentSendOperationView` | S5 | S6/S8 | `Accepted { receipt, status, obligations, available_actions }`または`OutcomeUnknown { operation_id }`をbackend projectionのまま描画する。Acceptedはimmutable receiptと最新のmutable statusを分離し、restart後も同じoperation IDから再取得する。`SendExecutionStatus::ReconciliationRequired`や`Failed`へ進んでもtop-level Acceptedとreceiptを維持し、未受理へ戻したり自動再sendしたりしない。未知operation IDの`NotFound`、保存結果不明のembedded `OutcomeUnknown`、同じcaller operation IDへ異なるpayloadを渡したtyped `PayloadConflict { identity: Send { operation_id } }` application errorを区別する。`PayloadConflict`をSafeOperationFailureやsend result variantへ合成しない。`RejectedBeforeCommit`はdurable viewを作らない即時resultであり、frontendがsynthetic operationを生成しない。obligation observationとavailable actionsはRustが返したものだけを表示し、frontendで状態・failure・retry可否を合成しない |
| `SendAcceptanceReceipt` | S5 | S1 | immutableな受理事実としてoperation ID、session、backend発行opaque `input_ref`、`StartedTurn`または`Queued`のdispositionを表示し、provider establish待ちでも受理時の値を変えない。`input_ref`をclientで生成・解析せず、replay/queryで同じ値を保持する。一度表示したreceiptをstatus失敗でRejectedへ戻さず、対応するcomposer snapshotだけをclearする。receipt dispositionをprovider startedの証拠に使わない |
| `SendExecutionStatus` | S5 | S6/S8 | AwaitingProviderStart / Queued / ProviderStartReserved / Running / ReconciliationRequired / Failed / Terminalをreceiptと別に更新する。複数obligationの観測はbackendがcommitted manifest overlay＋最大4 direct lookupで合成した各obligation projectionから表示し、operationへ単数provider observationを合成しない。dependencyはbackendのimmutable fixed-kind DAG / max depth 1をそのまま表示し、Turn / Queueには同owner・同operationのProviderEstablish 0..=3件だけを許す。dependencyを満たすのは`Terminal(Succeeded)`だけで、ProviderEstablishがそれ以外またはpending中はreceiptのStartedTurn / Queuedを維持したままstatusをAwaitingProviderStartとして表示し、ProviderStartReserved / Running / provider-startedへ進めない。frontendで任意graphを探索・補完しない。ReconciliationRequired / Failedはreceiptを維持し、same-effect readback / Rust-owned手動解決だけを提示して自動sendしない |
| `RecoveryPublication` obligation projection | S6 | S1 | recovery messageのPending / ReconciliationRequired / Failed / `Terminal(Succeeded \| CancelledBeforeEffect \| Superseded \| FailedTerminal)`を独立identityで表示する。これはexternal effect obligationではなく、BackendRecovery completionでPendingになりlocal publication claim後もEffectReservedを通らない。BackendRecovery statusやclaim取得からpublication完了を推測せず、BackendRecovery側はpublication obligationへの参照だけを表示する。messageはpublication markerと同じclosureで`Terminal(Succeeded)`になったprojectionから一度だけS1へ出す |
| `Error`（part） | S1 | S6 | `retryable=true` は「再試行中」表示にし、`resolved=true` へ更新されたら成功扱いに畳む（CX-8: 恒久の赤エラーにしない） |
| `Permission` | S4 | S1 | 下記「permission UX」 |
| `TaskStatus` / Task 配下 | S1 | S2 | Task 展開内に thinking / tool / 未 pair の tool result も描画。要約の 200 字切りは全文展開可能にする（FE-6） |
| `TurnResult` | S1 | S8 | 下記「turn 終端の表示」 |
| `TokenUsage` | S7 | — | context 使用率バー＋ token 数＋ cost（FE-4/RG-9）。turn 中は `TokenUsageUpdated` で逐次更新 |
| `AgentMode` | S9b | S8 | `Ask / Edit / Plan / Auto / Bypass` を排他的 selector で表示。Plan toggle は置かない。Auto は provider reviewer の範囲、Bypass は危険性を常時明示 |
| `ReasoningEffort` | S9b | — | UI名は「工数（推論レベル）」。selected ProviderDefault/Explicit、effective Known(value/source)またはUnknown(selected/expected/reason)、runtime availability、option/description/defaultを広告順で表示し、TokenUsage/cost/budgetと同じcontrolにしない |
| `SessionGoalProjection` | S9b | S6/S8 | current Goal、pending transition、sync state、latest evidence、Rust評価済みavailable actionsとstrategy/scope/effectsを表示。Completed/Failedは根拠付きoutcome |
| `AgentSessionConfigurationProjection / TurnStartState` | S9b | S6 | selected/effective provider/model/mode/effort revision、provider generation、pending/observation/recovery/resolution、Rust評価済みconfiguration available actions、StartingTurn/ReconciliationRequiredを描画。activation前をeffective表示しない |
| `AgentLaunchPreflight / PreparedAgentLaunch / AgentLaunchAttempt` | S9a | S6 | Checking→Compatible→draft reservation/challenge→startを表示。draft変更でreservation失効。full projection＋after_seq watchでreload/reconnect復元 |
| `AgentConfigurationTemplate / ResolvedLaunchConfiguration` | S9c | — | baseline / WorkflowExecution default / NodeDefinition override、revision、field provenanceを表示し、解決previewはRust queryの結果だけを描画 |
| `BackendProtocolIdentity` | S9b | S6 | 互換なら通常は詳細内。schema/binary/flag不一致はProtocolIncompatibleとしてerror表示しsendをdisable |
| turn configuration revision | S1（turn 詳細） | — | TurnStartedのimmutable effective snapshotからprovider/model/mode/effective effort/unknown reason/Goal/protocol identityに加え、当時のprovider permission/effects/residual protections/context hashを展開し、後から監査可能にする |
| `StopOperationView` / turn_phase / stall | S1 末尾 ＋ S8 | S2 | `StopOperationView`はclosed `Accepted { receipt } / Terminal { receipt, resolution, result } / ReconciliationRequired { receipt, failure, available_actions } / OutcomeUnknown { operation_id }`の4 variantだけを描画する。未知operation IDの`NotFound`と、same request ID / different targetのtyped `PayloadConflict { identity: Stop { request_id } }` application errorをviewへ合成しない。StartingTurnは「開始を確定中」、Streamingはspinner、Interruptingは「停止中（最大10秒）」。same-target重複Stopは同じ進捗へjoinし、distinct未完了32件上限またはdeadline schedule不能はAccepted表示にせず`StopCapacityExceeded`を表示する。10秒でReconciliationRequiredになったStopもterminal result確定までは32件occupancyへ数えるbackend projectionをそのまま示す。PendingTerminalCommit / Recovering / ReconciliationRequiredは通常Idleと区別し、理由とRust-owned available actionsを表示してsend / queue drainを止める。stall診断で観測停止を表示する |
| `SessionLifecycleResult / SessionLifecycleOperationView` | S6 | S8/S9b | view close以外のClose / ArchiveOpen / ArchiveClosed / SwitchBackendはTauri専用operation projectionをそのまま表示する。Acceptedはbackend発行opaque operation ID、session、normalized action、first accepted expected revision、accepted_atを含むimmutable receiptとInProgress / ReconciliationRequired / Completedを分離する。response喪失・reload後も同じoperation IDをqueryし、Accepted後のfailureを未受理へ戻さない。same request ID / different commandの`PayloadConflict { identity: SessionLifecycle { request_id } }`、異action競合の`PendingOperation`、Busy / RevisionConflict / InvalidState、cross-principal / unknown operationのNotFoundをembedded stateへ合成しない。別request ID / same unresolved actionのjoinは同じreceipt / progressを表示し、別operationやdeadlineを作らない。10秒後も未確定なら同receiptのReconciliationRequiredとRust-owned actionsを表示する。Completedは保存済みClosed / Archived / BackendSelected outcomeをexact replayし、frontendでcurrent sessionから再構築しない。BackendSelected.runtime_startedは次sendまでfalse、closed archiveのqueue_pausedはbackend値をmirrorする。view closeではこの進捗、request ID、operationを一切表示しない |
| `TurnSteerRequested / TurnSteerAccepted / TurnSteerRejected / TurnSteerReconciliationRequired` | S5 | S1 | Requested / Accepted / Rejected / ReconciliationRequiredを同じsteer idのbackend read modelから表示する。結果不明時はRustが返すavailable actionsだけを提示し、入力を自動再steer/queueしない |
| queue | S5 | — | chips: Queued / AwaitingBypassConfirmation / Starting / Started / Paused / Failed / Cancelled / NeedsResolutionを可視化。通常Completedかつ既にunpausedの場合だけ次itemの評価を表示する。Stop、active / Idle close、open archive、backend switch、quit、Fatal、Crash、recoveryではpausedを維持する。Queued acceptanceでbackendが固定したexecution ID / reserved turn ID / snapshotを表示用identityとして使い、drain時にfrontendでturn IDを採番しない。acceptance後のruntime / configuration generation driftでpredeclaredされていないProviderEstablishが必要ならNeedsResolutionを表示してauto startせず、#1404のRust-owned CAS付きrebase / cancelだけを提示する。backend switch後に旧snapshotと不整合なitemもNeedsResolutionとし、自動drainしない。snapshot/goal ref/current差とexecution idを表示し、Bypass確認の期限切れは再prepare、その他の解決不能差はCAS付きrebase/取消を提示 |
| session 状態 | S8 | S6 | Error 時は理由（最後の Fatal / TurnError の要約）を tooltip で表示（RT-6）。reload 後も理由が残る（durable Notice 由来） |
| `ApplicationShutdownProjection / ShutdownPlanPage` | S10 | S8 | quit開始時にopenなactive / Idle Sessionと進行中Workflowだけをtargetにし、Preparing / Prepared / Activated / Quiescing / Completed / Failed / Cancelled / ReconciliationRequiredの8 phase、summary、counts、absolute deadlineをbackend projectionから表示する。current surfaceはTauri `get_application_shutdown` / WebSocket `GetApplicationShutdown`のclosed `CurrentApplicationShutdownResult`だけを使い、exact `Current(Option<ApplicationShutdownProjection>) / OutcomeUnknown { failure }`を区別する。resultを安全に構築できない内部障害は別の`GetApplicationShutdownApplicationError::Internal { correlation_id }`として表示し、`Current`、`OutcomeUnknown`、`None`へ合成しない。hash-validなcomplete rootがexactly oneでplan ID / epoch / exit intentを一意にanchorでき、pointer等の冗長semantic identityだけが矛盾する場合に限って`ShutdownAuthorityMismatch`を持つ同identityのReconciliationRequiredとして表示する。storage / decode / envelope・self-hash / pointer-to-root hash failure、required record欠損、state composite・activation lineage integrity failure、identity unanchorable / ambiguousは`Internal`であり、query failureや保存結果不明を通常起動へ隠さない。same-boot current flightへの後続quitは同じresultへjoinする。未解決previous shutdownまたはscope fenceが残るnew quitは`PreviousShutdownReconciliationRequired`、retiring detail競合は`PreviousShutdownCompactionPending`としてAccepted前に表示し、新planやeffectを作らない。plan action `RetryQuit`はsame-boot pre-activation effect-0 Failed、durable fence、admission Open、store Healthyを同snapshotで証明できる場合だけ表示する。各Session deadlineは`min(executor開始 + 10s, T0 + 13s cutoff)`、global exitは15秒である。`GetShutdownPlan`はstable orderで1 page最大128件 / encoded 1 MiB、全体最大4096 targetを返し、`QueryBusy / DeadlineExceeded / CursorExpired`時にpartial pageや別revisionを継ぎ足さない。frontendはtarget key、ordinal、state hash、counts、result、actionを再計算せず、Rust projectionをmirrorする。terminal resultはSucceeded / CancelledBeforeEffect / Superseded / FailedTerminalを区別し、exit evidence後の未解決targetは`ExitCoupledOutcomeUnknown`付きReconciliationRequiredとして表示する。関連runtime / childはowner targetのsubordinate effectであり別targetへ数えない。closed / archived Sessionとdurable OrphanRuntimeはtarget上限へ含めず、current pending queryとplan-pinned pending snapshot queryを別authorityとして表示する。target / pending actionは共通resolverへbackend発行action IDとguardをopaqueに返し、frontendでretry可否やprovider resultを構築しない。resolverの`Completed / InProgress / Rejected(NotFound / RevisionConflict / ActionUnavailable / TargetRevisionChanged) / ActionOutcomeUnknown`をそのまま表示し、writer unknownをtransport errorや失敗確定へ読み替えない |
| `ApplicationQuitOperationView / ApplicationQuitProjection` | S10 | S8 | backend発行のopaque `ApplicationQuitOperationId`を使い、Tauri `get_application_quit_operation` / WebSocket `GetApplicationQuitOperation`からknown quit operationを表示する。top-level viewはexact `Accepted { operation_id, current } / Terminal { operation_id, projection } / OutcomeUnknown { operation_id, intent }`である。`Accepted.current`はOptionを持たない`CurrentApplicationQuitOperationResult::Current(ApplicationQuitProjection) / OutcomeUnknown { failure }`であり、closed `ApplicationQuitProjection`の`Shutdown / Bootstrap`を区別する。direct record確定前のacceptance writer結果不明だけをtop-level `OutcomeUnknown`、durable direct locator先のcurrent transaction outcome不明だけを`Accepted.current`内の`OutcomeUnknown`として表示する。`Terminal`のShutdown branchはterminal history、Bootstrap branchは`Exited`だけに使う。未知operation IDの`NotFound`と、known direct recordのlocator欠損、Shutdown locatorのlive / immutable archive双方不在またはparity不一致、Bootstrap flight欠損、storage / decode / integrity failureによる`Internal { correlation_id }`はtop-level application errorとして表示し、view/result variantへ合成しない。archive-only normal locatorは同じTerminal / Compacted projectionを表示し、bootstrap-safe flightをnormal shutdown plan、`CurrentApplicationShutdownResult`、`Current(None)`へfallbackしない |

`ApplicationShutdownProjection / ShutdownPlanPage / ShutdownSummary`はfirst accepted ingressから不変のcanonical `ShutdownExitIntent { mode, code }`を同じbackend authorityから返す。UIは各surfaceをbackendが正規化した結果だけを表示し、frontendでmode / codeを再推測しない。同じrequest ID / different intentはembedded shutdown resultやSafeOperationFailureではなく、`ApplicationQuit { request_id }` identityを持つtyped `PayloadConflict` command failureとして表示する。別request IDでsame flightへ到着した後続quitはfirst intentの同resultへjoinするため表示中intentを変更せず、pre-activation effect 0 abort後のnew flightだけを新intentとして表示する。`ShutdownExitIntentV1`はadaptor DTOでありUI stateの正本にしない。

new flightにeligibleなprior latestはCompleted、またはdurable Failed / Cancelled terminal fenceと同じidentity / counts / safe failure / exit intentを持つeffect 0件の`AbortedBeforeActivation` terminal planだけであり、同時にadmission Open、store Healthy、unresolved shutdown scope fence 0を満たす。pre-activation terminal closure直後は`details=Available`としてfull target detail / pinned snapshotを表示できる。new flightのroot-initはpriorがAvailableかつretiring Noneならpriorをretiringへreserveしてnew Preparing / latest pointerと同時確定し、retiringが既にある場合は`PreviousShutdownCompactionPending`を表示してnew planを作らない。priorがCompactedならarchive parity検証後に進む。background compactorの`ArchiveSwitch`成功後は、detail detach途中でもold-plan queryをarchive-onlyの`details=Compacted`、entries空、next cursorなしとして描画し、Availableへ戻さない。`DetailDetachChunk`（F3では`InventoryDetachChunk`）と`FinalizeDetach`はbackend内部のbounded cleanupであり、crash resume中もretiring状態を保持し、Phase 0は全page absence後の0-page dedicated closureだけclearし、future F3 SQLはlast atomic page batchまたはpage 0 transactionでclearする。process-only Failed、Stalled / unhealthy store、admission Closed、scope fence残存はexplicit retryでもnew planを表示しない。`RetryQuit`もsame-boot effect 0 Failedとこれら全guardを満たす場合だけ表示する。

shutdown snapshotは最大4096 target、1 page最大128 entry、encoded 1 MiB以下という公開上限だけを表示する。各上限超過は`CapacityExceeded`または`ResponseTooLarge`としてnew plan / partial page / effectを表示しない。物理packやpage配置はD3の実装設計であり、UI契約へ露出しない。

Tauri / WebSocketでRust u64へ写像するepoch、revision、sequence、ordinal、count、offset等のsemantic fieldは`0`または先頭ゼロのないcanonical decimal stringとして受け取り、その文字列をopaqueにmirror / echoする。bounded transport controlの`limit: u16` / `max_bytes: u32`はJSON nonnegative integer、shutdown exit codeの`i32`はJSON signed integerとして扱い、decimal stringへ変換しない。frontendはJavaScript number / `BigInt`への変換、locale format、rounding、leading-zero補完をauthorityにせず、表示用整形が必要でもcommand guardには受信した原値を使う。`9223372036854775807`まで両surfaceで同じsemantic値を保ち、semantic fieldの非canonical値、control / exit codeのstring・fraction・型 / route範囲外、one-based semantic fieldの`0`に対する`InvalidRequest`、current maximumからの`CapacityExceeded`を別のrevisionや再試行成功へ読み替えない。

S10 / S6が表示するpending countとpageは、backendの3-tree inventory contractをそのまま使う。canonical primaryだけが各pending obligationを`OpenSession / WorkflowExecution / ApplicationShutdown / ClosedSession / ArchivedSession / UnownedRuntime`の1 partitionへ置いてcountし、actual-owner secondaryとimmutable shutdown-association secondaryを重複countしない。All / Partitionはprimary、Ownerはowner secondary、ShutdownPlanはassociation secondaryからprimaryへbounded lookupする。backendのrange / root hashはcomposable Merkle nodeからO(path)で得るため、frontendは全entryを収集してordered hash / countを再計算しない。同じentryをfilter横断で別identityとして数えたり、Session lifecycleからApplicationShutdown associationを再分類したりしない。

Pending / Terminalのcanonical stateは別のobligation-state-by-ID COW root、cross-plan duplicate gateは`UnresolvedShutdownScopeFenceV1` COW rootにある。common pending queryはtransaction inventory、pending envelope、state root、latest-activated pointerをbase `Phase0ReadSnapshotRef`へpinする。shutdown projection / action / compactorは同じcommit間隙でscope-fence rootとlatest-attempt pointerを追加した`Phase0ShutdownReadSnapshotRefV1`を使い、baseだけからcurrent candidate / count / actionを合成しない。frontendはpending discovery entryだけからterminalを推測せず、snapshot leaseの`CursorExpired`後に旧pageとfresh pageを連結しない。`PreviousShutdownReconciliationRequired`はsame owner＋exact scope fenceが残るRust-owned rejectionとして描画し、frontendでowner一覧から独自判定しない。

Phase 0 bridgeのautomatic bootstrap中は固定legacy sourceから既存Session / transcriptをread-onlyで表示し、Tauri `get_phase0_bootstrap` / WebSocket `GetPhase0Bootstrap`の同じ`Option<Phase0BootstrapProjection>`をapplication-level bannerへ描画する。`InspectingSource / Importing / Verifying / Activating / Failed`、finalize済みimported / optional total count、`read_only=true`をbackend値のまま表示し、source内deterministic substepのpartial count / staging projectionを表示しない。Failedだけsafe failureを表示する。pointer CAS後もPhase0 pointer / root / parity検証、reachable manifest replay、pending inventory validation、normal read / mutation admission openまでは同bootstrap IDのActivatingを表示し、その完了後に返るNoneだけをbootstrap完了として扱う。storage / decode / pointer OutcomeUnknownを完了へ変換しない。bootstrap projectionがSomeの間はsend、permission response、Session close / archive、backend switch、workflow mutation、recovery actionをdisabled reason付きで無効にするが、application quitは無効化せずbootstrap-safe bounded exitへ渡す。このquitではnormal shutdown plan / target / obligation、明示agent / workflow commandを表示せず、backend発行opaque `ApplicationQuitOperationId`で取得した`ApplicationQuitProjection::Bootstrap`からfirst accepted ingressのexit intentを固定したspecial flight、current bootstrap step / pointer attemptのsettle、同intentの15秒one-shot exitだけを表示する。known operationを`Current(None)`やnormal shutdownへfallbackしない。後続quitの別mode / codeへ表示を上書きしない。frontendがstaging root、source parity、bootstrap完了、OutcomeUnknownの結果を推測しない。scope quarantineは該当Session / Workflow / orphanだけにsafe recovery表示を出し、private quarantine blob ref / exact raw bytesは公開せず、別scopeのdataで補完しない。None確定後はPhase0 projectionだけへ一度切り替え、legacy / Phase0 resultをmergeまたはrecord単位fallbackしない。physical identity / pathもfrontendで組み立てない。

send由来のTurnExecution / QueueExecution / ProviderEstablishは同じcaller指定`SendOperationId`をsemantic correlationへ固定し、RustがTurn / QueueとProviderEstablishの一致およびcompact Terminalのdependency bindingを検証する。frontendはoperation IDをtransport request IDと混同せず、別operationのSucceeded ProviderEstablishを再利用可能と表示しない。correlation hashやexact dependency bindingはbackend recovery authorityであり、public viewにはsafe lifecycle / observation / actionだけを出す。

全9 obligation kindのpayload / effect / owner / optional fieldはRustのclosed matrixだけが解釈する。UIはTurn / Queueのturn・operation Some、TerminalCommitのturn Some / operation None、PermissionDeliveryの両field None、ProviderEstablishのturn None / operation Some、残り4 kindの両field Noneを補完しない。Turn / Queue / Permission / ProviderEstablish / BackendRecovery / WorkflowShutdownは各matrixの5 pending lifecycle、TerminalCommitはEffectReserved / ReconciliationRequired / Failedだけ、RecoveryPublicationはPending / ReconciliationRequired / Failedだけを表示する。SessionClose effect / scope Noneはpending表示せず直接Terminal(Succeeded)、RecoveryPublicationはexternal effect / dispatch fence / provider proofとEffectReserved表示を持たない。backendがwrong-kind / wrong-owner / wrong-association recordをquarantineした場合はsafe failure / resolution actionを表示し、別kindへ見せかけない。

started turnまたはqueued由来で既にstartedになったreserved turnのterminal表示はbackendの単一winner closureだけに従う。StopのInterrupted(User / Timeout)がwinnerならTurnExecution / QueueExecution umbrellaとTerminalCommitを共にSucceeded、normal completion / Fatal / SessionClosed /競合terminalがwinnerならumbrellaをSucceeded、TerminalCommitをSupersededとして表示する。UIはumbrellaのSucceededをprovider業務結果の成功へ読み替えず、unstarted queue itemのcancelをこのwinner表示へ混ぜない。

recovery action IDはbackendが発行するopaque identityであり、frontendはparse・検証・再生成せずprojection値をcommandへそのまま返す。action decisionはcurrent resourceやshutdown detailsより先にidentityでlookupする。Completedなら保存済み`RecoveryActionReceipt`とcanonical safe resultをexact replayし、current stateの前進やplan detail compactionを理由に再実行しない。decision Absentのfresh executionまたはnonterminal joinだけがcurrent revision / root / state guardを検証する。`OutcomeUnknown`は同attemptのresolve中として表示し、successへ推測変換しない。別action IDのrevision conflictだけを最新projection再取得へつなぐ。action tokenの物理encoding、key rotation、将来のdata deletion契約はD3 / F3側の設計事項であり、本書のPhase 0表示契約に含めない。

authoritative observationはprovider / runtime / workflow adaptorが認証済みresponseまたはcanonical eventから作り、UIはpublicなsafe observationとproof digestだけを表示する。proof ID、private blob ref、provider raw evidenceを表示せず、clientからproof outcomeを送らない。action observationがeffect開始 / ackだけならbackendはobligationをPendingのままproof ref / safe observationとowner / operation statusを同時更新し、UIは`RecoveryActionOutcome::Pending`として表示してCompletedを推測しない。terminal proof時もcurrent obligation revision / effect / correlationへ束縛したproofをkind-specific closureが採用し、side stateとcompact resultが同時確定した後だけTerminal表示へ進む。side participant failure時は元pending viewを保つ。QueueExecutionにはPhase 0のCancelIfSafeを表示せず、cancel / rebaseは#1404のqueue操作だけを使う。

shutdown targetはbackendのclosed `ShutdownTargetView`をそのまま描画する。`Completed`表示はbackendの`terminal_result=Some`を必須とし、Succeeded / CancelledBeforeEffect / Superseded / FailedTerminalをresult labelへ保つ。FailedTerminalだけsingle safe failureを表示し、他3値ではfailureを表示しない。derived CancelledBeforeActivation / Supersededはterminal resultなし、ReconciliationRequiredはresultなし＋保存済みreasonだけを表示する。public stateだけからresult / failureを推測せず、backendが返すtarget-state guardをaction requestへopaqueに返す。

obligationがAbsentのlogical shutdown targetでもresult単独を作らない。Succeeded / ConfirmedNoEffectはSessionClose / WorkflowShutdown owner projection、compact result、action attempt Completed receipt / safe resource resultが同じtarget closureで確定した後だけCompleted表示する。AmbiguousはresourceをPending(ReconciliationRequired)として表示する一方、readback action自体はCompleted / outcome=Pendingの保存済みreceipt / canonical safe resultをsame action IDへexact replayし、action失敗へ読み替えない。attempt ReconciliationRequiredはaction自身のresult / commitが未確定の場合だけ表示する。side participant failure時はlogical Prepared / ExitCoupled viewを保ち、frontendでterminalを補完しない。explicit retryはreservation表示後もbackendの`EffectDispatchGate(effect_id)`がterminal Absence、claim boot / lease、effect / intent hash、state / dispatch fence / scope fence / external action attempt generationをcall直前に再検査し、non-async / non-blocking handoffでcancellation-shielded owned driverのstarted handleをregistryへ登録できた場合だけ開始する。EffectReservedやregistry登録前を「effect呼出済み」と読み替えず、登録不能のeffect 0件failure、terminal / reclaimとのfirst-winner、gate解放後のabsolute deadlineをbackend projectionのまま表示する。

Pinned snapshotの`ClosedSession / ArchivedSession / UnownedRuntime`はcount 0を含むexactly 3 rangeなので、いずれのpartitionもvalidでempty rangeは空pageとして描画する。unknown tag / limit範囲外のInvalidRequest、rootとのsnapshot ref不一致のSnapshotMismatch、別plan / snapshot / partitionへcursorを再利用したCursorMismatch、CursorExpired、DetailsCompactedを別stateへ読み替えない。

recovery actionを押した後はaction IDを操作identityとして進捗を表示する。response喪失後のsame action IDは新しいretryとして再生成せず、backendのdurable action decisionへjoinしてCompleted / ReconciliationRequired / OutcomeUnknownを取得する。異なるstale action IDにだけRevisionConflict / ActionUnavailableを表示して最新projectionを再取得する。ReadAgain / RetrySameEffect / UseObservedResult / CancelIfSafeの可否、idempotency / readback / proof判定、logical Prepared targetからのsame deterministic obligation生成はRust projection / resultだけを描画し、frontendで推測しない。最後のtarget actionではtarget result、kind-specific owner side state、plan `Completed`、latest-attempt pointer、summaryが一つのguarded closureで確定した場合だけterminal planを表示し、last-target resultだけのpartial commitを完成扱いしない。全target terminalでnon-success / preexisting recoveryがあってもplan stateはCompleted、summary outcomeはExitedWithRecoveryと表示する。

local `UseObservedResult | CancelIfSafe | KeepForManualResolution`はbackendがresource / kind-specific side stateとaction attempt `Completed` receipt / safe resultを一つのclosureで直接確定した結果だけを表示する。UIはlocal actionにPrepared / EffectReservedの中間進捗を表示せず、claim / effect fenceを合成しない。`KeepForManualResolution`はresourceを変更しない`Completed + outcome=Unchanged`としてsame action IDへexact replayし、後続の別mutating actionを妨げない。

Phase 0の公開表示・操作にはresource-isolated input、operation / resource privacy purge、managed backup / restore、app-data reset、complete privacy authority reset、export / importを置かない。これらはD3 / F3後続のdata lifecycle設計事項であり、private ref、proof digest、quarantine retentionからfrontendが削除・復元・退避操作を合成しない。

### Notice 振り分け

| NoticeKind | 表示先 | 規則 |
|---|---|---|
| Compaction | S1 inline | 進行 → 完了 / **失敗**（SD-7）まで単一ブロックで遷移 |
| ModelRerouted | S1 inline ＋ S6（一時） | 「モデルが X に変更された」を明示（CX-7） |
| ConfigWarning / Deprecation / GuardianWarning | S6 ＋ S1 inline | セッション開始時警告はバナー、turn 中発生は inline（CX-7/RG-6） |
| RateLimit | S7 に反映 ＋ S1 inline（閾値超過時のみ） | 最新値は usage indicator の導出値として表示（RG-6） |
| McpServerStatus | S6 ＋ S1 inline | 接続失敗・認証切れを明示（CL-5） |
| UnsupportedMessage / classified OversizeDropped | S1 inline（低強調） | content-planeと確定できた「未対応の応答をN件受信」など。未分類/malformed/oversize frameはS6のProtocolIncompatible（V-P1） |
| ProtocolIncompatible | S6（error）＋S1 inline | control-planeのschema/binary/flag/capability drift。protocol identityと対象を表示し、新規sendをdisableする |
| PersistFailure | S6（error） | 保存失敗はバナーで明示（I8）。永続化故障を知らせる性質上、transient 表示を許容する唯一の例外（P1） |
| Diagnostic | S1 inline | stall 等。転記される診断はユーザーが症状報告に使える文面にする |

### turn 終端の表示

`TurnResult` の表示は stop_reason / 失敗理由を区別する（CL-3/RG-3 の解消）:

| 終端 | 表示 |
|---|---|
| Completed{EndTurn} | 現行通り（明示表示なしで Idle に戻る） |
| Completed{MaxTurns} | 「ターン上限で停止。続行できます」＋続行アクション（送信欄に定型文） |
| Completed{MaxTokens} | 「出力上限で途切れました」＋続行アクション |
| Completed{Refusal} | 「モデルが応答を拒否しました」を明示（workflow へは failure_signal） |
| Failed{TurnError} | S1 に Error block（**live で即時**。reload 後にだけ現れる状態を禁止 — FE-2）＋ S8 バッジ |
| Interrupted{UserAbort} | 「停止しました」チップ |
| Interrupted{Timeout/Crash/SessionClosed} | 理由付き中断チップ（I1/I2 の回収結果を含む） |
| stats | duration / cost / num_turns を終端チップの詳細（hover / 展開）に表示 |

## Agent 実行設定 UX

- **5 mode**: S9bは`Ask / Edit / Plan / Auto / Bypass`の単一selectorとし、旧`permission mode + Plan toggle`を廃止する。schema上の存在とruntime availabilityを区別し、availability source/checked at/unavailable reasonをRust capabilityから表示する。各mode controlのenabled/reason、反映時点、Bypass challenge要否は`AgentSessionConfigurationProjection.available_actions`だけから描画し、別modeへsilent fallbackしない。
- **危険性**: `Bypass`は影響範囲を説明し、Rustが返すexecution固有guard（Session revision、launch reservation＋draft hash、queue execution＋snapshot hash、`WorkflowExecution.id / NodeExecution.id / NodeDefinition.name / NodeExecution.attempt`＋resolution＋resolved hash、またはscope/action/targetまで含むreconciliation attempt）、target、期限、nonceへ束縛したone-time challengeに対する明示確認を要求する。通常承認を最大限迂回してもexplicit rules、MCP user interaction、provider circuit breaker等の`residual_protections`が残ることを列挙し、「全保護無効」とは表示しない。waiting projectionまたは`get_bypass_challenge`からIssued/Consumed/Expired/Cancelledをreload後も復元し、Issued時だけnonceを確認操作へ使う。確認中に設定やattemptが変わったらchallengeを失効表示して再取得する。provider側launch opt-inが必要ならrestart-requiredを表示する。template保存は権限付与でなく、S9a/S9cもexecution時に新しいchallengeを通す。managed policyによる禁止はUIで解除できない。
- **Auto**: 「無制限」「Releashが承認」とは表現せず、provider側classifier/reviewerがeligibleな要求を審査し、sandboxやmanaged policyは広がらないことを示す。typed ModeEffectから、Claudeのclassifier delegation＋keep-working/質問削減nudgeと、Codexのreviewer swapのみという差を表示する。approved/deniedだけをAuto解決として履歴表示し、inProgressはactivity、timedOut/aborted/manual fallbackは未解決・fallback状態として表示する。
- **工数（推論レベル）**: model選択後にcapabilityからoption、説明、default、schema/runtime availability、source/context/checked at、反映時点を描画する。selectedのProviderDefault/ExplicitとeffectiveのKnown(value/source)/Unknown(selected/expected/reason)を区別し、providerの広告順を維持する。model / effort controlのenabled/reasonは`AgentSessionConfigurationProjection.available_actions`に従う。model変更patchではtarget modelとeffortを一緒にpreviewする。Claudeのorganization limit等を含むauthoritative validation/readbackができない明示値は、tableのpreviewとdisabled理由を表示し、effective確定とは見せない。
- **使用実績との分離**: S9b の工数は model の応答・推論強度の signal、S7 は token / context / cost の使用実績である。工数は厳密な token 上限ではなく、時間・turn 数・token / cost / time budget の入力欄を追加しない。
- **Goal**: objective/statusだけでなくpending transition/sync state/latest evidence/provider snapshotをS9bに置く。controlはRustが実行contextで評価した`available_actions`だけを描画し、schema/runtime availability、source/context/checked at、strategy/application scope/effectsを操作前に示す。Claude set/editとclear後の再setが伴うStartsTurn、progress reset、identity replacementを確認文に含め、`--resume / --continue`によるSession復元をGoal Resume/turn開始と表示しない。Completed/Failed/Blockedはevidence付きoutcomeとし、根拠なしのcomplete/fail controlは置かない。historyはpaged queryでkind/result/time/before/after/evidenceを展開し、turnのgoal id/revisionから当時のobjectiveをlookupする。atomic batch失敗時はprovider turn/interrupt状態もreconciliation詳細に表示する。
- **scope 分離**: S9aはlaunch preflight/draft reservation＋start後のdurable launch attempt、S9bはcanonical configuration/Goal eventからbackend queryが構築した実行Session/turn-start read model、S9cはworkflow definition templateである。見た目を再利用してもstateとcommandを共有せず、S9b projectionをmutation authorityにしない。S9aはusecase-owned watch serviceの`open_watch`が同じcommit境界で決めたcommon-watermark snapshot/replay＋subscription handleを使う`after_seq` watchで小さなfull projectionを受ける。bootstrapはhandleのfence、live更新は各notice commitへ厳密にpinしたfenceの`read_at`だけからserviceがtyped `AgentLaunchChanged`を構築し、frontendはstorage noticeやleaseを扱わない。lag/lease失効/`ProjectionBehind`/gapではserviceが`close_watch`して部分結果を捨てsnapshotから再openし、disconnect時も`close_watch`する。launch成否不明時は最後に完了したstage、provisional provider resource、local Session、initial Goal handoff/reconciliation、観測値、部分protocol identityとRustが返すcleanup/readback/reuse/recreate操作を表示する。providerが安全なlookup/idempotent createを持たない場合はRecreateを表示しない。initial Goal待ち/明示rejectを区別し、reject時はRetry Goal / Goalなしで続行 / Session取消のRust許可済みactionだけを出す。Session確立前のProtocolIncompatibleもS9aに復元表示する。S9cはrequired/optional overrideとfield provenanceを表示し、実行Sessionのack済み結果と区別する。
- **確定表示**: frontendはdraft/requested/selected/effectiveと、Goal current/pending/syncを分ける。pending中は要求値と旧effective/currentを併記し、next-turn/restart activation前にeffectiveとして見せない。canonical commit失敗や結果不明は旧値表示へ戻さず`ReconciliationRequired`とRustが返す`reconciliation_id`・起点request/observation（存在する場合だけ）・解決操作を表示し、sendをdisableする。SessionMeta cacheだけの失敗はPersistFailure＋再投影として区別する。

## permission UX

- **正本**: pending permission は backend state（`get_session` の `pending_permission_request`、#1379）と transcript の permission part の 2 経路で届くが、描画の正本は read model。二重表示はしない（現行方針を維持）。
- **回答確定**: click後は、backendがexact validated response payload（updated input / answers / deny message）をowner-only private blobへ保存し、redacted intent＋private ref＋`PermissionDelivery` obligationを確定できた場合だけdurable `Responding`として操作不能にする。provider ack / authoritative observation後だけResolvedへ進める。timeout/restartで実効性不明ならpermission reconciliationを表示し、idempotency / readback根拠なしに同じ回答を再送させない。
- **失効**: `Cancelled` への遷移（CLI 取り下げ CL-1 / interrupt / turn 終端）を受けたら、dialog は即座に操作不能にし「取り下げられました」チップへ差し替える（FE-1: 押せるのに効かないダイアログを残さない）。
- **実効性**: `effective=false` の解決は「回答は届きませんでした（ツールは実行されていません）」と表示し、Allowed/Denied と視覚的に区別する（P6）。
- **整形表示**: `ApprovalDisplay` により command / diff / 対象ファイルを整形表示する。生 JSON の直接表示は fallback のみ（SD-6）。tool 名は transcript と dialog で一致させる。
- **Question**: question ごとに描画し、`is_secret` はマスク入力、`is_other_allowed` は自由記述欄、multi_select は複数選択 UI（CX-1 の語彙前提）。secret plaintextとprivate payload refはUI / history / logへ出さず、reload後も「回答済み」のredacted markerだけを表示する。backendは未完了responseのexact payloadをeffect Observed / terminalまでowner-only private blobで保持するが、暗号化済みとは表示しない。blobが安全に利用できずproviderが同requestをPendingと確認した場合だけ、新responseとして再入力を求める。
- **解決済みチップ**: decision（Allowed / Denied / Cancelled）に加え、`decision_reason` / `description`（ルール名・CLI の説明文）を表示する（FE-7）。

## エラー・バナーのスコープ規則

- バナー state は session_id をキーに保持し、他 session のイベントで消える・混ざることを禁止する（FE-5）。
- turn に紐づくエラーは S1 の part（durable）が正本。バナーは「操作の失敗（送信・切替等）」という session スコープの一時通知に限定する。
- operation feedbackのfeedback / attempt identity、revision、operation/failure kind、安全なlabel/detail、linked successによるclear、resolved / dismissed entryとowner indexのbounded cleanupはRust command/usecaseが所有する。frontendは`${error}`、`includes`等でraw errorを分類・整形せず、未解決failureだけの1 page最大32件typed snapshotをcursorでmirrorし、feedback ID / expected revision付き明示dismissだけを行う。Session close / archiveだけで未解決entryを消さない。
- process全体512件の未解決identityをsize/capacity都合で無言drop / coalesceしたり、別sessionのactive entryをevictしたりしない。Loadを含むfeedbackを返し得るdomain read / mutationは開始前にslotを予約し、成功時に解放する。予約できないoperationは対象へ作用前に`FeedbackCapacityExceeded`として直接表示し、mutation effectは0件である。一方、get / expected-revision dismiss / resolution retryは新entryを作らないexempt control planeとしてcapacity飽和時にも表示・実行する。retryはfeedback ID / expected revision / Rust-owned action IDを使い、再失敗は同じentryを更新する。RustがUTF-8安全なlabel最大160 bytes / detail最大2048 bytes、truncated marker、original bytes、correlation id/digestとavailable actionsを返す。
- application shutdown以外のappスコープ通知（更新通知等）は本書の対象外。

## close / quit UX

- chat tab / panel、workflow node tab、workspace、windowのview closeはviewだけを即時に閉じる。Session terminal、flush、permission / queue / runtime / workflow進捗の変更やshutdown progressを合成しない。
- normal Session close / open archiveはbackend projectionを表示する。activeはSessionClosed terminalを履歴に残し、Idleはsynthetic terminalを表示しない。どちらも既存queueを保持してPausedを表示する。runtimeが無ければ同じclosureでcompact SessionClose successとなるためpending / 「停止中」を表示しない。runtimeがあるclosure確定後にclose結果不明となった場合だけ、Closed / ArchivedをOpenへ戻さずqueue pauseと`SessionClose` reconciliation、Rust-owned actionsをS6 / S8へ表示する。
- closed Session archiveはArchived projectionだけを反映し、terminal / queue / provider shutdownの表示を追加しない。
- backend switchはIdleかつpending permission / recovery / obligationなしの場合だけenableにする。D1 operation中は既存queueを保持してPaused、旧snapshot不整合itemをNeedsResolutionとして表示し、old effective backendを表示し続ける。old runtime closeのObserved後だけnew effectiveへ切り替える。結果不明ではold effective＋queue pause＋ReconciliationRequiredを表示し、new backendを起動済みと見せない。
- active / Idle normal close、active / Idle open archive、closed archive、Idle backend switchはrequestから10秒以内にbackendの完了または同じoperation identityの結果未確認表示へ必ず移る。runtime close未応答ではClosed / Archivedとpaused queueを保ち、activeだけにSessionClosed terminalを表示する。closed archiveはClosedのままqueueを変えず、backend switchはold effective backendを保つ。10秒後のlate resultで別の「停止中」、terminal、reopen、new backend開始を追加せず、frontend timeoutから独自retryを生成しない。
- graceful quitはS10でquit開始時にopenなactive / Idle Sessionと進行中Workflowだけをeffect targetとして全target Preparing、global Activated、per-target stopping / reconciliation、13秒cutoff、15秒exitをbackend projectionから表示する。current surfaceは`get_application_shutdown / GetApplicationShutdown`のclosed `CurrentApplicationShutdownResult`を使い、exact `Current(Option<ApplicationShutdownProjection>) / OutcomeUnknown { failure }`を区別する。hash-validなcomplete rootがexactly oneでplan identityを一意にanchorできる場合だけsemantic pointer mismatchを`ShutdownAuthorityMismatch`付きReconciliationRequiredとし、storage / decode / envelope・self-hash / pointer-to-root hash failure、required record欠損、state composite・activation lineage integrity failure、identity unanchorable / ambiguousは別の`GetApplicationShutdownApplicationError::Internal { correlation_id }`として扱って通常起動、`OutcomeUnknown`、`None`へ隠さない。quit operation queryはOptionなしの`CurrentApplicationQuitOperationResult`と`ApplicationQuitProjection::Shutdown | Bootstrap`を使う。各Session targetは`min(executor開始 + 10s, cutoff)`までであり、遅いactivation後に新しい10秒を表示しない。closed / archived Sessionとdurable `OrphanRuntime` recovery obligationは専用partitionのpending recovery snapshotとしてexit / restart監督を示すが、新規shutdown effectやtarget countへ混ぜない。frontendはquit時にpending一覧を収集・複製せず、current inventoryとplanが固定したhistorical snapshotを別queryから描画する。same-boot current flightはsame resultへjoinし、未解決shutdownまたはscope fenceが残るnew quitは`PreviousShutdownReconciliationRequired`、retiring detail競合は`PreviousShutdownCompactionPending`としてAccepted前に表示する。Activatedだけでeffect開始済みと見せず、未予約Prepared targetもdurable identityを持つrecovery pendingとして表示する。`RetryQuit`はsame-boot pre-activation effect-0 Failed、durable fence、admission Open、store Healthyを同じsnapshotで証明できる場合だけ表示し、activation OutcomeUnknownまたはActivated後はadmission reopenを提示せず15秒以内のexitを示す。

## frontend 実装規約（ST-8 の解消方針）

1. reducer は「read model 断片の適用」に限定する。順序・欠落・合成の判断（seq 検証、snapshot との整合）は backend が read model / delta の契約として保証し、frontend は契約違反を検出したら snapshot 再取得する（FE-3）。
2. 表示ロジックは「part → component」の写像に限定し、状態機械（turn phase の解釈・permission の有効性判定・queue の遷移判断）を frontend に持たない。
3. 描画の正本は get_session で復元される read model / runtime state（語彙文書 §11。#1379 の pending permission 復元を含む）とする。transient event の蓄積から frontend が独自に再構成した状態を正本にせず、初期化経路は get_session の完全復元に一本化する。
4. Agent設定は`base_selected_revision`を持つtyped `ConfigurationPatch`の1variantでbackend usecaseへ要求し、selected/effective/pending/sync stateのmirrorだけを保持する。Goal commandは独立`base_goal_revision`を使う。turn送信payloadのfrontend値からcurrent stateを再構成しない。
5. 5 modeのprovider写像、runtime availability、Goal strategy/effects/transition、reasoning effortのselected/effective/互換判定、reconciliation、workflow override解決、protocol compatibilityをfrontendに置かない。Rust adapter/usecase/queryの結果を表示する。
6. mode/model/reasoning effortの選択肢とconfiguration action enabled判定は`AgentSessionConfigurationProjection.available_actions`、Goal action enabled判定は`SessionGoalProjection.available_actions`駆動とし、UIにprovider/model固有値やlifecycle表をhard-codeしない。Bypass challenge、Goal emulation effects、Unsupported/ProtocolIncompatible reasonもbackend応答をそのまま表現する。

## トレーサビリティ（本書が解消する問題）

| 問題 ID | 節 |
|---|---|
| FE-1 | permission UX（失効） |
| FE-2 | turn 終端の表示（live 即時） |
| FE-3 | P1 / frontend 実装規約 1 |
| FE-4 | S7 usage indicator |
| FE-5 | エラー・バナーのスコープ規則 |
| FE-6 | マトリクス Thinking / TaskStatus |
| FE-7 | permission UX（解決済みチップ） |
| SD-6（表示面） | permission UX（整形表示） |
| RG-4, RG-8（表示面） | マトリクス ToolCall |
| CX-3, RG-1（表示面） | マトリクス Thinking |
| CX-5, RG-2, RG-5（表示面） | マトリクス TodoListSnapshot |
| CL-3, RG-3（表示面） | turn 終端の表示 |
| CX-7, RG-6, CL-5（表示面） | Notice 振り分け |
| CX-8（表示面） | マトリクス Error |
| CX-11（表示面） | マトリクス ToolCall（WebSearch） |
| RT-6（表示面） | マトリクス session 状態 |
| OB-3, OB-5, OB-6（表示面） | マトリクス queue |
| RG-9（表示面） | S7 / turn 終端 stats |
| ST-8 | frontend 実装規約 |
| #1445, #1446 | S9a/S9b/S9c / frontend 実装規約 4（Rust-owned projection / revision / reconciliation） |
| #1447 | Agent 実行設定 UX（5 mode） |
| #1448 | S9a/S9b/S9c ReasoningEffort / S7 との分離 |
| #1449 | S9a/S9b/S9c AgentGoal |
| #1450 | Agent 実行設定 UX（workflow override / restart 継承） |
| #1451 | S9a/S9b/S9c 全体 / capability-driven UI |
| #1499 | P1 / P4 / P5、SessionOperationFeedback、PendingTerminal / Recovery表示、close / quit UI結果 |

## 設計判断

- **P-D1**: usage indicator は入力エリア上部（`MessageInput` 上縁）に常設（compact: context 使用率バー＋残量、クリックで token 内訳 / cost）。「あとどれくらい送れるか」を送信操作の直前で判断できる。#1150 で削除された旧表示の復活ではなく、「常時は要約のみ・詳細はオンデマンド」に再設計する。
- **P-D2（2026-07-19改訂）**: durable Noticeはtranscriptを正本とし、バナーは同一Notice idへの参照表示とする。`SessionOperationFeedback`はstate transition前のtyped command feedbackとして別型・別保存規則にし、durable Noticeへ偽装しない。clearはfeedback identity一致に限定し、stale successで新しいfailureを消さない。どちらもRustがkind/operation/safe text/clear/capacityを所有する。
- **P-D3**: 取り消した queue メッセージは transcript に「取り消し」マークで残す（lifecycle L-D4 と対応。非表示にしない）。
- **P-D4**: mode / Goal / 工数のvisual componentは再利用するが、S9a launch draft/attempt、S9b configuration/Goal projection、S9c required/optional workflow templateを別surface/state/commandとする。
- **P-D5**: mode は排他的 5 値 selector とし、Plan toggle は廃止する。Auto / Bypass の意味と危険性は provider capability と共に表示する。
- **P-D6**: 「工数（推論レベル）」を UI 名とし、S7 の TokenUsage / cost と視覚・状態・説明を分離する。
- **P-D7**: GoalはS9bの独立`SessionGoalProjection`としてcurrent/pending/sync/evidenceを表示し、Rust評価済みavailable actionsからset/edit/pause/resume/clearを提供する。completion/failureはevidence付きoutcome、provider strategy/effectsは操作前表示とする。
- **P-D8**: automatic Phase 0 bootstrap中はlegacy projectionをread-only表示し、application bannerへbackend-owned `InspectingSource / Importing / Verifying / Activating / Failed`、count、Failedだけのsafe failureを出す。pointer切替後のvalidationとnormal admission openまでActivatingを維持し、projectionがSomeの間はnormal mutation / irreversible effectをdisabled reason付きで止めるが、quitはnormal planを作らないbootstrap-safe 13秒settle・15秒one-shot exitへ渡す。None確定後はPhase0 authorityだけを表示してlegacyとのmerge / fallbackを行わない。quarantine raw bytes / blob refは公開しない。

## 確定事項（2026-07-07、2026-07-15 レビューで確定）

1. **P-D1**: usage indicator は**入力エリア上部**（パネルヘッダは不採用）。常時表示は最小要約（context 使用率バー＋残量）。
2. **Notice 振り分け**: 本書の振り分け表を初期値として確定。ModelRerouted / RateLimit の強調度は実装後の使用感で調整する（調整はこの表の更新として行う）。
3. **P-D3**: 取り消し queue メッセージは**マーク表示**（lifecycle L-D4 と同一決定）。
4. **Thinking**: **streaming 中は自動展開・完了で自動折畳**を既定とする。
5. **P-D4 / P-D5**: S9a/S9b/S9c の state scope を分け、mode は `Ask / Edit / Plan / Auto / Bypass` の排他的 selector とする。
6. **P-D6**: 工数は model の応答・推論強度として表示し、token / cost / budget の表示・入力とは分離する。
7. **P-D7**: GoalはSessionごとにcurrent最大1件の独立projectionとしてS9bから操作し、pending/reconciliation、Native/Emulated/Unsupported、strategy/scope/effectsを明示する。
8. **確定タイミング**: UIはconfigurationのrequested/selected/effectiveとGoalのcurrent/pending/syncを区別し、provider ack/canonical commit/activation前にeffectiveとして表示しない。reconciliation中は送信を止める。
9. **Bypass / Auto**: BypassはRustのexecution-scoped one-time challengeとprovider launch gateを必須とし、Autoはprovider reviewerの全status/fallbackとsandbox非拡張を表示する。どちらもworkflow checkpointを越えない。
10. **protocol compatibility**: compiled schemaと実行binary/flags/capabilitiesの不一致はS6のProtocolIncompatibleとして表示し、新規sendを止める。
11. **P-D8**: 初回upgradeのbootstrapは既存履歴をread-only表示しながら同一backend projectionのphase / count / safe failureを示し、pointer切替後のvalidationを含めprojectionがSomeの間はsend / permission / close / workflow / recoveryを無効にする。quitだけはnormal shutdown plan / target /明示commandを作らないbootstrap-safe bounded exitへ渡す。scope quarantineは対象scopeだけへ表示し、private raw bytesは出さない。
