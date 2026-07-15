# Agent セッションライフサイクルの理想形（不変条件）

作成日: 2026-07-07
更新日: 2026-07-15（Agent 実行設定 lifecycle を追加）

milestone 84「Agentチャット安定化」のドキュメント群:

- [agent-chat-instability-audit.md](agent-chat-instability-audit.md) — 問題点インベントリ（全 66 件、要求リスト）
- [agent-chat-ideal-vocabulary.md](agent-chat-ideal-vocabulary.md) — 正規化語彙・データ構造の理想形
- **agent-chat-ideal-lifecycle.md（本書）** — ライフサイクルの理想形（不変条件）
- [agent-chat-ideal-presentation.md](agent-chat-ideal-presentation.md) — UI 表示の理想形

本書は「セッション・turn・permission・queue が、どの経路を通っても何を保証するか」を不変条件（invariant）として定義する。監査で確定した lossy-lifecycle / divergent 問題群の解消先。語彙は vocabulary 文書の型を前提とする。

## 設計原則

- **L-P1 (durable-first)**: UI が表示する全状態は durable event → read model から復元可能でなければならない。transient event（Tauri emit）は同じ結果を早く見せるための加速手段であり、正しさに関与しない。
- **L-P2 (終端の明示)**: turn・permission・tool call・queue item は、どの経路（正常・中断・切断・クラッシュ）でも必ず終端状態に到達し、その終端が durable に記録される。「進行中のまま忘れられる」状態を作らない。
- **L-P3 (失敗の着地)**: 内部の失敗（永続化失敗・emit 失敗・backend 応答エラー）は握りつぶさず、`Notice` / `Error` / 構造化ログのいずれかに必ず着地させる。
- **L-P4 (backend 差の吸収)**: 停止・生存確認・頑健性の保証水準は backend に依らず同一とし、差はランタイム層が吸収する。
- **L-P5 (configuration authority)**: provider/model / Agent mode / Reasoning effort の正本は Rust の `AgentSessionConfigurationProjection`、Goal の正本は独立した Rust-owned `SessionGoalProjection` とする。frontend の入力値、provider の未確認値、各 turn の送信値を正本にしない。
- **L-P6 (atomic event visibility)**: 「同じlocal atomic batch」はvocabulary §9.5の`LocalEventTransactionStore.commit_batch`を指す。launch / Session / Goal / workflow / queueの全participant eventはcommit前に不可視、commit後に一括可視とし、独立logへの逐次appendをatomicと呼ばない。query/watchも同じglobal commit barrierを読む。

## 不変条件

### I1: turn 終端保証

あらゆる終了経路 — 正常完了・interrupt・Fatal・session close・backend 切替・アプリ終了 — で、`TurnResult` の durable 記録と streaming parts の Final 化（`FinalPartsRecorded`）が実行される。

- ギャップ: RT-1（close/切替/終了時に finalize も flush もされない）
- 要点: `close_session` / backend 切替 / アプリ終了 hook は「flush → `TurnResult::Interrupted { reason: SessionClosed }` で finalize → runtime close」の順を必ず踏む。backend の終了イベントを捨てない。

### I2: クラッシュ回収

アプリ起動時（およびセッション初回ロード時）に dangling turn（`TurnStarted` があり終端 event が無い）を検出し、`Interrupted { reason: Crash }` で finalize する。permissionは状態で分け、未送信の`Pending`だけを`Cancelled(effective=false)`へ畳む。write-ahead済み`Responding`/`Resolving`はresponse/resolution intentを保持し、provider cancel/tool-start/decision/readbackを相関して確定不能ならpermission reconciliationへ送る。`ReconciliationRequired`をCrash理由で上書きしない。tool call は `Interrupted` に畳む。

- ギャップ: RT-2（回収経路が無く、スピナー・確認待ちが永久残留）
- 要点: 回収は read model 投影時に lazy に行ってもよいが、結果は durable に書き戻す（reload の度に再判定しない）。

### I3: streaming flush 保証

streaming 中の本文・tool 出力は一定間隔（現行 1s の定期 flush を保証として明文化）で durable 化され、損失窓は最大 flush 間隔に限定される。turn 終端では必ず Final 化する。

- ギャップ: RT-2（クラッシュ時に直前約 1 秒が消える — これは許容損失として明文化）、FE-3（hydration が flush 済みデータを取りこぼす — presentation 側 P1 で解消）
- 要点: 「1 秒」は保証値として定数化し、テストで検証する。

### I4: queue の永続と一貫性

pending queue は durable であり、アプリ再起動・session close・backend 切替を跨いで生存する。queue item の終端は「実行開始 / ユーザー取り消し / 起動失敗」のいずれかで、全て記録・可視化される。

- ギャップ: OB-3 / RT-3（メモリのみで無言消滅）、OB-4（取り消しても human message が復活）、OB-6 / RT-7（起動失敗で無言停止）
- 要点:
  - queue eventをSession event logの正本とし、message/queue storeは再構築可能なprojectionにする。human message記録と`QueueItemEnqueued { item_id, message_id, input_ref, snapshot_hash }`、取消時のmessage cancelled markerと`QueueItemCancelled`は、それぞれ同じlocal atomic batchでappendする。片方だけ永続化される窓を作らない。
  - queue item は enqueue 時点の effective configuration revision と resolved mode / reasoning effort の小さな snapshot、`goal_id + goal_revision` を保持する。後続の Session 設定変更で既存 item を暗黙更新せず、変更する場合は revision conflict を検査する明示的 rebase command を使う。
  - drain 時は provider / model capability と Goal の active 性を最新contextで再検証する。Goal が completed / cleared / replaced、またはprovider/model/mode/effort/effects/residual protections/managed policyのsemantic結果がenqueue snapshot hashと変わった場合は、古い item を暗黙更新せず`QueueItemResolutionRequired`をappendして`NeedsResolution`とする（回復不能時だけ`Failed`）。明示`QueueItemRebased`だけがsnapshotを更新する。`checked_at/evaluated_at`等の観測時刻はcanonical semantic hashから除外し、意味が同一ならTurnStartedに最新revalidation evidenceだけを添える。
  - rebaseは`item_id + expected item revision + expected snapshot hash + resolution_id`をCASし、new snapshot/hashを持つ`QueueItemRebased`をappendして初めて`Queued`へ戻す。projectionだけを書き換えない。Stop受理時は`TurnInterruptRequested + QueuePaused`をprovider I/O前の同じlocal atomic batchでappendし、終端前にcrashしても自動drainを止める（既にpausedならidempotent）。resumeはユーザー操作によるCAS付き`QueueResumed`だけで行う。
  - adapter は per-turn override があれば snapshot のcanonical semantic値を turn start payload だけへ適用し、Session の selected / effective configuration は変更しない。ただしsnapshot modeがBypassのqueue itemはrevision差の有無にかかわらず毎executionでfresh challengeを要求し、最新再評価で得たeffects/residual protectionsをchallengeへ含める。`prepare_queue_execution`がexecution id/snapshot hashをreserveし、`QueueExecutionPrepared + BypassChallengeIssued`を同じlocal atomic batchでappendして`AwaitingBypassConfirmation`へ移す（non-Bypassはprepare不要）。確認後だけconsume＋start intentへ進み、失効/変更時は新reservationを要求する。provider が session-scoped 更新しか持たず item snapshot が current effective と異なる場合は、一時適用・復帰を行わず `NeedsResolution { reason, resolution_id, actions: [Rebase, Cancel] }` にして明示 rebase を要求する。
  - 取り消し時は永続化済み human message を「cancelled」としてマークし（削除しない。マークは message read model へ durable に記録し reload 後も保たれる）、復元コンテキストからも除外する（OB-4）。
  - non-Bypass drain、またはBypass confirmation後のstartは`QueueExecutionRequested { item_id, execution_id, snapshot_hash }`をappendして`Starting`にする。Bypassは`BypassChallengeConsumed`を同じpre-I/O batchに含める。独立configuration activation済みproviderではさらに`QueueItemStarted + TurnStarted { queue_item_id, queue_execution_id }`を同じbatchでappendし、その後だけ送る。turn/start payloadでper-turn overrideを適用するproviderは`TurnStartRequested`を同じpre-I/O batchへ含め、ack後に`QueueItemStarted + TurnStarted`だけをatomic commitする。queue snapshotで`SessionConfigurationActivated`をappendせずSession effectiveを変更しない。restart時、TurnStartRequestedが無いStartingは`Failed(InterruptedBeforeStart)`へ決定的に回収し、未完TurnStartRequestedがある場合はprovider送信結果不明としてTurnStartReconciliationへ移す。どちらも暗黙再送しない。
  - queue item status は `Queued / AwaitingBypassConfirmation / Starting / Started / Failed / Cancelled / NeedsResolution` を型で持つ。起動失敗した item は `Failed` として残し、ユーザー操作（再試行・取り消し）を待つ。無言のまま次の送信で暗黙復活させない（RT-7 の解消）。
  - Failedの再試行は`item_id + expected item revision`をCASし、新revisionを持つ`QueueItemRequeued`をappendして同じimmutable input/snapshotを`Queued`へ戻す。以前のexecution/challengeを再利用しない。Bypassなら必ず新しい`prepare_queue_execution`とchallengeを通す。semantic snapshotを変更する場合はretryでなく明示rebaseを使う。

### I5: interrupt 保証

Stop 操作は常に受理される。backend への interrupt 送出が不可能・無応答でも、猶予（既存 Claude の synthetic abort 10s を両 backend の共通保証にする）の後にランタイムが turn を強制終端する。

- ギャップ: OB-1 / SD-2（Codex は turn_id 未取得窓で無言 no-op、フォールバック無し。frontend も再押下を握りつぶす）
- 要点:
  - turn_id 未取得窓の Stop は「turn_id 取得後に interrupt を送る」予約として保持する。
  - `StartingTurn`（TurnStartRequested後/ack前）のStopも常に受理し、起点や現在のitem有無に関係なく`TurnInterruptRequested + QueuePaused`（既にpausedならidempotent）をatomic appendして`TurnStartState::InterruptRequested`へ移す。late ackを通常のTurnStarted commitへ流さず、取得したprovider turnをinterruptしてInterruptedでfinalizeするか、ack/interrupt結果不明なら同じrequest idのTurnStart reconciliationへ畳む。
  - interrupt 後の queue は自動 drain しない（OB-5）。Stop受理を`TurnInterruptRequested + QueuePaused`のpre-I/O atomic batchで確定してからbackend interruptを送り、再開はユーザー操作によるCAS付き`QueueResumed`だけにする（presentation 文書 §queue）。

### I6: ユーザー入力の無損失

送信操作は成功（turn 開始 or queue 追加）以外の結果を持たない。steer 非対応・stall 中・起動失敗のいずれでも、入力テキスト・添付画像は失われない。

- ギャップ: OB-2（stalled turn 中の送信が即エラーになり入力ごと消える）
- 要点: `steer` 未対応 backend への実行中送信は queue へ積む（現行仕様）。stalled 判定中も同じ経路に載せ、エラーにしない。送信 API が失敗した場合も入力欄の内容は保持する（presentation 側 P5）。

### I7: permission の有効性

permission request は「backend が回答を待っている間」だけ Pending である。CLI 側の取り下げ（Claude `control_cancel_request`）・turn 中断・turn 終端で即座に `Cancelled` に遷移し、durable / UI の両方へ反映される。解決の記録は実効性（`effective`）を区別する。

- ギャップ: CL-1（cancel 未処理で効かないダイアログが残り、誤った Allowed/Denied が永続する）、FE-1（interrupt 中に操作可能なダイアログが残る — 表示は presentation 側）
- 要点: `control_cancel_request` を処理して `PermissionRequested` を Cancelled へ更新する。失効後に届いた回答は `effective=false` で記録し、backend へは送らない。

### I8: 状態変更の ack 駆動と失敗の可視化

backend に対する状態変更（set_model / set_permission_mode / interrupt / permission 応答）は、backend の応答（control_response / JSON-RPC response）を検査して初めて確定する。楽観更新で UI と backend 実態を乖離させない。永続化・emit の失敗は握りつぶさず着地させる。

- ギャップ: CL-2（control_response を無言破棄し楽観更新）、CX-6（JSON-RPC error response を warn ログのみで握りつぶし）、ST-4（`let _ =` による persist 失敗の無視）、RT-4（event log が自己修復せず恒久故障）、RT-8（append 失敗時に欠落 parts で上書き）
- 要点:
  - 応答エラー時は state を巻き戻し、`Notice(level=Error)` で通知する。
  - permission回答は`PermissionResponseRequested { response_id, request_id, decision, persisted_answers }`をprovider I/O前にappendして`Responding`へ移し、ack後だけ`PermissionResolved`にする。要求自体はPendingのままproviderが回答を明示rejectした場合は`PermissionResponseRejected { response_id, request_id, reason }`をappendして旧responseを終端し、requestを`Pending`へ戻す。secret plaintextはその場で破棄する。timeout/restartはrequest cancel/tool-start/readbackを相関し、確定不能ならpermission専用reconciliationとして同じresponse idを二重送信しない。
  - permission reconciliationも共通write-ahead解決sagaを使う。`ReconciliationResolutionRequested(Permission(...))`をappendして`Resolving`へ排他遷移し、ReadBackは新observation、AcceptObservedは同request/decisionの一意なaccepted/cancelled/tool-start証拠、Cancelはprovider cancel ackまたはturn終端を確認してから`PermissionResponseReconciled`で閉じる。ReenterSecretはproviderが同requestをPendingと確認した場合だけ旧responseをrejected terminalにして新response idを発行し、再入力plaintextは永続化しない。未完了resolution attemptはrestart時に同じidでreadbackし、二重回答・二重cancelをしない。
  - secret answerのplaintextはprovider送信用ephemeral memoryだけに置き、durable event/read modelには`Redacted { answered }`だけを保存する。secretを含む未完了responseはcrash後に自動retryせず、providerがまだpendingと確認できた場合だけ再入力を要求する。
  - `let _ =` を全廃し、persist 失敗は「リトライ → 失敗継続なら Notice(PersistFailure) ＋ 該当操作のエラー化」に統一する。
  - event log の破損（欠け `]` 等）は読み込み時に自己修復し、修復した事実を Notice に残す（RT-4）。
  - **例外**: `Notice(PersistFailure)` 自体の永続化も失敗し得るため、PersistFailure に限り transient（session バナー）での表示を許容する。durable-first（L-P1）の唯一の例外として presentation 文書 P1 にも明記する。

### I9: resume 回復の統一

backend session の resume 失敗（mismatch・thread 消失）は、両 backend とも同一の回復経路を通る: transientな`BackendSessionCleared`受信 → `BackendSessionRecoveryStarted { recovery_id, old_provider_session_generation, reason }`とresume metadata clearを同じlocal atomic commitで確定 → configuration/Goalを同じ`recovery_id`の`RecoveringBackendSession`へ投影してturn/queue/workflow resumeをblock → 新規 establish → selected configurationのreapply/readbackと`SessionConfigurationReactivated`（provider session generation更新、readback observation idをconsume）→Goalの有無・terminal/currentを含めたrestore/readback/reapply。最終`SessionGoalReactivated + BackendSessionRecoveryCompleted`を同じlocal atomic batchでappendして初めてSynced/公開し、Goal restore strategyが`StartsTurn`なら`TurnStarted`もそのbatchへ含めearly streamをbufferする。Goal readback pathはconsumed observation idを保存する。結果不明はGoal/turn reconciliationへ送り、NoCurrentGoal/terminal/no-changeでも必ずGoal reactivation eventをappendする。旧effectiveを新provider sessionの実効値として使わず、回復失敗はreconciliationへ送る。

- ギャップ: SD-1（Claude は無言自動復旧・Codex は恒久死）、OB-8（requeue で editor_context が脱落）
- 要点: requeue する turn input は editor_context を含め完全に保全する。

### I10: backend stdout の頑健性統一

両backendのstdout読み取りは共通の分類契約を持つ。provider/versionで明示識別できるdiagnostic log/noiseだけをskipしてカウントし、`Notice(Diagnostic)`として可視化する。未分類の非JSON・malformed frame・size上限超過はcontent/controlを判別できないため破棄継続せず、raw refと部分protocol identityを`ProtocolIncompatible`へ保存して新規turnをblockする。「セッションを殺さない」は既存read modelを閲覧・回復操作できるままにする意味であり、send継続を意味しない。

parse可能でcontent-planeと確定できた未知message/partだけはraw payloadを保全して`Notice(UnsupportedMessage)`へ着地させる。mode・Goal・effort・approval・sandbox・reviewer等のcontrol-plane未知値/variantは`ProtocolIncompatible`または対象aggregateのreconciliationへ移す。

- ギャップ: SD-3（Codex は非 JSON 行 1 行で即死・サイズ上限なし。Claude は 8MB 破棄が type 不明のまま）
- 要点: 読み取りラッパを infrastructure 共通部品にし、両 backend で使う。

### I11: 生存シグナルと stall 判定

「進行中」の判定は backend からの進行シグナル（parts delta / KeepAlive / status 変化）の有無で行い、シグナル要件は backend 別に定義する。長考（reasoning）中も何らかのシグナルが届くこと（Codex は reasoning delta 購読 — CX-3 の解消 — で充足）。stall 診断は「実際に進行が観測できない」場合のみ発火する。

- ギャップ: SD-4（Codex reasoning 中の無シグナルで stall 誤検知）、ST-9（permission 待ち以外の不可視停止に診断が無い、閾値 60s）
- 要点: stall 診断を全 phase（Streaming / WaitingPermission / Interrupting）に拡張し、`Notice(Diagnostic)` として durable に残す。

### I12: エラーの着地保証

backend プロセス死・turn 失敗・Fatal は、発生時点で durable な `Error` part または `Notice` として着地し、live UI にも即時反映される（表示規則は presentation P3）。workflow への turn 完了通知は構造化された失敗理由（`TurnError`）を運ぶ。

- ギャップ: RT-6（Idle 中 Fatal が痕跡ゼロ）、FE-2（crash/timeout が live で無言 — 表示側）、RT-5（workflow に exit 1 しか伝わらない）

### I13: 排他とロックの規約

session runtime lock の保持中に、別 session の lock・長時間 await（backend I/O）を行わない。lock の取得順序と保持範囲を規約として明文化し、prune はランタイムハンドル取得に依存しない方式にする。

- ギャップ: ST-5（二段ロックと prune skip）、ST-3（巨大 module 内で lock 範囲が追えない）

### I14: Agent 実行設定の ack・revision 保証

provider/model、mode、reasoning effort の変更は Rust usecase が所有する。Goal は I15 の独立 aggregate とし、configuration に Goal 参照を持たせない。外部 provider と local persistence の atomicity を仮定せず、`durable intent → adapter apply / ack → canonical event commit → activation` の順で処理する。

- `AgentSessionConfigurationProjection` は selected intent、effective provider state、1件の pending update、sync state を持つ。selected effort と effective / unknown effort を別型にし、provider/modelを同じ revisionへ含める。
- command は full snapshot でなく `ConfigurationPatch` の1 variantを送る。model変更だけは対象modelとmodel-bound effort selectionを同じsemantic patchに含め、`target_model.provider_id == session.provider_id`をRustで必須検証する。provider変更は通常patchではなく、turn finalize、Goal/queue handoff判断、protocol preflightと新launchを伴う別usecaseとする。Rustがbase selected snapshotからtarget snapshotを導出する。
- execution-affectingなuser更新は初期実装ではIdleのみ受け付け、Session共通control-operation leaseを取得する。`base_selected_revision`をCAS検証し、configuration/Goalのいずれかが非Synced、Goal transition pending、別update/lease中ならprovider I/O前にconflictを返す。Streaming / WaitingPermission中のmode変更や、configuration activation中のClaude Goal StartsTurnを許さない。
- provider I/O前に`ConfigurationUpdateRequested { update_id, base_selected_revision, target_revision, patch, applies_from }`をevent logへappendする。append成功がdurable intentのcommit pointで、idempotenceは`update_id`で判定する。
- liveのprovider ack、またはnext-turn/restart stagingのtyped adapter acceptance後の`SessionConfigurationSelected` event appendをselected configurationの唯一のdurable commit pointとする。staging acceptanceはprovider effectiveを意味しない。SessionMetaはeventから再構築可能なprojection/cacheであり、その更新失敗は`PersistFailure`と再投影で回復する。
- live activationは`SessionConfigurationActivated` append後だけeffective revisionを進める。独立configuration APIがある`AwaitingNextTurn`はprovider turn開始前にselected patchを適用し、activation ackとevent append後にeffective snapshotでturnを開始する。`AwaitingRestart`もrestart/readback後に同じ順序でactivateする。
- providerがmodel/mode/effortを`turn/start` payloadでしか受けない場合、`TurnStartRequested`は実行sourceを区別する。既存effective/queueは`ExistingEffective(ResolvedTurnConfiguration)`としてcanonical semantic snapshotとsource hashを固定し、pending selectedはprovider ack前の値をeffectiveと呼ばず`ActivateSelected { selected, originating_update_id, canonical_target_hash, prevalidated_context_hash }`として保存する。queue起点ではcurrent Session selectedへjoinし直さない。通常sendはhuman message記録、queue/Bypassはqueue execution intent/challenge consumeと同じlocal batchにする。early eventをbufferし、ack/readback後に初めてactual effective snapshotを`TurnStarted`へ固定する。`SessionConfigurationActivated`は`ActivateSelected`をsession-scopeで実際にactivateした場合だけappendし、queue per-turn overrideでは`QueueItemStarted + TurnStarted`だけをatomic commitしてSession effectiveを進めない。ack/commit結果不明はprovider turn/config observation付き`TurnStartReconciliation`へ送り、未完了intentはrestart後もblockする。
- TurnStart reconciliationのReuse/AcceptObserved成功はconfiguration activation、`TurnStartReconciled + TurnStarted`、queue起点なら`QueueItemStarted`（provider側が既に完了ならTurnResultも）を同じlocal atomic batchで確定する。Cancel/CleanUp成功は`TurnStartReconciled + QueueItemFailed/Cancelled + message marker`をatomic appendする。どのactionでもqueue itemを`Starting`のまま残さず、結果不明は同じresolution attemptでblockを継続する。
- pending updateはActivated / Rejected / Reconciledまで保持し、`Synced`以外では次の更新を受け付けない。`SessionConfigurationSelected`前のprovider rejectだけは`ConfigurationUpdateRejected`を記録してpendingを消し、旧selected/effectiveを維持する。selected commit後のNextTurn/Restart activation reject・timeoutではnew selected / old effectiveを維持してreconciliationへ入り、selectedを旧revisionへ戻さない。rollbackを選ぶ場合は旧effective相当を新しいselected revisionとしてcanonical appendする。
- timeout、部分成功、provider ack後のcanonical event append失敗、provider-originated競合は`ConfigurationReconciliationRequired`とし、新規turn / queue drain / workflow resumeをblockする。各reconciliationに新しい`reconciliation_id`を発行し、local request由来のときだけ`originating_update_id`、provider観測があるときだけ`observation_id`を関連付ける。provider-originated driftのために架空のupdateを作らない。`ProviderConfigurationStateObserved`、差分、allowed actionsをdurable化し、readback / idempotent reapply / rollback /明示acceptを`ConfigurationReconciled`で確定する。未完了request自体もrestart後のreconciliation根拠になる。
- `ProviderConfigurationStateObserved`のappend時点でsync stateを`ObservationPending { observation_id }`にしてblockする。同じobservation idをcanonical activation/no-change acceptance/reconciliation eventがconsumeするまでSyncedへ戻さず、restartは未consumed observationを必ず再評価する。
- configuration / Goal / launch / turn-start / permissionのreconciliation解決は、expected observation id/projection seqをCAS検証してresolution attempt idをreserveする。Bypassを結果にする場合はscope/reconciliation/attempt/observation/seq/action/target hashへ束縛したfresh challengeを発行する。`ReconciliationResolutionRequested { resolution_attempt_id, reconciliation_id, action, target_hash }`とchallenge consumeをprovider I/O前の同じbatchでappendし、`ResolvingReconciliation`へ排他遷移する。ack/readback後だけ`*Reconciled`をappendし、未完了intentはrestart時に同じattempt idでreadback/recoveryしてReapply/Recreate/cleanup/Claude Goal turn/permission responseを二重実行しない。
- initialize時にspawnしたbinaryの`BackendProtocolIdentity`とcompiled schema/flags/capabilitiesを照合する。不一致やcontrol-plane decode failureは`ProtocolIncompatible`としてfail-closedにする。initialize完了前は取得済みfieldだけの`ObservedProtocolIdentity`、expected hash、raw control refをlaunch attemptへ保存する。
- pre-session `ProtocolIncompatible`でもprovisional resourceがあり得るため、statusは`reconciliation_id`付き`LaunchReconciliation`を内包する。cleanup/cancel/reuseは共通`ReconciliationResolutionRequested` sagaを通し、protocol errorを理由にwrite-ahead回復を省略しない。
- New Agentのsubmit前はworkspace/provider/context keyed preflight queryが`Checking | Compatible(capabilities) | ProtocolIncompatible(partial identity)`を返す。`prepare_agent_launch`がdraft検証後にattempt id/canonical draft hashをreserveし、Bypassなら`AgentLaunchDraftPrepared + BypassChallengeIssued`を同じlocal atomic batchでappendする（non-BypassはPrepared単独）。`start_agent_launch`はreserved id/hash/preflight context、workspace trust、managed policy、provider identity/capability/gateを再検証し、Bypass consume＋`AgentLaunchAttemptStarted`をatomic appendしてからprovider I/Oする。draft変更/期限切れはreservationを失効させ再prepareする。
- 初回createはvalidated `AgentConfigurationDraft`と`canonical_draft_hash`からdurable `AgentLaunchAttempt`を発行する。`attempt_id`から安定生成したprovider create correlation keyと、provider対応時のidempotency keyをrequest前に保存し、provider session/threadをprovisionalに作成する。各`StageAdvanced`はprovider ref、local session id等のpayloadも保存する。途中失敗はcleanupし、成否不明ならlaunch reconciliationへ移す。readbackはcreate lookupのFound/NotFound/Ambiguous/Unsupported、match basis、lookup consistency/stable-since、provider ref/configurationをdurable化する。Reuseは一意Found、Recreateはauthoritative NotFoundまたは同じkeyで冪等createを保証できる場合だけ許可し、eventual NotFound単独では許可しない。reserved attempt別event streamと`get_agent_launch / watch_agent_launch(after_seq)`、単調`seq`を正本とし、watchはsnapshot/replayとsubscription登録を同じbarrierで行う。古いcursorはfull projectionを返し、get→subscribe raceとfield欠落を作らない。frontend draftは正本ではない。
- provider resource作成とinitial configurationのapply/readback後、`SessionCreated + SessionConfigurationSelected(revision=1) + SessionConfigurationActivated(revision=1) + LaunchStageAdvanced(LocalSessionCommitted)`をlaunch/session stream横断のlocal atomic batchでappendし、new Sessionのconfigurationをseedする。initial Goalなしは`AgentLaunchCompleted`も同じbatchへ含める。batch失敗ではSessionを公開せずlaunch reconciliationへ入り、SessionMetaは成功batchから後で再投影する。
- draftにinitial Goalがある場合もGoalはconfiguration/launch provider createへ混ぜない。local Session commit後、`LaunchStageAdvanced(InitialGoalTransitionRequested)`と`GoalTransitionRequested { originating_launch_attempt_id }`を同じlocal multi-stream atomic batchでappendしてからprovider I/Oする。canonical Goal event、Claudeの`TurnStarted`、`LaunchStageAdvanced(InitialGoalCommitted)`、`AgentLaunchCompleted`も同じatomic batchで確定する。それまでは`WaitingForInitialGoal`とする。結果不明時はattemptからGoal reconciliationを参照し、restart/reuseでも同じtransition idを再開する。明示rejectはGoal streamの`GoalTransitionRejected`とlaunch streamの`LaunchInitialGoalRejected`を同じatomic batchでappendし、attemptを`WaitingForInitialGoalResolution`へ再投影可能にする。
- reject解決はexpected transition/attempt seqをCASし、`InitialGoalResolutionRequested`をappendして`ResolvingInitialGoalFailure`へ移す。RetryGoalは`InitialGoalResolutionCompleted { action: RetryGoal, next_transition_id }`＋新transition idのlaunch/Goal intentをatomic appendして`WaitingForInitialGoal`へ移す。ContinueWithoutGoalはresolution completed＋launch completed、CancelSessionはcleanup intent後にresolution completed＋launch cancelled＋session closedをatomic appendする。cleanup結果不明は同resolution attemptのlaunch reconciliationへ送り、未完了resolutionをrestart時にreadbackして暗黙retryしない。
- `BypassConfirmationChallenge`はtarget、期限、nonceに加え、Sessionなら`session_id + selected_revision`、Launchなら`attempt_id + canonical_draft_hash`、Workflowなら`run_id + node_id + execution_attempt_id + resolution_id + resolved configuration hash`へ束縛する。Workflow Bypassは`WorkflowNodeBypassPrepared + BypassChallengeIssued`をatomic appendして`WorkflowWaitingBypassConfirmation`へ移し、reload後も同じ期限・guardを表示する。失効時は`BypassChallengeExpired`から`BypassConfirmationExpired`へ投影して新prepareを要求する。確認後はprovider I/O前にmanaged policyとguardを再検査し、`BypassChallengeConsumed`をSessionの`ConfigurationUpdateRequested(SetMode(Bypass))`、Launchの`AgentLaunchAttemptStarted`、Workflowのnode execution/launch intentと同じlocal atomic batchでappendする。provider I/O中にlockは保持せず、失敗後もchallengeはconsumedのままにする。同一intent id・同一guardのidempotent retryだけを許し、新しいintentには新しいchallengeを要求する。Claudeのdangerous launch opt-in等provider gateも別に必要で、gate無しSessionではrestart-requiredまたはdisabledとする。template保存だけではBypassを付与しない。
- `send_agent_message`はSessionのeffective snapshot、queue drainはI4のimmutable snapshot、workflow起動はdurable baseline/default/overrideからRustが解決したsnapshotを使う。frontendから毎turn設定を再送して正本を上書きしない。
- ただし全`TurnStarted`、queue drain、backend resume、workflow resume直前にprovider/model/mode/reasoning effortと必要なGoal continuation capabilityを、最新のdeployment/org override、workspace trust、managed policy、provider gate/residual protectionsを含むavailability context hashでRustが再評価する。effortのauthoritative validationができない、またはeffective Bypassのpolicy/gateが設定後に失効した場合は送信せずUnknown/NeedsResolution/reconciliationへ移し、silent clampや必要なprovider restartの隠蔽を禁止する。成功時のprovider permission/effects/residual protections/context hashはimmutable`EffectiveModeSnapshot`としてexecution configurationと`TurnStarted`へ固定する。
- legacy / unknown設定はvocabulary V-D10のmigrationを通す。復元不能ならscope/field/raw payload/resolution id/actionsを持つ`NeedsConfigurationResolution`としてblockし、`Edit`等へfallbackしない。

### I15: Goal lifecycle と回復保証

Goal は configuration とは独立した `goal_id / goal_revision / pending transition / sync state` を持つ Rust-owned aggregate とする。Session ごとのcurrent Goalは最大1件で、terminal Goalもclearまたは次のsetまではcurrentとして保持する。provider eventは`goal_id`またはopaque provider refで相関し、古いGoalの遅延通知をcurrent Goalへ適用しない。

| 現在 | 入力 / actor | 次 | 規則 |
|---|---|---|---|
| None / Completed / Failed | set（User） | Active | 新しい `goal_id` を発行。既存 Goal を再利用しない |
| Active / Paused | edit（User） | 同 status | objective と goal revision を更新。provider / evaluator へ再適用 |
| Active | pause（User / Provider） | Paused | native 非対応時は capability に明示した clear 等で emulation |
| Paused / Blocked | resume（User / System） | Active | capability 再検証後に再接続 |
| Active | blocked（Provider / System） | Blocked | reason と raw provider status を保持 |
| Active | completion（Provider / Evaluator / System） | Completed | evidence ref 必須。turn 停止だけから推測しない |
| Active / Paused / Blocked | failure（Provider / Evaluator / System） | Failed | failure reason を保持 |
| Active / Paused / Blocked | clear（User / System） | None | `GoalCleared` を記録。Completed を合成しない |
| Completed / Failed | clear（User / System） | None | terminal outcome の表示を明示的に閉じる |

- user起点のset/edit/pause/resume/clearはIdleかつconfiguration/GoalともSynced・pending無しの場合だけ受け付け、Session共通control-operation leaseを取得して`base_goal_revision`をCAS検証する。provider I/O前に`GoalTransitionRequested`をappendし、ack後の`GoalSet / GoalTransitioned / GoalCleared` appendをcanonical commit pointとする。rejectは旧currentを維持し、timeout/部分成功/ack後append失敗/provider競合はGoal専用reconciliationにする。各reconciliationに新しい`reconciliation_id`を発行し、local request由来のときだけ`originating_transition_id`、provider観測があるときだけ`observation_id`を関連付ける。provider-originated observationのために架空のtransitionを作らない。
- pending transitionは`GoalSet / GoalTransitioned / GoalCleared / GoalTransitionRejected / GoalReconciled`のいずれかまで保持する。成功終端はcanonical Goal eventだけで、別の`GoalTransitionApplied`を設けない。`SessionGoalProjection.available_actions`はstatus、capability、pending state、managed policyからRustが評価し、frontendは遷移表を再実装しない。
- `ProviderGoalStateObserved`のappend時点でGoal sync stateを`ObservationPending { observation_id }`にし、provider refとMatched/Unmatched/Ambiguous相関をsnapshotへ保存する。同じobservation idをcanonical/no-change/reconciliation eventがconsumeするまで新規turnをblockし、未consumed observationはrestart時に再評価する。
- capability はset/edit/pause/resume/clear等の操作ごとにstrategy、application scope、effectsを含む`Native / Emulated / Unsupported(reason)`を返し、schema supportとは別にworkspace trust/session/managed-policy contextでruntime availabilityを評価する。provider RPC、provider CLI command、明示的なReleash-managed evaluatorを分け、暗黙のprompt接頭辞で対応済みに見せない。
- Claude `/goal <objective>`は`ProviderCliCommand`であり、setまたはactive Goalのeditと同時にturnを開始する。Setは`StartsTurn`、Editは`StartsTurn / ReplacesProviderGoalIdentity / ResetsProviderProgress`を宣言する。acceptanceはpinしたCLI fixtureで証明した`system/command_lifecycle(completed, command_uuid)`とtyped Goal state (`goal_set`/`goal_status`またはactive Goal snapshot)の両方、かつ要求objective hash一致を必要とする。Goal intent→command後はcontent-plane deltaだけをbufferし、evidence observation＋`GoalSetまたはGoalTransitioned + TurnStarted`を一つのatomic batchでappendしてから公開する。commit前の`can_use_tool`/`request_user_dialog`等の応答必須control-planeはbufferせずfail-closed応答→interruptし、raw request/response/interrupt evidence付きGoal/turn reconciliationへ送る。shape/order/相関をfixtureで証明できないCLI versionやworkspace trust/hooks/managed policy不明ではStartsTurn actionをadvertiseしない。このprotocolはinitial Goal、RetryGoal、resume emulation、backend recovery中のGoal restoreにも共通適用する。
- Claude Codeの`--resume / --continue`によるSession復元はGoal ActionのResumeではない。復元されたGoal stateとaccounting baseline resetを観測してもturn開始を合成しない。pauseをclearでemulateした後のGoal Resumeを`/goal <objective>`再setで行う場合だけ、`StartsTurn / ReplacesProviderGoalIdentity / ResetsProviderProgress`を宣言する。Codex objective editもset RPCによるidentity置換・progress/accounting resetをeffectとして公開する。
- clear/re-setによるpause/resume emulationがprovider進捗reset、turn開始、provider Goal identity置換を伴う場合、そのeffectsを操作前に表示する。補償不能ならUnsupportedとする。
- Codex statusはactive/paused/complete/blocked/usageLimited/budgetLimitedを全域写像する。unknownはraw `ProviderGoalSnapshot`を保持してreconciliation、FailedはReleash/System側statusとする。
- close / crash / app restart / backend resume / workflow restart を跨いでも同じ Goal id、objective、status、goal revision を復元する。Claude の active Goal resume や Codex readback など provider 固有の回復を adapter が担う。
- Goal の automatic continuation は `AgentMode::Auto` と別概念である。どの mode でも permission と workflow human checkpoint を維持し、Plan / provider policy が continuation を制限する場合は理由付きで停止する。
- workflow の `task` は step の作業指示、Agent Goal は Session の継続的な completion condition / status であり、別フィールドとして保持する。片方から他方を暗黙生成しない。
- provider 固有の Goal budget / usage accounting は raw status の保持と停止理由の表示だけを行い、今回の Goal 設定にも ReasoningEffort にも取り込まない。
- clear/replace後もturn監査から当時のobjectiveを引けるよう、Goal canonical eventからpaged historyと`goal_id + revision` lookupを提供する。transition recordはkind/result/time、before/after snapshot、source/evidence、launch attempt相関を持ち、current projectionへ全履歴を保持しない。

### I16: ReasoningEffort capability と反映時点の保証

Reasoning effort の選択肢・説明・既定値・反映時点（live / next turn / session restart）は provider/model capability から取得し、provider が広告した順序のまま read model に含める。runtime capability APIがないproviderではprotocol identityにpinしたRust compatibility tableをsourceとして明示し、検証不能なmodel/valueはUnsupportedにする。

- selectedは`ProviderDefault / Explicit(value)`、effectiveは`Known { value, source } / Unknown { selected, expected?, reason }`として分ける。default、未取得、非対応、effective不明を`Option`一つに畳まず、tableの予想値をprovider確認済みeffectiveとして扱わない。
- 選択値が model に非対応なら更新をprovider I/O前に拒否し、利用可能な値と理由を返す。model変更patchはtarget modelとそのmodel向けeffort selectionを一体で検証し、別値へsilent fallbackしない。Claudeはprovider/model/deployment、organization上限、capability overrideまで含む実行contextでauthoritative validationまたはeffective readbackができなければ明示effortをUnsupportedとし、static tableや下位levelへのsilent clampに依存しない。
- next-turn は selected を確定して `AwaitingNextTurn`、restart 必須は `AwaitingRestart` とし、activation event まで旧 effective revision を使う。次turn開始前にactivationを完了し、失敗時は`TurnStarted`を記録しない。reconnect 後も pending / awaiting state を durable intent から復元する。
- Plan preset 等が effort を内包する場合、明示選択値を上書きせず、override 可否を capability として返す。両立不能なら mode / effort conflict を解決するまで turn を開始しない。
- Reasoning effort は provider / model の応答・推論強度を調整する behavioral signal で、実使用量や上限を保証しない。`TokenUsage`、cost、経過時間、turn 数、token / cost / time budget を停止条件や代替値として混ぜない。`TurnStarted`にはprovider/model/effective effort/unknown reason/protocol identityを保存する。

#### Workflow / launch configuration の解決

workflow definition はprovider-ack済みSession configurationを直接保存せず、revision付き`AgentConfigurationTemplate`を持つ。必須model/modeは`RequiredOverride::Inherit | Set`、optional reasoning effort/initial Goalは`OptionalOverride::Inherit | Set | Clear`とし、必須値に意味不明なClearを許さない。

- Rust usecaseはmanaged policyとprovider/model capabilityから`LaunchConfigurationBaseline`を解決し、`baseline → Run default → Agent Node override`の順で`ResolvedLaunchConfiguration`を作る。effortのClearは`ProviderDefault`、GoalのClearはNoneを意味する。各fieldにprovenance、baseline/run/node revision、resolution id/versionを持たせ、canonical hashを算出する。
- Run開始時はbaseline、default、解決規則versionを`RunStarted`へappendし、これをdurable commit pointとする。run metadataはprojectionである。Node実行時のresolved snapshotもeventとして記録し、provider adapterが受理した後に初めてSession selected/effective configurationになる。
- Workflow NodeがNew Agentをlaunchする場合、全modeで`execution_attempt_id + resolution_id`からstableな`attempt_id`を導出し、`WorkflowNodeExecutionRequested + AgentLaunchAttemptStarted { origin: WorkflowNode(...) }`をworkflow/launch stream横断のlocal atomic batchで開始する。Bypassだけは先に`WorkflowNodeBypassPrepared + BypassChallengeIssued`で`WaitingBypassConfirmation`へ進め、確認後の開始batchへconsumeを追加する。non-Bypassはchallenge無しで共通開始batchへ進む。launch完了時の`AgentLaunchCompleted`と`WorkflowNodeAgentBound { attempt_id, session_id }`、失敗/取消時の`AgentLaunchFailed/Cancelled`と`WorkflowNodeAgentLaunchFailed/Cancelled { attempt_id, reason }`をそれぞれ同じbatchにする。crash/restart時はorigin相関から既存attemptをresumeし、retryは新しい`execution_attempt_id`を発行してprovider/sessionを二重作成しない。
- workflow `task` と initial Goal spec は別フィールドである。Goal を作る場合は新しい `goal_id` を発行し、既存 Session Goal の暗黙継承・復活を行わない。
- templateにBypassを保存しても権限付与ではない。Run/Nodeの各execution attemptでmanaged policyを再検査し、`run_id + node_id + execution_attempt_id + resolution_id + canonical resolved hash`へ束縛したone-time challengeとClaude launch opt-in等provider gateを検証する。失効・consumed・異なるattempt/configurationのchallengeを再利用しない。
- 旧Run / external execution restoreに必要な設定が無い、または現providerで非対応になった場合はresolution id、field、reason、allowed actionsを持つ`WorkflowWaitingConfiguration`へ遷移する。`Edit`固定へfallbackしない。

## turn 状態機械（明示化）

ST-3 の解消として、phase 遷移を規範として一覧化する（実装は本表と 1:1 対応のモジュールに分解する）:

| phase | 入力イベント | 遷移先 | 必須アクション |
|---|---|---|---|
| Idle | start_turn（独立activation済み、configuration / Goal ready） | Streaming | policy/gate再検証後、`TurnStarted`にprovider/model/mode/effective effort/protocol identity/goal refを記録し、provider I/Oへ進む |
| Idle | start_turn（turn/startでactivation） | StartingTurn | human message＋`TurnStartRequested`をatomic append。queue/Bypass intentも同batch。early provider eventはbuffer |
| StartingTurn | provider ack/readback＋canonical batch成功 | Streaming | Session pending activationなら`SessionConfigurationActivated + TurnStarted`、queue per-turn overrideなら`QueueItemStarted + TurnStarted`をatomic commitしbuffer公開。queue値でSession effectiveを変更しない |
| StartingTurn | timeout / provider ack後commit失敗 | Idle | provider turn/config observation付き`TurnStartReconciliationRequired`。interrupt/readbackし、send/config/Goal/queueをblock |
| StartingTurn | interrupt 要求 | Interrupting | 起点に関係なく`TurnInterruptRequested + QueuePaused`をdurable化。late ackは通常commitせずinterrupt/finalize、結果不明はTurnStart reconciliation |
| Idle | start_turn（NeedsResolution / ReconciliationRequired / ProtocolIncompatible） | Idle | provider I/O を行わず理由とRustが算出した解決操作を返す（I14/I15） |
| Idle | Fatal | Idle | Notice(Error) durable 化（I12、RT-6） |
| Streaming | PartsMerged / TokenUsageUpdated | Streaming | merge → 定期 flush（I3） |
| Streaming | PermissionRequested | WaitingPermission | permission part durable 化＋state change emit |
| Streaming | TurnCompleted | Idle | finalize（Final 化→TurnResult 記録→workflow 通知→queue 評価） |
| Streaming | interrupt 要求 | Interrupting | backend interrupt 送出 or 予約（I5） |
| Streaming | Fatal / stream 終了 | Idle | Interrupted{Crash} で finalize（I1） |
| WaitingPermission(Pending) | respond_permission | WaitingPermission(Responding) | `PermissionResponseRequested`をdurable化してからbackend送出。secret plaintextはephemeralのみ |
| WaitingPermission(Responding) | response ack | Streaming | `PermissionResolved`をappendしeffectiveを確定 |
| WaitingPermission(Responding) | provider explicit reject（requestはPending） | WaitingPermission(Pending) | `PermissionResponseRejected`で旧responseを終端し再回答可能にする。secret ephemeral値は破棄 |
| WaitingPermission(Responding) | timeout / restart recovery | WaitingPermission | cancel/tool-start/readbackを相関。確定不能はpermission reconciliationで再回答をblock |
| WaitingPermission | permission cancel（CLI 取り下げ） | Streaming | Cancelled(effective=false) へ更新（I7） |
| WaitingPermission | interrupt 要求 | Interrupting | pending permission を Cancelled(effective=false) へ畳む（I7）＋backend interrupt 送出 or 予約（I5） |
| WaitingPermission | TurnCompleted | Idle | 未解決 permission を Cancelled に畳んで finalize（現行踏襲） |
| Interrupting | TurnCompleted / 猶予超過 | Idle | finalize。猶予超過時は強制終端＋Interrupted{Timeout} |
| （全 phase） | close_session / app quit | Idle | I1 の手順（flush → finalize → close） |
| Idle | configuration update request | Idle | CAS / capability / policy / challenge検証→durable intent→adapter送信（I14）。非IdleはBusy |
| Idle | configuration ack success | Idle | canonical eventでselectedをcommit。live以外はawaiting化（I14） |
| Idle | configuration activated | Idle | canonical event後にeffectiveを進めて`Synced`化（I14/I16） |
| Idle | configuration reject | Idle | rejectionを記録し旧selected/effectiveを維持（I14） |
| Idle | Goal user transition request | Idle | goal CAS→durable intent→strategy/effects検証→adapter送信（I15） |
| （全 phase） | provider/evaluator Goal observation | 同一 phase | `observation_id`付きraw snapshotを先にdurable化し、goal id/revision照合→遷移検証→evidence付きcommit。対応transitionが無い競合は独立`reconciliation_id`で回復（I15） |
| （全 phase） | timeout / partial apply / canonical commit failure / conflict | 同一 phase | 対象aggregateを`ReconciliationRequired`。新規turnをblockしreadback/reapply/rollback等（I14/I15） |
| Idle | control-plane protocol drift | Idle | `ProtocolIncompatible`をdurable化し、新規turnをfail-closedでblock（I10/I14） |
| Streaming / WaitingPermission / Interrupting | control-plane protocol drift | Interrupting → Idle | `ProtocolIncompatible`をdurable化し、pending permissionをCancelled、provider interrupt、猶予後`Interrupted{ProtocolIncompatible}`で必ずfinalize。以後send block（I1/I5/I7/I10） |

queue はphaseと直交するevent-sourced sub-state: `items: [{ input_ref, configuration_snapshot, goal_ref, status: Queued/AwaitingBypassConfirmation/Starting/Started/Failed/Cancelled/NeedsResolution }]`＋`paused: bool`（I4/I5）。configurationとGoalも別aggregateとしてselected/effective/pending/sync state、current/pending/sync stateをread modelから観測できる。

## シナリオ別保証（受け入れ基準の骨子）

| シナリオ | 保証 |
|---|---|
| 正常完了 | TurnResult(Completed{stop_reason}) が durable。reload 後も同一表示（P1） |
| ユーザー Stop | 最悪 10s で Idle。queue は paused。入力欄・queue は無損失 |
| streaming 中に tab close / backend 切替 / アプリ終了 | 再オープン時: 本文は flush 済みまで表示、turn は Interrupted{SessionClosed}、スピナー・permission 残骸なし |
| クラッシュ → 再起動 | dangling turn が Crash で回収済み。損失は最大 1s の本文のみ（I2/I3） |
| resume 失敗 | 両 backend とも新規 establish ＋ Notice。以後のターンは正常。editor_context 保全（I9） |
| queue に 2 件積んで再起動 | queue が復元され、明示操作で実行再開できる（I4） |
| permission 待ち中に interrupt | ダイアログは即 Cancelled 表示。誤記録なし（I7） |
| 5 mode の切替成功 | provider ackとcanonical selected/activated event後だけrevisionと表示が更新され、次turn/reload/resumeでも同じmode（I14） |
| selected commit前のprovider reject | UIは旧selected/effectiveのまま、非対応理由またはNotice(Error)が出る（I8/I14/I16） |
| selected commit後のactivation reject | new selected / old effectiveを保ってReconciliationRequired。rollbackは新revisionとして記録しCASを逆行させない（I14/I16） |
| provider ack 後に canonical event append 失敗 | old stateへ戻ったふりをせずReconciliationRequired。未完了requestからrestart後もreadback/reapply/rollbackを継続（I14/I15） |
| canonical event 後に SessionMeta cache 更新失敗 | provider driftとは扱わずPersistFailureを表示し、event logから再投影する（I14） |
| next-turn / restart effort 更新 | selected と effective を別表示し、activation 後だけ effective revision が進む（I14/I16） |
| stale base revision から並行更新 | provider へ送信せず conflict。snapshot 再取得後に再操作できる（I14） |
| model 変更で推論レベル非対応 | silent fallback せず、利用可能な推論レベルと解決操作を表示する（I16） |
| Goal を設定→pause→restart→resume | Goal専用pending/sync stateとid/objective/status/revisionを保持し、strategy/effectsを表示して再開する（I15） |
| Claude `/goal` でGoal設定/編集 | durable Goal intent後にCLI commandを実行。StartsTurn等のeffectsを表示し、Goal canonical event＋TurnStartedのatomic batch commit後にだけ公開（I15） |
| Goal 完了後に古い queue item を drain | goal id / revision 不一致を検出し、Goal を復活させず rebase / 取消を要求する（I4/I15） |
| Bypass 選択 | execution固有guardへ束縛し、managed policy再検査後にconsume＋durable intentをlocal atomic batch commitしてからproviderへ送る。失敗後も同一intentだけ再試行可（I14） |
| New Agent create応答が喪失 | correlation/idempotency keyとstageからreadback/reuse/cleanup。存在不明かつ安全なlookup/createが無ければRecreate不可（I14） |
| initial Goal適用中にcrash | local SessionとGoal transitionの相関を復元し、同じtransitionをreconcileしてGoal/turnを重複起動しない（I14/I15） |
| compiled schemaと実行CLIが不一致 | Session前はlaunch attempt、Session後はAgentProtocolStateにProtocolIncompatibleを表示し新規turnを開始しない。部分identityとraw control refを監査可能（I14） |
| Auto / Bypass で workflow checkpoint 到達 | mode に関係なく workflow は停止し、人間の承認・却下・再指示を待つ（I15） |
| stdout に識別済みdiagnostic非JSON行 | 両backendともskip件数をNotice化して継続。未分類/malformed/oversize frameはProtocolIncompatibleでsendをblock（I10） |

## backend 差の吸収規約

| 関心事 | 保証水準 | Claude | Codex |
|---|---|---|---|
| interrupt | 常に受理・最悪 10s で終端（I5） | synthetic abort 踏襲 | turn_id 予約＋猶予強制終端を追加 |
| 生存シグナル | 長考中もシグナル継続（I11） | thinking delta ＋ keep_alive | reasoning delta 購読を追加（CX-3） |
| stdout 頑健性 | 識別済みdiagnosticだけskip、content-plane未知はNotice、未分類/malformed/oversizeはfail-closed（I10） | 既存を分類付き可視化へ強化 | 共通ラッパ導入でprocess即死を閲覧可能なProtocolIncompatibleへ置換 |
| steer | 非対応は queue へフォールバック（I6） | queue | queue（将来 turn/steer 対応時に置換） |
| resume 失敗 | Cleared → 再 establish ＋ Notice（I9） | 無言復旧を通知付きに | 恒久死を回復経路に接続 |
| AgentMode | schema supportとruntime availabilityを分け、近似・非対応を明示（I14） | `default / acceptEdits / plan / auto / bypassPermissions`。auto availabilityとBypass launch opt-inを実行時検証 | approval / sandbox / reviewerとruntime取得した`collaborationMode/list` presetを構成 |
| Auto review | provider reviewerの境界を広げず、全statusを監査 | classifier active / manual fallbackを`AutoOperationalState`へ | approved/deniedだけAuto解決。inProgress/timedOut/abortedはactivity/未解決へ |
| ReasoningEffort | selected/effective/unknown、source、option/default/反映時点を返す（I16） | runtime API/readbackが無ければpinしたversion×model table。検証不能はUnsupported | `model/list`の広告順・defaultを保持しturn overrideへ写像 |
| Goal | current Goal最大1件、独立id/revision/pending/sync、操作別strategy/effects（I15） | `/goal`はProviderCliCommand。setはStartsTurn、pause/resume emulationは進捗reset等を明示 | typed RPC、notification、全statusとread-only accountingを`ProviderGoalSnapshot`へ |
| protocol identity | generated schemaと実行binary/flags/capabilitiesの一致を必須化（I14） | CLI/SDK versionとlaunch gatesを照合 | executable version、schema hash、experimental flags、initialize capabilitiesを照合 |

## トレーサビリティ（本書が解消する問題）

| 問題 ID | 不変条件 / 節 |
|---|---|
| RT-1 | I1 |
| RT-2 | I2, I3 |
| RT-3, OB-3 | I4 |
| OB-4 | I4（cancelled マーク） |
| OB-5 | I5（queue paused） |
| OB-6, RT-7 | I4（起動失敗の可視化） |
| OB-1, SD-2 | I5 |
| OB-2 | I6 |
| OB-8 | I9 |
| CL-1 | I7 |
| CL-2, CX-6 | I8（ack 駆動） |
| ST-4, RT-4, RT-8 | I8（失敗の着地・自己修復） |
| SD-1 | I9 |
| SD-3 | I10 |
| SD-4, ST-9 | I11 |
| RT-6, RT-5 | I12 |
| FE-1, FE-2, FE-3（backend 側の裏付け） | I7, I12, I3 |
| CX-2 | I7 の変種: elicitation は「応答義務のある要求」として permission 経路に載せる（無応答ハングの解消。語彙は V-D6 の Question を流用） |
| ST-3 | turn 状態機械の明示化 |
| ST-5 | I13 |
| #1445, #1446 | I14（正本・ack・revision・永続化・migration） |
| #1447 | I14（5 mode cross-backend mapping） |
| #1448 | I16（ReasoningEffort capability / 反映時点） |
| #1449 | I15（Goal lifecycle / provider capability） |
| #1450 | I4, I9, I14, I15, I16（workflow / queue / restart 継承） |

## 設計判断

- **L-D1（2026-07-15改訂）**: queueの正本はSession event logとする。enqueue/cancel/execution/started/failedをappend-only eventで記録し、session queue stateとmessage cancelled markerは再構築可能なprojectionにする。通常UIはactive/nonterminal projectionだけを保持し、terminal履歴はpage参照してfull-retentionを避ける。旧「session stateへ上書き、event replay外」の判断はcrash時atomicityを満たさないためsupersedeする。
- **L-D2**: interrupt の強制終端猶予は 10s（既存 Claude synthetic abort の実績値を共通保証に昇格）。
- **L-D3**: crash 回収は「検出は lazy・記録は durable」。起動時の全セッション走査はしない（開いたセッションから回収）。
- **L-D4**: queue 取り消しは human message の削除ではなく cancelled マーク。履歴の誠実性と OB-4（復元コンテキスト除外）の両立。
- **L-D5**: interrupt 後の queue は paused（自動 drain 禁止）。「止めたのに続く」（OB-5）の解消を優先し、再開は明示操作。
- **L-D6**: Agent configurationはdiscriminated patch、write-ahead intent、canonical event commitを使い、selected/effective/pending/reconciliationを分離する。event logが正本、SessionMetaは再構築可能なprojection/cacheであり、外部providerとのatomic commitを仮定しない。
- **L-D7**: GoalはSessionごとにcurrent最大1件、configurationとは独立したgoal id/revision/pending/sync stateを持つ。workflow nodeは開始時にinitial Goal specを解決し、新規Goalとして記録する。
- **L-D8**: Reasoning effortのselected/effective/unknown、値・説明・default・反映時点はprovider APIまたはprotocol identityにpinしたcompatibility table駆動。Releash固有の固定enumやTokenUsageからの推測は持たない。
- **L-D9**: workflow definitionはrequired/optional overrideを分けた`AgentConfigurationTemplate`、Run/Node実行はprovenance付き`ResolvedLaunchConfiguration`、provider ack後のSessionは`AgentSessionConfigurationProjection`と型を分ける。
- **L-D10**: queue item は enqueue 時の effective configuration snapshot と Goal ref を保持し、後続の Session 設定変更では書き換えない。変更は明示 rebase のみ。

## 確定事項（2026-07-07、2026-07-15 レビューで確定）

1. **I5 / L-D5**: interrupt 時は queue を**常に paused** にする（選択式は不採用）。再開は queue chips の明示操作。
2. **L-D4**: 取り消した queue メッセージは **cancelled マークで transcript に残す**（非表示は不採用）。復元コンテキストからの除外は共通実施。
3. **I2**: crash 回収は**中断チップのみ**（起動時のバナー・ダイアログ通知は出さない）。古いセッションの一斉回収も静かに行う。
4. **I14 / L-D6**: provider/model/mode/reasoning effortはRust-owned configuration、Goalは別aggregateとする。canonical eventをcommit pointにし、provider ack後のappend failureを旧値維持とは扱わない。
5. **I15 / L-D7**: GoalはSessionごとにcurrent最大1件、独立goal id/revision/pending/sync stateとする。restart/resume/workflow継続で保持し、strategy/scope/effects付きNative/Emulated/Unsupportedを明示する。暗黙prompt fallbackは行わない。
6. **I16 / L-D8**: 工数はmodelの応答・推論強度を調整するbehavioral signalとし、selected/effective/unknownとcapability source/反映時点に従う。TokenUsageや各種budget、厳密な上限は範囲外。
7. **Auto / Bypass**: Auto の判定は provider classifier / reviewer が行い、Releash は結果を監査する。Bypass は Rust 側の managed-policy 検査と二段階確認を必須とする。どちらも workflow checkpoint を迂回しない。
8. **L-D9 / L-D10**: workflow template、解決済み launch config、Session projection、queue snapshot を別型にし、暗黙 inheritance / fallback / mutation を行わない。
9. **protocol identity**: generated schemaとspawnしたCLI/flags/capabilitiesをinitialize時に照合し、control-plane driftはProtocolIncompatibleとしてfail-closedにする。
