# Design

## The actual design

### Architecture

Issue #1499 は、通常 send、terminal、pending recovery、close / quit を別々の応急処置として追加せず、Rust-owned の五つの境界で閉じる。

1. caller 指定 operation identity による single-send acceptance
2. 現行 file store 上の一つの Phase 0 closure commit
3. external effect より先に確定する durable obligation
4. turn ごとに一つの terminal owner と stale-result fence
5. 全 graceful quit surface を束ねる bounded shutdown coordinator

物理 store の将来正本は specs/milestone-84-agent-chat-stabilization/d3-durable-event-store-design.md、close / quit の利用者可視意味論は specs/milestone-84-agent-chat-stabilization/close-quit-decision-table.md とする。本書は #1499 の Phase 0 runtime 実装と public contract を定め、D3 にある F3 の SQLite 実装を重複定義しない。

公開二段階send、未受理send一覧 / content query、resource単位data lifecycle、backup / reset系runtime APIは実装しない。Phase 0 store 内部で write set を準備する状態は public send lifecycle ではなく、commit 前の transaction implementation detail である。

#### Layer ownership

| Layer | Responsibility | Placement |
| --- | --- | --- |
| domain | operation / terminal / obligation identity、closed transition、receipt、fence、action policy | src-tauri/src/domain/agent_session/ |
| usecase | send acceptance、terminal arbitration、Stop deadline、recovery、feedback、shutdown orchestration | src-tauri/src/usecase/agent_session/ と src-tauri/src/usecase/shutdown_coordinator.rs |
| adaptor/gateway | Phase 0 closure、direct lookup index、bounded snapshot / cursor、legacy bootstrap | src-tauri/src/adaptor/gateway/agent_session/phase0_reliability/ |
| adaptor/controller | Tauri / WebSocket DTO validation、usecase 呼出し、presenter mapping | src-tauri/src/adaptor/controller/ と src-tauri/src/adaptor/protocol/ |
| infrastructure | provider start / interrupt / close、workflow / child shutdown、native quit ingress | 既存 agent runtime、workflow runtime、platform lifecycle |
| frontend | caller operation identity と composer snapshot の対応、backend result の mirror、明示 action | src/hooks/useSessionStore.ts、src/hooks/useAgentChat.ts、MessageInput.tsx |

frontend は receipt、status、failure、retryability、terminal winner、recovery action、shutdown state を合成しない。operation identity の生成は transport caller の責務だが、その形式検証、同一性、payload binding、受理判断は Rust が所有する。

#### Authority and commit points

| Fact | Phase 0 authority | Commit point | Derived output |
| --- | --- | --- | --- |
| send acceptance | operation record、exact request binding、human input、turn または queue identity、必要な obligation を含む closure | transaction inventory root が closure を参照し required sync が完了した時点 | legacy message / event / meta、UI emit |
| streaming part | part identity、target turn、expected revision を含む closure | 同じ root cutover | live event、message projection |
| terminal | terminal record、final parts、assistant message、session / permission / queue / Stop resolution を含む closure | 同じ root cutover | event / message / meta materialization、notification |
| pending external effect | effect identity、capability、owner / runtime fence、safe observation を持つ obligation | external effect の前に同じ root cutover | pending recovery page |
| shutdown plan | intent、target pages、preexisting recovery snapshot、plan phase | 全 page を参照する plan root cutover、その後の Activated cutover | progress emit、one-shot exit permit |
| legacy bootstrap | immutable source inventory、staging root、parity result、authority pointer | authority pointer の one-shot cutover | progress projection |

closure file や materialized legacy file の存在だけを commit 根拠にしない。root から到達できる closure だけが authority であり、root 未到達の staging data は public query へ出さない。materialization と notification は commit 後の再実行可能な派生処理である。

#### Mutating path classification

| Path | Effect 前に必要な fact | External effect | Completion |
| --- | --- | --- | --- |
| normal / new-session send | Accepted receipt、human input、turn / queue identity、provider establish dependency | provider create / resume、turn start | status または terminal closure |
| queued send | Accepted receipt、queue item、dispatch guard | queue が実行可能になった後の turn start | started status または terminal |
| permission response | exact payload と permission identity を持つ obligation | provider response | permission settlement |
| provider establish | create / resume intent と effect identity | provider create / resume | observation と依存 send status |
| streaming | part identity と expected turn fence | なし | part と read model |
| Stop | target turn、queue pause、deadline permit、terminal obligation | provider interrupt | terminal と Stop resolution |
| session close / open archive | lifecycle target、active turn terminal または Idle closure、queue pause、close obligation | provider runtime close | Closed / Archived と close result |
| backend switch | Idle guard、old backend、queue pause、close obligation | old runtime close | selected / effective backend |
| backend recovery | stable recovery identity と effect obligation | provider readback / resume | recovery result と publication |
| workflow shutdown | execution identity と shutdown obligation | workflow / child stop | workflow terminal |
| application quit | fixed intent、all target pages、preexisting recovery snapshot | Activated 後の target effects と process exit | terminal plan または recovery exit |

各 usecase の result は must-use とし、mutating path の warn-only result discard を禁止する。対象 entry point ごとに、上表のどの closure と failure landing を使うかを owner test で固定する。

#### Production cutover

1. Phase 0 bridge を初めて読む起動では exclusive app-data writer lock を取得し、mutation / provider effect admission を閉じたまま automatic bootstrap を開始する。
2. staging generation は authority ではない。source inventory、record parity、required sync が成功した後だけ authority pointer を Legacy から Phase0 へ一度切り替える。
3. 切替前は legacy reader、切替後は Phase 0 writer / reader の一方だけを使う。dual write と record 単位 fallback は行わない。
4. #1499 対象 path は同じ release で closure commit へ切り替え、旧 write-success 判定と event 全履歴 scan fallback を通常 path から外す。
5. F3 migration 中は mutation admission を閉じ、D3 の one-shot import と parity check を行う。F3 authority cutover 後に Phase 0 writer へ戻さない。

D3 gate は、storage / transaction、stream / global order / index、CAS / idempotency、durability / crash atomicity、worker / error、versioned envelope / raw preservation、projection / rebuild、bounded read / watch / replay、obligation inventory / GC、legacy cutover / dual-write、crash-resumable migration、backup / restore、performance の13分類を正本上で完結させる。各分類に採用案、却下案、理由、failure behavior、migration verification、具体上限を必須とする。backup / restore はF3物理storeの設計判断であり、#1499のruntime commandやpublic routeを追加しない。

### Interface

#### Common identity and safe failure

SendOperationId は caller opaque の 1..=128 bytes、文字集合 A-Z a-z 0-9 . _ : - である。scope は current installation authority であり、Tauri / WebSocket connection、surface、session ごとには分けない。WebSocket outer request_id は transport request の重複制御だけに使い、SendOperationId、Stop request_id、quit request_id の代わりにしない。

以下の設計用aliasは保存・公開整数の同じdomainを名前で区別するためだけに使う。wireでは後述のcanonical decimal stringへ写し、別の数値domainを作らない。

    pub type PersistedRevision = u64;
    pub type PersistedCount = u64;
    pub type PersistedEpoch = u64;
    pub type PersistedOrdinal = u64;
    pub type Sha256Digest = [u8; 32];
    pub type HmacSha256Digest = [u8; 32];

SafeOperationFailure の closed kind は次の19値である。

    pub enum SessionOperationFailureKind {
        StorageUnavailable,
        StorageCorrupt,
        MigrationBlocked,
        PersistFailure,
        ProtocolIncompatible,
        ProviderUnavailable,
        ExternalEffectFailed,
        OutcomeUnknown,
        DeadlineExceeded,
        CapacityExceeded,
        StopCapacityExceeded,
        ShutdownAuthorityMismatch,
        TargetRevisionChanged,
        OwnerRevisionChanged,
        RuntimeGenerationChanged,
        InvalidEffectIntent,
        PreviousShutdownReconciliationRequired,
        PreviousShutdownCompactionPending,
        Internal,
    }

    pub struct SafeOperationFailure {
        pub kind: SessionOperationFailureKind,
        pub retryable: bool,
        pub label: BoundedNoticeText,
        pub detail: Option<BoundedNoticeText>,
        pub correlation_id: String,
    }

    pub struct BoundedNoticeText {
        pub value: String,
        pub truncated: bool,
        pub original_bytes: Option<PersistedCount>,
        pub digest: Option<String>,
        pub correlation_id: Option<String>,
    }

label は UTF-8 160 bytes、detail は2048 bytes以下に縮約する。切り詰めた場合だけoriginal_bytesとdigestをSomeにし、SafeOperationFailure内のnested correlation_idは常にNone、failure identityはtop-level correlation_idだけに置く。path、secret、raw SQL、provider payload、raw storage / provider error は public field、log、telemetryへ出さない。

`ExitCoupledOutcomeUnknown`はfailure kindではなく`SafeEffectObservation`だけに置く。PayloadConflict、NotFound、InvalidRequest、cursor / snapshot error は command / query 固有の typed application error であり、上の durable safe failure kind へ水増ししない。OutcomeUnknown と acceptance 後の failure は command result / projection に置き、transport error と二重符号化しない。PayloadConflict だけは保存済みidentity bindingとのdeterministicなcommit前競合であり、SendAgentMessageResult、StopResult、ApplicationQuitResult、SessionLifecycleResultのvariantを増やさず、send / Stop / quitはTauri / WebSocket、SessionLifecycleはTauri専用のtyped errorへ写す。

    pub enum PayloadConflictIdentity {
        Send { operation_id: SendOperationId },
        Stop { request_id: String },
        ApplicationQuit { request_id: String },
        SessionLifecycle { request_id: String },
    }

    pub enum AgentSessionInternalErrorClass {
        InvalidRequest,
        PayloadConflict { identity: PayloadConflictIdentity },
        NotFound,
        CursorMismatch,
        CursorExpired,
        SnapshotMismatch,
        DetailsCompacted,
        QueryBusy,
        DeadlineExceeded,
        CapacityExceeded,
        FeedbackCapacityExceeded,
        BootstrapInProgress,
        ShutdownInProgress,
        ResponseTooLarge,
        StorageUnavailable { failure: SafeOperationFailure },
        Internal { correlation_id: String },
    }

`AgentSessionInternalErrorClass`はgateway / storage / protocol adaptorがusecase境界へ渡すprivateな分類supersetであり、application usecaseのreturn型でも公開error型でもない。各usecase methodは後掲matrixから生成したendpoint固有のnamed enumを直接返し、その行にないvariantを型として持たせない。private分類からendpoint enumへの変換も同じdeclarative matrixから生成し、正常経路はrow内variantだけをtotal matchする。row外分類は契約違反としてoutermost boundaryでcorrelation ID付き`Internal`へ閉じる防御境界であり、通常変換のfallbackにしない。WebSocketだけの`RequestIdConflict | RateLimited`とHTTP / close-code errorはprivate supersetへ加えない。

#### Normal send

public send は既存 send_agent_message 一回だけで完結する。

    pub struct SendOperationId(String);

    pub struct SendAgentMessageCommand {
        pub operation_id: SendOperationId,
        pub target: AgentSendTarget,
        pub content: String,
        pub images: Vec<SendImageInput>,
        pub mentions: Vec<MentionReference>,
        pub editor_context: Option<EditorContext>,
        pub active_turn_policy: ActiveTurnSendPolicy,
    }

    pub struct AgentSendTarget {
        pub chat_session_id: Option<String>,
        pub worktree_path: String,
        pub permission_mode: PermissionMode,
        pub plan_mode: bool,
        pub backend_id: Option<String>,
        pub model_id: Option<String>,
    }

    pub struct SendImageInput {
        pub bytes: Vec<u8>,
        pub media_type: String,
    }

    pub struct MentionReference {
        pub file_path: String,
        pub start_line: Option<u32>,
        pub end_line: Option<u32>,
    }

    pub struct EditorContext {
        pub active_editor_path: Option<String>,
        pub open_editor_paths: Vec<String>,
        pub selection: Option<EditorSelection>,
    }

    pub struct EditorSelection {
        pub file_path: String,
        pub start_line: u32,
        pub end_line: u32,
    }

    pub enum ActiveTurnSendPolicy {
        QueueAfterCurrent,
    }

    pub enum SendAgentMessageResult {
        Accepted {
            receipt: SendAcceptanceReceipt,
            status: SendExecutionStatus,
        },
        RejectedBeforeCommit {
            operation_id: SendOperationId,
            failure: SafeOperationFailure,
        },
        OutcomeUnknown {
            operation_id: SendOperationId,
        },
    }

    pub struct SendAcceptanceReceipt {
        pub operation_id: SendOperationId,
        pub session_id: String,
        pub input_ref: String,
        pub disposition: SendDisposition,
    }

    pub enum SendDisposition {
        StartedTurn { turn_id: String },
        Queued { queue_item_id: String },
    }

    pub enum SendExecutionStatus {
        AwaitingProviderStart {
            dependency_obligation_ids: Vec<String>, // 0..=3
        },
        Queued {
            queue_item_id: String,
            reserved_turn_id: String,
        },
        ProviderStartReserved { obligation_id: String },
        Running { turn_id: String },
        ReconciliationRequired {
            failure: SafeOperationFailure,
        },
        Failed {
            failure: SafeOperationFailure,
        },
        Terminal { result: TurnResult },
    }

    pub struct SendObligationStatusView {
        pub obligation_id: String,
        pub kind: ObligationKind,
        pub lifecycle: ObligationPublicLifecycle,
        pub safe_observation: Option<SafeEffectObservation>,
        pub safe_failure: Option<SafeOperationFailure>,
        pub available_actions: Vec<OperationAction>, // max 5
    }

    pub struct GetAgentSendOperationRequest {
        pub operation_id: SendOperationId,
    }

    pub enum AgentSendOperationView {
        Accepted {
            receipt: SendAcceptanceReceipt,
            status: SendExecutionStatus,
            obligations: Vec<SendObligationStatusView>, // ordered, max 4
            available_actions: Vec<OperationAction>,    // max 5
        },
        OutcomeUnknown {
            operation_id: SendOperationId,
        },
    }

既存Tauri DTOはchat_session_id、worktree_path、content、permission_mode、plan_mode、backend_id、model_id、images、mentions、editor_contextを維持し、operation_idだけをadditiveに必須化する。controllerはabsentなimages / mentionsを空vectorへ正規化し、現行active-turn挙動をQueueAfterCurrentへ写す。新規sessionはchat_session_id=Noneで表し、別のlaunch attempt IDやdraft hashをpublic inputへ追加しない。

built-in desktop callerはfrontendからTauriを一回だけinvokeする。controllerがrequestを受けた後の最初のRust stepで、normal send usecaseのdispatch、canonical acceptance writer、provider effectのいずれよりも前に、operation IDと送信時snapshotをowner-onlyなcaller-attempt journalへ一つのlocal transactionで保存する。journal保存が失敗した場合は`RejectedBeforeCommit`としてprovider I/O、human message、turn / queue、durable operation viewを0件にし、frontendのsnapshotを保持する。controller到達前のUI draftは未送信のままであり、journal commit後は同じprincipal、operation ID、exact command bytesを保持する。response喪失またはUI process restart時はjournalから同じcommandだけを再開し、identity queryがAcceptedを返した時またはdeterministicなRejectedBeforeCommitを確認した時だけentryをclearする。OutcomeUnknown、query failure、process restartでは保持し、別operation IDを生成しない。このjournalはpublic prepared lifecycle、prepared list、content resolverではなく、built-in callerが自ら発行済みの一件を回復するlocal outboxである。WebSocketその他のcallerはrequest送信前にoperation IDとexact payloadを自身のdurable stateへ保存することをprotocol preconditionとし、serverはcallerが失ったidentityをpayload scanで探索しない。

send result の境界は次のとおりである。

| State | Immediate result | Identity query |
| --- | --- | --- |
| validation または rollback 確認済みの commit 前 failure | RejectedBeforeCommit | NotFound |
| writer の commit 有無を確認不能 | OutcomeUnknown | writer 未解決中は OutcomeUnknown |
| canonical acceptance 済み | Accepted | 同じ receipt と latest status を持つ Accepted |
| Accepted 後の effect 結果不明 | Accepted + ReconciliationRequired | 同じ receipt を持つ Accepted |
| current authority 内の unknown ID | 該当なし | NotFound |

PayloadConflict は、同じ installation principal と operation ID に保存済みの exact request binding と異なる command を提示した deterministic pre-commit application error である。既存 receipt / status を変更せず effect 0件で返す。別principalから既存operation IDを提示した場合はidentityの存在を開示せずNotFoundを返し、同じIDの別operation、receipt、status、effectを作らない。lookupとbinding比較は成功・不一致を含めて同じbounded pathを通す。SendAgentMessageResult の3variantを増やさず、Tauri send application errorはoperation_id、WebSocketはAgentSessionWsErrorV1::PayloadConflictのSend identityを返す。

#### Stop

    pub struct StopOperationId(String);

    pub struct StopAcceptanceReceipt {
        pub operation_id: StopOperationId,
        pub session_id: String,
        pub target_turn_id: String,
        pub accepted_at: String,
    }

    pub struct StopAgentSessionCommand {
        pub request_id: String,
        pub session_id: String,
        pub target_turn_id: String,
        pub expected_session_revision: PersistedRevision,
    }

    pub enum StopResult {
        Accepted { receipt: StopAcceptanceReceipt },
        RejectedBeforeAcceptance { failure: SafeOperationFailure },
        OutcomeUnknown { operation_id: StopOperationId },
    }

    pub enum StopOperationView {
        Accepted { receipt: StopAcceptanceReceipt },
        Terminal {
            receipt: StopAcceptanceReceipt,
            resolution: StopResolutionResult,
            result: TurnResult,
        },
        ReconciliationRequired {
            receipt: StopAcceptanceReceipt,
            failure: SafeOperationFailure,
            available_actions: Vec<OperationAction>,
        },
        OutcomeUnknown { operation_id: StopOperationId },
    }

    pub enum StopResolutionResult {
        Succeeded,
        Superseded,
    }

    pub struct GetStopOperationRequest {
        pub operation_id: StopOperationId,
    }

Stop request_id は1..=128 bytesの`[A-Za-z0-9._:-]`で、current installation authority 内の caller retry key である。形式不正はInvalidRequestとしてStop受理、provider interrupt、terminal変更を0件にする。exact payloadは`session_id / target_turn_id / expected_session_revision`の3 fieldであり、同じkeyと3 field byte-equivalentの場合だけ同じStop operationへjoinする。同じkeyでsession、turn、expected revisionの一つでも異なる場合はPayloadConflict、effect 0件である。別keyでも同じ未解決session / target turnなら既存Stopへjoinし、後続expected revisionをcaller bindingへ保存するが初回Acceptedのrevision guardを変更しない。

Accepted Stop は target turn、queue pause、10秒 absolute deadline、process-wide deadline permit を一つの closure へ入れる。異なる unresolved target は32件まで受理し、33件目は StopCapacityExceeded の RejectedBeforeAcceptance とする。terminal closure が Stop candidate を勝者にしたとき resolution は Succeeded、normal completion、Fatal、close など別 candidate が先に確定したときは Superseded である。

#### Pending recovery and actions

    pub struct ListPendingRecoveryRequest {
        pub filter: PendingRecoveryFilter,
        pub cursor: Option<String>,
        pub limit: u16,
    }

    pub enum PendingRecoveryFilter {
        All,
        Owner(ObligationOwner),
        Partition(PendingRecoveryPartition),
        ShutdownPlan { plan_id: String, epoch: PersistedEpoch },
    }

    pub enum PendingRecoveryPartition {
        ClosedSession,
        ArchivedSession,
        UnownedRuntime,
    }

    pub struct PendingRecoveryPage {
        pub inventory_revision: PersistedRevision,
        pub entries: Vec<PendingRecoveryView>,
        pub next_cursor: Option<String>,
    }

    pub struct PendingRecoveryView {
        pub obligation_id: String,
        pub owner: ObligationOwner,
        pub shutdown_association: Option<ApplicationShutdownAssociation>,
        pub kind: ObligationKind,
        pub lifecycle: ObligationPublicLifecycle,
        pub safe_observation: Option<SafeEffectObservation>,
        pub safe_failure: Option<SafeOperationFailure>,
        pub available_actions: Vec<OperationAction>,
        pub revision: PersistedRevision,
    }

    pub struct PendingRecoveryInventorySnapshotRef {
        pub inventory_revision: PersistedRevision,
        pub root_page_sha256: Sha256Digest,
        pub ranges: Vec<PendingRecoveryInventoryRangeRef>,
        pub record_count: PersistedCount,
        pub snapshot_sha256: Sha256Digest,
    }

    pub struct PendingRecoveryInventoryRangeRef {
        pub partition: PendingRecoveryPartition,
        pub first_key: Option<String>,
        pub last_key: Option<String>,
        pub record_count: PersistedCount,
        pub range_sha256: Sha256Digest,
    }

    pub struct GetPendingRecoverySnapshotRequest {
        pub plan_id: String,
        pub epoch: PersistedEpoch,
        pub snapshot: PendingRecoveryInventorySnapshotRef,
        pub partition: PendingRecoveryPartition,
        pub cursor: Option<String>,
        pub limit: u16,
    }

    pub struct PendingRecoverySnapshotPage {
        pub plan_id: String,
        pub epoch: PersistedEpoch,
        pub snapshot_sha256: Sha256Digest,
        pub partition: PendingRecoveryPartition,
        pub entries: Vec<PendingRecoveryView>,
        pub next_cursor: Option<String>,
    }

`get_pending_recovery_snapshot`の公開error型は後掲matrixから生成する`GetPendingRecoverySnapshotApplicationError`だけである。snapshot query専用の第二のerror正本は作らない。

current pending recovery は1 pageを200件かつencoded 4 MiB以下とする。`PendingRecoveryFilter::ShutdownPlan`はcurrent inventoryのsecondary keyを`(plan_id, epoch)`でpoint rangeし、entryの`shutdown_association`が両fieldとも一致するものだけを返す。別plan、別epoch、associationなしのentryを混ぜず、plan固定snapshotまたはfilterなしcurrent inventoryへfallbackしない。

shutdown planが固定したexact pending recovery snapshotはplan ID、epoch、snapshot identity、ClosedSession / ArchivedSession / UnownedRuntime partitionを必須にし、1 pageを200件かつdecoded 4 MiB以下とする。requestのplan ID / epoch / snapshot hashまたは保存済みplanのsnapshot refが一致しない場合は`SnapshotMismatch`、cursorのsnapshot / partition / last key / MACが一致しない場合は`CursorMismatch`、process restart・期限・保持期間の失効は`CursorExpired`、plan detailsが整理済みなら`DetailsCompacted`である。unknown partition tagはstoreを読まず`InvalidRequest`とし、validな3 partitionは該当entryが0件でもempty pageを返す。どのerrorまたはempty pageもcurrent inventoryへfallbackせず、partial count / entry / cursorを返さない。

    pub enum RecoveryActionKind {
        ReadAgain,
        RetrySameEffect,
        UseObservedResult,
        CancelIfSafe,
        KeepForManualResolution,
    }

    pub struct OperationAction {
        pub action_id: String,
        pub kind: RecoveryActionKind,
    }

    pub struct ResolvePendingRecoveryActionRequest {
        pub obligation_id: String,
        pub expected_revision: PersistedRevision,
        pub action_id: String,
    }

    pub struct ResolveShutdownTargetActionRequest {
        pub plan_id: String,
        pub epoch: PersistedEpoch,
        pub target_key: String,
        pub expected_plan_revision: PersistedRevision,
        pub expected_root_sha256: Sha256Digest,
        pub expected_target_state_sha256: Sha256Digest,
        pub action_id: String,
    }

    pub enum RecoveryActionResultClassification {
        Pending,
        Succeeded,
        ConfirmedNoEffect,
        Ambiguous,
        CancelledBeforeEffect,
        Unchanged,
    }

    pub enum RecoveryActionCommandResult {
        Completed { result: RecoveryActionResult },
        InProgress { action_id: String },
        Rejected { rejection: RecoveryActionRejection },
        ActionOutcomeUnknown { action_id: String },
    }

    pub enum RecoveryActionRejection {
        NotFound,
        RevisionConflict { current_revision: PersistedRevision },
        ActionUnavailable,
        TargetRevisionChanged,
    }

    pub struct GetRecoveryActionRequest {
        pub action_id: String,
    }

    pub enum RecoveryActionOperationView {
        InProgress { action_id: String },
        OutcomeUnknown { action_id: String },
        ReconciliationRequired {
            action_id: String,
            failure: SafeOperationFailure,
        },
        Completed { result: RecoveryActionResult },
    }

    pub struct RecoveryActionResult {
        pub action_id: String,
        pub receipt: RecoveryActionReceipt,
        pub resource: RecoveryActionResourceView,
    }

    pub enum RecoveryActionResourceView {
        Pending(PendingRecoveryView),
        ShutdownTarget {
            plan: ApplicationShutdownProjection,
            target: ShutdownTargetView,
        },
    }

    pub struct RecoveryActionReceipt {
        pub outcome: RecoveryActionOutcome,
        pub classification: RecoveryActionResultClassification,
        pub resource_revision: PersistedRevision,
        pub canonical_result_sha256: Sha256Digest,
    }

    pub enum RecoveryActionOutcome {
        Pending,
        Terminal,
        Unchanged,
    }

action identity は backend 発行の CSPRNG opaque value であり、current projection が提示した identity だけを client が echo する。action command に provider result、effect key、observation を入力させない。Completed attempt は action ID、outcome、classification、resource revision、canonical safe `RecoveryActionResourceView` bytes、そのschema versionとSHA-256をimmutable authorityとして保持する。presenterは保存済みbytesのdecode、closed pair、action / revision parity、hashを検証して`RecoveryActionResult`を再生し、current obligation、shutdown target、owner revision、plan detailsから再構築しない。したがって時間経過、restart、current resource revision更新、details compaction、current resource resolver failure後も同じresultをexact replayし、Completed record自体のdecode / parity / hash failureだけをInternalとしてfail closedにする。

許可する outcome / classification pair は Pending+Pending、Pending+ConfirmedNoEffect、Pending+Ambiguous、Terminal+Succeeded、Terminal+CancelledBeforeEffect、Unchanged+Unchanged の6組だけである。EffectStarted または ack を Succeeded へ、Ambiguous を ConfirmedNoEffect へ読み替えない。CancelIfSafe は kind 固有 policy と authoritative no-effect proof の双方がある場合だけ提示する。QueueExecution の cancel / rebase は #1404 のため提示しない。

action writer の結果を確認できない場合は command result の ActionOutcomeUnknown を返す。AgentSessionWsErrorV1 には ActionOutcomeUnknown variant を置かない。

#### Failure feedback

    pub struct SessionOperationFailureFeedback {
        pub feedback_id: String,
        pub attempt_id: String,
        pub session_id: String,
        pub operation: SessionOperationKind,
        pub failure: SafeOperationFailure,
        pub available_actions: Vec<SessionOperationFeedbackAction>,
        pub revision: PersistedRevision,
    }

    pub enum SessionOperationKind {
        Load,
        Send,
        Stop,
        Close,
        Archive,
        Restore,
        Fork,
    }

    pub enum SessionOperationFeedbackAction {
        Dismiss,
        RetryResolution { action_id: String },
    }

    pub struct GetSessionOperationFeedbackRequest {
        pub session_id: String,
        pub cursor: Option<String>,
        pub limit: u16, // 1..=32
    }

    pub struct SessionOperationFeedbackSnapshot {
        pub session_id: String,
        pub entries: Vec<SessionOperationFailureFeedback>,
        pub next_cursor: Option<String>,
        pub total_unresolved: PersistedCount,
    }

    pub struct DismissSessionOperationFeedbackCommand {
        pub session_id: String,
        pub feedback_id: String,
        pub expected_revision: PersistedRevision,
    }

    pub struct RetrySessionOperationFeedbackResolutionCommand {
        pub session_id: String,
        pub feedback_id: String,
        pub expected_revision: PersistedRevision,
        pub action_id: String,
    }

    pub enum SessionOperationFeedbackControlResult {
        Applied { snapshot: SessionOperationFeedbackSnapshot },
        Rejected { rejection: SessionOperationFeedbackControlRejection },
        Failed { failure: SafeOperationFailure },
    }

    pub enum SessionOperationFeedbackControlRejection {
        NotFound,
        RevisionConflict { current_revision: PersistedRevision },
        ActionUnavailable,
    }

feedback store は SessionMeta から独立した process-owned snapshot である。session data / meta を読めない command でも request の validated session ID をscopeとしてentryを追加できる。1 session の page は32件、process全体は512 unresolved entriesである。

dismiss と resolution retry は feedback identity と expected revision を必須にする。成功とretry再失敗は更新後snapshotを持つApplied、stale / unknown / action不正はeffect 0件のRejected、control自身のsafeなstorage failureはFailedを返して新しいfeedbackを再帰生成しない。retryが再び失敗したclosureは同じfeedback IDを保ち、failureとavailable actionsを置換し、revisionをexactly 1増やす一方、global unresolved countとsession内entry数を変えない。capacity slotを新たにreserveせず、retry元以外のentryを変更しない。capacity到達中もquery、dismiss、既存identityのresolution retryは通す。

#### Close and shutdown

close / archive / backend switch の public意味論は close-quit-decision-table.md をそのまま usecase policy tableへ写す。view closeはUI stateだけ、normal session closeとopen archiveはqueueを保持してpause、activeならSessionClosed terminal、Idleならsynthetic terminalなしとする。closed archiveはqueueを変更しない。backend switchはIdleかつpending permission / recovery / provider operationなしの場合だけ受理し、old runtime close後もnew runtimeは次sendまで開始しない。

view closeを除く三操作は、response喪失とrestartを同じoperationへ収束できる次の一つのcanonical usecaseを使う。caller request IDは1..=128 bytesの`[A-Za-z0-9._:-]`、backend operation IDはCSPRNG opaque valueである。

    pub struct SessionLifecycleOperationId(String);

    pub struct RequestSessionLifecycleCommand {
        pub request_id: String,
        pub session_id: String,
        pub expected_session_revision: PersistedRevision,
        pub action: SessionLifecycleAction,
    }

    pub enum SessionLifecycleAction {
        Close,
        ArchiveOpen,
        ArchiveClosed,
        SwitchBackend { backend_id: String },
    }

    pub struct SessionLifecycleAcceptanceReceipt {
        pub operation_id: SessionLifecycleOperationId,
        pub session_id: String,
        pub action: SessionLifecycleAction,
        pub accepted_expected_session_revision: PersistedRevision,
        pub accepted_at: String,
    }

    pub enum SessionLifecycleResult {
        Accepted {
            receipt: SessionLifecycleAcceptanceReceipt,
            current: SessionLifecycleOperationState,
        },
        RejectedBeforeAcceptance { rejection: SessionLifecycleRejection },
        OutcomeUnknown { operation_id: SessionLifecycleOperationId },
    }

    pub enum SessionLifecycleRejection {
        Busy,
        PendingOperation,
        RevisionConflict { current_revision: PersistedRevision },
        InvalidState,
        Failed { failure: SafeOperationFailure },
    }

    pub enum SessionLifecycleOperationState {
        InProgress,
        ReconciliationRequired {
            failure: SafeOperationFailure,
            available_actions: Vec<OperationAction>,
        },
        Completed { outcome: SessionLifecycleOutcome },
    }

    pub enum SessionLifecycleOutcome {
        Closed {
            terminal_result: Option<TurnResult>,
            queue_paused: bool,
        },
        Archived {
            source_was_open: bool,
            terminal_result: Option<TurnResult>,
            queue_paused: bool,
        },
        BackendSelected {
            backend_id: String,
            runtime_started: bool,
        },
    }

    pub struct GetSessionLifecycleOperationRequest {
        pub operation_id: SessionLifecycleOperationId,
    }

    pub enum SessionLifecycleOperationView {
        Accepted {
            receipt: SessionLifecycleAcceptanceReceipt,
            current: SessionLifecycleOperationState,
        },
        OutcomeUnknown { operation_id: SessionLifecycleOperationId },
    }

first requestはprincipal、request ID、session、expected revision、normalized actionを不変に束縛し、same key / same payloadは同じoperationを返し、same key / different payloadは`PayloadConflict { SessionLifecycle }`、別principalはNotFoundとする。different key / same principal / same unresolved session / same normalized actionは既存operationへjoinしてbindingだけを追加し、first receipt、revision guard、action、deadlineを変更しない。SwitchBackendはbackend IDまでをnormalized actionへ含める。different key / same session / different actionは`PendingOperation`でbinding、operation、effectを0件にする。same-key raceはcaller-key winner一件、different-key raceはsession single-flight winner一件とし、loserはwinnerを再読込してsame actionならjoin、different actionなら`PendingOperation`へ閉じる。session authorizationはoperation lookupより先に検査し、unauthorized command / cross-principal queryは存在を秘匿したNotFoundとする。Accepted後はqueue pause、必要なterminal intent、runtime close obligation、10秒deadlineを一つのclosureへ保存してからeffectを開始する。10秒以内にeffectまたはresultが確定しない場合もAccepted receiptを維持し、stateをReconciliationRequiredへ進める。`get_session_lifecycle_operation`はbackend operation IDのdirect lookupだけを使い、response喪失とrestart後に保存済みreceipt / state / outcomeをexact replayし、current sessionから再構築しない。`BackendSelected.runtime_started`は次sendまで常にfalseである。closed archiveではqueueを変更せず、`Archived { source_was_open: false, queue_paused }`のqueue_pausedは保存済みresulting projectionを返す。view closeはbackend operationを作らない。

canonical shutdown types と wire DTO を分離する。

    pub struct ShutdownExitIntent {
        pub mode: ShutdownExitMode,
        pub code: i32,
    }

    pub enum ShutdownExitMode {
        Exit,
        Restart,
    }

    pub enum ShutdownTargetSubjectView {
        OpenSession {
            session_id: String,
            activity: OpenSessionShutdownActivity,
        },
        Workflow { workflow_execution_id: String },
    }

    pub enum OpenSessionShutdownActivity {
        Active { turn_id: String },
        Idle,
    }

    pub enum CurrentApplicationShutdownResult {
        Current(Option<ApplicationShutdownProjection>),
        OutcomeUnknown { failure: SafeOperationFailure },
    }

    pub struct RequestApplicationQuitCommand {
        pub request_id: String,
        pub intent: ShutdownExitIntent,
    }

    pub struct ApplicationQuitOperationId(String);

    pub enum ApplicationQuitProjection {
        Shutdown(ApplicationShutdownProjection),
        Bootstrap(BootstrapApplicationQuitProjection),
    }

    pub struct BootstrapApplicationQuitProjection {
        pub bootstrap_id: String,
        pub exit_intent: ShutdownExitIntent,
        pub phase: BootstrapApplicationQuitPhase,
        pub accepted_at: String,
        pub durability_cutoff_at: String,
        pub global_deadline_at: String,
        pub failure: Option<SafeOperationFailure>,
    }

    pub enum BootstrapApplicationQuitPhase {
        Settling,
        Exited,
        ReconciliationRequired,
    }

    pub enum CurrentApplicationQuitOperationResult {
        Current(ApplicationQuitProjection),
        OutcomeUnknown { failure: SafeOperationFailure },
    }

    pub enum ApplicationQuitResult {
        Accepted {
            operation_id: ApplicationQuitOperationId,
            current: ApplicationQuitProjection,
        },
        RejectedBeforeAcceptance {
            failure: SafeOperationFailure,
            blocking_shutdown: Option<ApplicationShutdownProjection>,
        },
        OutcomeUnknown {
            operation_id: ApplicationQuitOperationId,
            intent: ShutdownExitIntent,
        },
    }

    pub enum ApplicationShutdownPhase {
        Preparing,
        Prepared,
        Activated,
        Quiescing,
        Completed,
        Failed,
        Cancelled,
        ReconciliationRequired,
    }

    pub struct ApplicationShutdownProjection {
        pub plan_id: String,
        pub epoch: PersistedEpoch,
        pub exit_intent: ShutdownExitIntent,
        pub phase: ApplicationShutdownPhase,
        pub details: ShutdownDetailsAvailability,
        pub target_count: PersistedCount,
        pub prepared_count: PersistedCount,
        pub effect_reserved_count: PersistedCount,
        pub terminal_count: PersistedCount,
        pub preexisting_recovery_count: PersistedCount,
        pub preexisting_recovery_snapshot: Option<PendingRecoveryInventorySnapshotRef>,
        pub durability_cutoff_at: String,
        pub global_deadline_at: String,
        pub failure: Option<SafeOperationFailure>,
        pub available_actions: Vec<ApplicationShutdownAction>,
    }

    pub enum ApplicationShutdownAction {
        RetryQuit,
    }

    pub struct ShutdownTargetView {
        pub target_key: String,
        pub target_ordinal: PersistedOrdinal,
        pub subject: ShutdownTargetSubjectView,
        pub state: ShutdownTargetPublicState,
        pub target_state_sha256: Sha256Digest,
        pub observation: Option<SafeEffectObservation>,
        pub terminal_result: Option<ObligationResult>,
        pub safe_failure: Option<SafeOperationFailure>,
        pub available_actions: Vec<OperationAction>,
    }

    pub enum ShutdownTargetPublicState {
        Prepared,
        EffectReserved,
        Completed,
        ReconciliationRequired,
        CancelledBeforeActivation,
        Superseded,
    }

    pub struct ShutdownSummary {
        pub plan_id: String,
        pub epoch: PersistedEpoch,
        pub exit_intent: ShutdownExitIntent,
        pub outcome: ShutdownPublicOutcome,
        pub details: ShutdownDetailsAvailability,
        pub target_count: PersistedCount,
        pub completed_count: PersistedCount,
        pub unresolved_count: PersistedCount,
        pub preexisting_recovery_count: PersistedCount,
        pub safe_failure: Option<SafeOperationFailure>,
    }

    pub enum ShutdownDetailsAvailability {
        Available,
        Compacted,
    }

    pub enum ShutdownPublicOutcome {
        AbortedBeforeActivation,
        Completed,
        ExitedWithRecovery,
        ReconciliationRequired,
    }

    pub struct ShutdownPlanPage {
        pub plan_revision: PersistedRevision,
        pub root_sha256: Sha256Digest,
        pub projection: ApplicationShutdownProjection,
        pub summary: Option<ShutdownSummary>,
        pub entries: Vec<ShutdownTargetView>,
        pub next_cursor: Option<String>,
    }

    pub struct GetShutdownPlanRequest {
        pub plan_id: String,
        pub epoch: PersistedEpoch,
        pub cursor: Option<String>,
        pub limit: u16, // 1..=128
    }

    pub enum ApplicationQuitOperationView {
        Accepted {
            operation_id: ApplicationQuitOperationId,
            current: CurrentApplicationQuitOperationResult,
        },
        Terminal {
            operation_id: ApplicationQuitOperationId,
            projection: ApplicationQuitProjection,
        },
        OutcomeUnknown {
            operation_id: ApplicationQuitOperationId,
            intent: ShutdownExitIntent,
        },
    }

    pub struct GetApplicationQuitOperationRequest {
        pub operation_id: ApplicationQuitOperationId,
    }

ShutdownExitIntentV1 と ShutdownTargetSubjectViewV1 は adaptor/protocol 内だけに置く。canonical command、projection、coordinator state、exit permit は V1 DTO に依存しない。

quit request_id は1..=128 bytesの`[A-Za-z0-9._:-]`で、current installation authority 内で exact intent に束縛する。形式不正はInvalidRequestとしてshutdown identity、admission変更、shutdown effectを0件にする。同じkey / intentはsame operation、同じkey / different intentはPayloadConflictとeffect 0件である。別keyでcurrent flightへ来た後続quitはfirst accepted intentへjoinし、mode / codeを変更しないが、そのcaller keyと提示intentから同じbackend operation IDへのexact bindingを保存する。このjoin closureは後続caller bindingと既存operation IDだけを追加し、first flightのintent、plan、T0、deadline、permit、target、effect stateを更新しない。提示intentがfirst intentと異なってもPayloadConflictにせず、同じoperationのfirst intentをresultへ返す。activation前にeffect 0件のabortが確定した後だけnew flightがnew intentを採用できる。`ApplicationQuitOperationId`はcaller keyと別のbackend発行opaque IDで、normal planだけでなくbootstrap-safe flightにも必ず発行する。`ApplicationQuitResult::Accepted.current`とoperation viewのprojectionは`Shutdown | Bootstrap`のclosed sumであり、known operation用`CurrentApplicationQuitOperationResult`はOptionを持たない。`ApplicationQuitOperationView::Terminal`のShutdown branchはterminal historyだけ、Bootstrap branchは`Exited`だけに使う。`RejectedBeforeAcceptance.blocking_shutdown`はfailure kindが`PreviousShutdownReconciliationRequired | PreviousShutdownCompactionPending`の場合だけSomeで、既存plan identity、phase、details、failure、available actionsをそのまま返す。それ以外のrejectionはNoneである。

current shutdown query はnormal shutdown planだけを読み、hash-validなcanonical plan rootをprojection fieldのowner、latest-attempt / latest-activated pointerをselectorと冗長cross-checkとする。同じbounded snapshotでhash-validなcomplete rootがexactly oneあり、plan ID / epoch / first intentを一意にanchorできる場合だけprojectionを構築する。same-bootのrootはそのexact phase、previous-boot nonterminal rootは同じidentityのReconciliationRequired、previous-boot terminal rootは`Current(None)`として返し、terminal historyは`get_shutdown_plan`またはknown quit operation queryから取得する。そのrootとpointerの冗長semantic identityだけが矛盾する場合はroot fieldから`Current(Some(ReconciliationRequired))`と`ShutdownAuthorityMismatch`を返す。verified current root不在だけが`Current(None)`、同じtransactionとplan identityへanchorできる保存結果不明だけがembedded `OutcomeUnknown`である。storage read / decode / envelope・self-hash / pointer-to-root hash failure、required record欠損、state composite / activation lineage integrity failure、複数rootまたはunanchorable authorityでidentityが一意でないcaseはresultを合成せず、Tauriの`GetApplicationShutdownApplicationError::Internal`またはWebSocketの`AgentSessionWsErrorV1::Internal`へ同じcorrelation IDを写す。Internal、OutcomeUnknown、ReconciliationRequiredを相互変換しない。bootstrap-safe quitはnormal planを作らず、`get_application_quit_operation`の`ApplicationQuitProjection::Bootstrap`から読む。

shutdown writer結果不明のpublic mappingは次のclosed表で固定する。

| 不明になった境界 | Public result |
| --- | --- |
| plan identityをanchorする最初のwriter | `CurrentApplicationShutdownResult::OutcomeUnknown`。known quit acceptance writerも同じならtop-level `ApplicationQuitOperationView::OutcomeUnknown` |
| durable plan root確定後のactivation writer | `Current(Some(ReconciliationRequired))`。明示shutdown commandは0件のまま同じplan identityを保持する |
| known quit acceptance確定後のlocator更新writer | `ApplicationQuitOperationView::Accepted { current: CurrentApplicationQuitOperationResult::OutcomeUnknown }` |

`ApplicationShutdownProjection.details`はlive rootから返す間はAvailable、immutable compact archiveから返す時だけCompactedである。nonterminal projectionにCompactedを設定せず、archive-only known quitはTerminalのShutdown projectionをdetails Compactedで返す。projectionと`ShutdownSummary.details`が共に存在する場合は同値を必須とする。

known operation queryはoperation IDのdirect authorityを最初にpoint lookupする。direct recordも、そのoperation IDへ束縛された未解決acceptance writerも存在しない場合だけNotFoundである。direct record / locatorのdecode・envelope・self-hash・generation不一致はInternalとし、binding、transaction inventory、history、normal current shutdown、別operationをidentity authorityへ昇格しない。`ShutdownPlan` locatorはexact plan ID / epochのhash-valid live rootまたはimmutable compact archiveのclosed unionへ解決する。liveだけならそのroot、archiveだけならTerminalかつdetails Compactedのprojection、双方ならsource root revision / hash、exit intent、terminal phase、summary / counts / deadline / failureのsemantic parityを検証した同じprojectionを返す。双方不在、plan / epoch不一致、双方のparity不一致はInternalであり、`Current(None)`へfallbackしない。`BootstrapFlight`はbootstrap ID / operation ID一致を必須にする。acceptance closure writerのcommit結果自体を確認できない場合は`ApplicationQuitOperationView::OutcomeUnknown`、acceptanceはCommittedだがlocator参照先の後続transaction結果だけが不明な場合は`ApplicationQuitOperationView::Accepted { current: CurrentApplicationQuitOperationResult::OutcomeUnknown }`を返す。この二つを相互変換せず、正常なlocatorだけをShutdownまたはBootstrapへtotal mappingする。

`RetryQuit`のavailabilityは一つのbounded authority snapshotから、same `coordinator_boot_id`、activation前`Failed`、shutdown effect reservation / dispatch / observationが全て0件、durable terminal fence確定、global mutation admission `Open`、store health `Healthy`の全predicateをANDして導出する。fresh boot、Preparing / Prepared / Activated / Quiescing / Completed / Cancelled / ReconciliationRequired、activation writerまたはeffect countのOutcomeUnknown、admission非Open、store非Healthyでは提示しない。queryはavailabilityを投影するだけでplan、admission、terminal、effectを変更しない。

pre-activation Failed / Cancelled terminalは`ShutdownSummary { outcome: AbortedBeforeActivation, details: Available, .. }`を返し、ArchiveSwitch前は保持したtarget page / authorityからentryを、保存済みsnapshot refからpending recovery detailを返す。ArchiveSwitchがcommitした時点でquery authorityを`ShutdownPlanCompactArchiveV1`へ一方向に切り替え、residual detailの削除途中でも`details=Compacted`、entries空、next cursorなしを返す。exact target / snapshot detail要求は`DetailsCompacted`であり、Available、empty page、current inventoryへfallbackしない。

#### Tauri and WebSocket

Tauri は次の command / query を同じ AgentSessionCommandUsecase と AgentSessionQueryService に接続する。

- send_agent_message
- get_agent_send_operation
- stop_agent_session
- get_stop_operation
- request_session_lifecycle
- get_session_lifecycle_operation
- list_pending_agent_recovery
- get_pending_recovery_snapshot
- resolve_pending_recovery_action
- resolve_shutdown_target_action
- get_recovery_action
- get_phase0_bootstrap
- get_application_shutdown
- request_application_quit
- get_application_quit_operation
- get_shutdown_plan
- get_session_operation_feedback
- dismiss_session_operation_feedback
- retry_session_operation_feedback_resolution

各Tauri endpointは`Result<canonical result, endpoint固有application error enum>`を返す。次表を一つのdeclarative schemaとしてdistinct enum、canonical supersetからのtotal conversion、OpenAPI / TypeScript bindingを同時生成する。direct error集合はexactであり、result内のRejected / Failed / OutcomeUnknown / ReconciliationRequiredをdirect errorへ複製しない。`StorageUnavailable`は必ずbounded `failure`を持ち、`Internal`はcorrelation IDだけを持つ。

| Endpoint | 公開error型 | exact variant |
| --- | --- | --- |
| send_agent_message | SendAgentMessageApplicationError | InvalidRequest, PayloadConflict(Send), CapacityExceeded, FeedbackCapacityExceeded, BootstrapInProgress, ShutdownInProgress, ResponseTooLarge, Internal |
| get_agent_send_operation | GetAgentSendOperationApplicationError | InvalidRequest, NotFound, QueryBusy, DeadlineExceeded, StorageUnavailable, Internal |
| stop_agent_session | StopAgentSessionApplicationError | InvalidRequest, PayloadConflict(Stop), FeedbackCapacityExceeded, BootstrapInProgress, ShutdownInProgress, Internal |
| get_stop_operation | GetStopOperationApplicationError | InvalidRequest, NotFound, QueryBusy, DeadlineExceeded, StorageUnavailable, Internal |
| request_session_lifecycle | RequestSessionLifecycleApplicationError | InvalidRequest, PayloadConflict(SessionLifecycle), FeedbackCapacityExceeded, BootstrapInProgress, ShutdownInProgress, Internal |
| get_session_lifecycle_operation | GetSessionLifecycleOperationApplicationError | InvalidRequest, NotFound, QueryBusy, DeadlineExceeded, StorageUnavailable, Internal |
| list_pending_agent_recovery | ListPendingAgentRecoveryApplicationError | InvalidRequest, CursorMismatch, CursorExpired, QueryBusy, DeadlineExceeded, ResponseTooLarge, StorageUnavailable, Internal |
| get_pending_recovery_snapshot | GetPendingRecoverySnapshotApplicationError | InvalidRequest, NotFound, SnapshotMismatch, CursorMismatch, CursorExpired, DetailsCompacted, QueryBusy, DeadlineExceeded, ResponseTooLarge, StorageUnavailable, Internal |
| resolve_pending_recovery_action / resolve_shutdown_target_action | ResolveRecoveryActionApplicationError | InvalidRequest, BootstrapInProgress, ShutdownInProgress, StorageUnavailable, Internal |
| get_recovery_action | GetRecoveryActionApplicationError | InvalidRequest, NotFound, QueryBusy, DeadlineExceeded, StorageUnavailable, Internal |
| get_phase0_bootstrap | GetPhase0BootstrapApplicationError | StorageUnavailable, Internal |
| get_application_shutdown | GetApplicationShutdownApplicationError | Internal |
| request_application_quit | RequestApplicationQuitApplicationError | InvalidRequest, PayloadConflict(ApplicationQuit), CapacityExceeded, ResponseTooLarge, Internal |
| get_application_quit_operation | GetApplicationQuitOperationApplicationError | InvalidRequest, NotFound, QueryBusy, DeadlineExceeded, StorageUnavailable, Internal |
| get_shutdown_plan | GetShutdownPlanApplicationError | InvalidRequest, NotFound, CursorMismatch, CursorExpired, QueryBusy, DeadlineExceeded, ResponseTooLarge, StorageUnavailable, Internal |
| get_session_operation_feedback | GetSessionOperationFeedbackApplicationError | InvalidRequest, CursorMismatch, CursorExpired, QueryBusy, DeadlineExceeded, ResponseTooLarge, StorageUnavailable, Internal |
| dismiss_session_operation_feedback / retry_session_operation_feedback_resolution | SessionOperationFeedbackControlApplicationError | InvalidRequest, StorageUnavailable, Internal |

WebSocket は loopback local API の GET /v1/agent-sessions/ws に追加し、既存 Bearer 検証後だけupgradeする。call と result は versioned DTO とする。

    pub enum AgentSessionWsCallV1 {
        SendAgentMessage(SendAgentMessageRequestV1),
        GetOperation(GetAgentSendOperationRequestV1),
        StopSession(StopAgentSessionRequestV1),
        GetStopOperation(GetStopOperationRequestV1),
        ListPendingRecovery(ListPendingRecoveryRequestV1),
        GetPendingRecoverySnapshot(GetPendingRecoverySnapshotRequestV1),
        ResolvePendingRecoveryAction(ResolvePendingRecoveryActionRequestV1),
        ResolveShutdownTargetAction(ResolveShutdownTargetActionRequestV1),
        GetRecoveryAction(GetRecoveryActionRequestV1),
        GetPhase0Bootstrap,
        GetApplicationShutdown,
        RequestApplicationQuit(RequestApplicationQuitRequestV1),
        GetApplicationQuitOperation(GetApplicationQuitOperationRequestV1),
        GetShutdownPlan(GetShutdownPlanRequestV1),
        GetOperationFeedback(GetOperationFeedbackRequestV1),
        DismissOperationFeedback(DismissOperationFeedbackRequestV1),
        RetryOperationFeedbackResolution(RetryOperationFeedbackResolutionRequestV1),
    }

    pub enum AgentSessionWsResultV1 {
        SendAgentMessage(SendAgentMessageResultV1),
        SendOperation(AgentSendOperationViewV1),
        Stop(StopResultV1),
        StopOperation(StopOperationViewV1),
        PendingRecovery(PendingRecoveryPageV1),
        PendingRecoverySnapshot(PendingRecoverySnapshotPageV1),
        RecoveryAction(RecoveryActionCommandResultV1),
        RecoveryActionOperation(RecoveryActionOperationViewV1),
        Phase0Bootstrap(Option<Phase0BootstrapProjectionV1>),
        ApplicationShutdown(CurrentApplicationShutdownResultV1),
        ApplicationQuit(ApplicationQuitResultV1),
        ApplicationQuitOperation(ApplicationQuitOperationViewV1),
        ShutdownPlan(ShutdownPlanPageV1),
        OperationFeedback(SessionOperationFeedbackSnapshotV1),
        OperationFeedbackControl(SessionOperationFeedbackControlResultV1),
    }

protocol DTOとcanonical typeのtotal mappingは次で固定する。各V1 DTOは右辺の全field / closed variantを一度だけ持ち、表にない暗黙fieldを追加しない。Tauri専用 lifecycle DTOも同じcodec規則を使う。

| Boundary DTO | Canonical request / result |
| --- | --- |
| SendAgentMessageRequestV1 / SendAgentMessageResultV1 | SendAgentMessageCommand / SendAgentMessageResult |
| GetAgentSendOperationRequestV1 / AgentSendOperationViewV1 | GetAgentSendOperationRequest / AgentSendOperationView |
| StopAgentSessionRequestV1 / StopResultV1 | StopAgentSessionCommand / StopResult |
| GetStopOperationRequestV1 / StopOperationViewV1 | GetStopOperationRequest / StopOperationView |
| RequestSessionLifecycleRequestV1 / SessionLifecycleResultV1 | RequestSessionLifecycleCommand / SessionLifecycleResult（Tauri only） |
| GetSessionLifecycleOperationRequestV1 / SessionLifecycleOperationViewV1 | GetSessionLifecycleOperationRequest / SessionLifecycleOperationView（Tauri only） |
| ListPendingRecoveryRequestV1 / PendingRecoveryPageV1 | ListPendingRecoveryRequest / PendingRecoveryPage |
| GetPendingRecoverySnapshotRequestV1 / PendingRecoverySnapshotPageV1 | GetPendingRecoverySnapshotRequest / PendingRecoverySnapshotPage |
| ResolvePendingRecoveryActionRequestV1 | ResolvePendingRecoveryActionRequest |
| ResolveShutdownTargetActionRequestV1 | ResolveShutdownTargetActionRequest |
| RecoveryActionCommandResultV1 | RecoveryActionCommandResult |
| GetRecoveryActionRequestV1 / RecoveryActionOperationViewV1 | GetRecoveryActionRequest / RecoveryActionOperationView |
| Phase0BootstrapProjectionV1 | Phase0BootstrapProjection |
| CurrentApplicationShutdownResultV1 | CurrentApplicationShutdownResult |
| RequestApplicationQuitRequestV1 / ApplicationQuitResultV1 | RequestApplicationQuitCommand / ApplicationQuitResult |
| GetApplicationQuitOperationRequestV1 / ApplicationQuitOperationViewV1 | GetApplicationQuitOperationRequest / ApplicationQuitOperationView |
| GetShutdownPlanRequestV1 / ShutdownPlanPageV1 | GetShutdownPlanRequest / ShutdownPlanPage |
| GetOperationFeedbackRequestV1 / SessionOperationFeedbackSnapshotV1 | GetSessionOperationFeedbackRequest / SessionOperationFeedbackSnapshot |
| DismissOperationFeedbackRequestV1 | DismissSessionOperationFeedbackCommand |
| RetryOperationFeedbackResolutionRequestV1 | RetrySessionOperationFeedbackResolutionCommand |
| SessionOperationFeedbackControlResultV1 | SessionOperationFeedbackControlResult |

公開二段階send用のcommand / owner list / content queryと、resource / privacy / backup / reset routeは存在しない。

domain / usecase type に serde を付けず、V1 DTO は adaptor/protocol でfield-for-field mappingする。上表のrootから再帰的に到達する全canonical struct / enum / aliasがV1 schema closureに含まれ、未宣言nested type、`serde_json::Value`、flatten、untagged enum、transport固有defaultを認めない。V1 schemaは次のmechanical ruleで完全に決まる。

- structはsnake_case JSON objectでcanonical fieldを宣言順にexactly once持つ。missing、duplicate、unknown fieldを拒否し、`Option<T>`もfieldを省略せずNoneをnull、SomeをTで表す。
- enumはadjacently tagged object `{ "type": "<snake_case variant>", ...variant fields }`である。unit variantはtypeだけ、newtype / tuple variantは`value` field、struct variantはcanonical field名を使い、unknown tag / extra fieldを拒否する。
- persisted semantic `u64` aliasはcanonical decimal string、`u16 / u32`のbounded controlとsource lineはJSON nonnegative integer、`i32` exit codeはJSON signed integer、bool / UTF-8 stringは同型である。
- `Sha256Digest / HmacSha256Digest / [u8; 32]`はexactly 64文字のlowercase hex、一般binary bytesはRFC 4648 standard alphabetのpadding付きbase64、vectorはcanonical orderのJSON arrayである。map / object keyによる集合表現は禁止する。
- canonical newtypeはunderlying scalarとしてencodeする。fieldごとのlength、one-based / zero-based、closed pair、page / byte boundはdecode後にdomain commandへ渡す前に検証する。

`SendAgentMessageResultV1` と `AgentSendOperationViewV1` の closed variant はcanonical typeと同じであり、internal staging、request binding、provider payload、filesystem pathを追加しない。schema snapshotは上表のrootと再帰closureに含まれるtype名、field順、variant tag、scalar codecを固定し、canonical type変更時に未更新V1 schemaがcompile / contract testで失敗する。TauriとWebSocketは同じgenerated schema fixtureとgolden bytesを使う。

AgentSessionWsErrorV1 は全WebSocket endpointが共用するtransport envelopeであり、個別query error型のaliasではない。authentication後のprotocol validation、outer request ID conflict、connection / rate / response bounds、read-only query の NotFound / cursor / snapshot / bounded failureに加え、deterministic pre-commit identity conflictだけを表す。後者はPayloadConflictのclosed identityでWebSocket routeを持つsend operation ID、Stop request ID、quit request IDだけを区別する。endpoint dispatchは上表と同じvariant集合からWebSocket未公開のSessionLifecycleを除いたallowlistを適用する。Tauriは各command固有application errorの対応fieldを返す。OutcomeUnknown、Accepted後failure、ActionOutcomeUnknownは対応するcommand result / projectionに置き、errorへ複製しない。domain / usecaseの`SafeOperationFailure`をwire enumへ直接埋め込まず、次のexact V1 DTOへfield-for-field変換する。

    pub enum SessionOperationFailureKindV1 {
        StorageUnavailable,
        StorageCorrupt,
        MigrationBlocked,
        PersistFailure,
        ProtocolIncompatible,
        ProviderUnavailable,
        ExternalEffectFailed,
        OutcomeUnknown,
        DeadlineExceeded,
        CapacityExceeded,
        StopCapacityExceeded,
        ShutdownAuthorityMismatch,
        TargetRevisionChanged,
        OwnerRevisionChanged,
        RuntimeGenerationChanged,
        InvalidEffectIntent,
        PreviousShutdownReconciliationRequired,
        PreviousShutdownCompactionPending,
        Internal,
    }

    pub struct BoundedNoticeTextV1 {
        pub value: String,
        pub truncated: bool,
        pub original_bytes: Option<String>,
        pub digest: Option<String>,
        pub correlation_id: Option<String>,
    }

    pub struct SafeOperationFailureV1 {
        pub kind: SessionOperationFailureKindV1,
        pub retryable: bool,
        pub label: BoundedNoticeTextV1,
        pub detail: Option<BoundedNoticeTextV1>,
        pub correlation_id: String,
    }

    pub enum AgentSessionWsErrorV1 {
        InvalidRequest,
        RequestIdConflict,
        PayloadConflict { identity: PayloadConflictIdentityV1 },
        NotFound,
        CursorMismatch,
        CursorExpired,
        SnapshotMismatch,
        DetailsCompacted,
        QueryBusy,
        DeadlineExceeded,
        CapacityExceeded,
        FeedbackCapacityExceeded,
        BootstrapInProgress,
        ShutdownInProgress,
        RateLimited,
        ResponseTooLarge,
        StorageUnavailable { failure: SafeOperationFailureV1 },
        Internal { correlation_id: String },
    }

`SessionOperationFailureKindV1`はcanonical enumのexact 19 tagである。`BoundedNoticeTextV1.original_bytes`はNoneまたは先頭ゼロなしcanonical decimal string、SafeOperationFailure内のlabel / detailはnested `correlation_id=None`、failure identityはtop-level `correlation_id`一件だけとする。unknown tag / field型、noncanonical integer、bounds超過、correlation identity二重化をdecode時に拒否する。

    pub enum PayloadConflictIdentityV1 {
        Send { operation_id: String },
        Stop { request_id: String },
        ApplicationQuit { request_id: String },
    }

    pub enum SessionLifecyclePayloadConflictIdentityV1 {
        SessionLifecycle { request_id: String },
    }

上限は1 process 16 connections、1 connection 32 in-flight、60 requests/s・burst 120、outer request ID 1..=128 bytesのA-Z a-z 0-9 . _ : -、request / response 16 MiB、outbound 32 responses / 16 MiBである。同じconnectionの同じouter request IDが進行中なら一方だけを受理し、他方はRequestIdConflictとする。BearerなしはHTTP 401、17本目はHTTP 503 CapacityExceeded、request frame超過はclose code 1009、outbound待ち超過はclose code 1013とする。in-flight、rate、responseの超過はconnectionを維持してCapacityExceeded、RateLimited、ResponseTooLargeを返す。

#### Numeric encoding

persisted semantic integer のdomainは0..=9223372036854775807である。count、index、ordinal、offset、Absent expected revisionは0を許し、epoch、existing revision、sequenceなど1始まりfieldは0を拒否する。

Tauri / WebSocket の semantic integer は先頭ゼロのないASCII decimal stringでencodeする。JSON number、負数、正符号、指数、空白、範囲超過、1始まりfieldの0はInvalidRequestである。bounded transport controlのpage limit / byte limitはJSON nonnegative integer、exit codeはJSON signed integerのままにする。最大値から次値が必要なmutationはCapacityExceeded、effect 0件とする。

### Data Model

#### Phase 0 authority

    pub struct AppDataGenerationId(String);

    pub struct AgentOperationBindingKeyV1 {
        pub schema_version: u16, // 1
        pub app_data_generation_id: AppDataGenerationId,
        pub hmac_sha256_key: [u8; 32],
        pub created_at_utc: String,
    }

    pub struct Phase0AuthorityPointerV1 {
        pub revision: PersistedRevision,
        pub authority: Phase0AuthorityV1,
    }

    pub enum Phase0AuthorityV1 {
        Legacy {
            app_data_generation_id: AppDataGenerationId,
        },
        Phase0 {
            app_data_generation_id: AppDataGenerationId,
            agent_operation_binding_key_record_sha256: Sha256Digest,
            transaction_inventory_revision: PersistedRevision,
            transaction_inventory_root_sha256: Sha256Digest,
            activated_bootstrap_id: String,
            parity_manifest_sha256: Sha256Digest,
        },
    }

authority pointer、transaction inventory root、manifest、direct record はversioned envelopeで保存する。closure manifestはD3正本の`Phase0ClosureManifestV1 / Phase0ParticipantMutationV1`をそのまま使い、本書で別shapeを再定義しない。unknown additive fieldはraw bytesを保持し、unknown required versionまたはhash mismatchはStorageCorrupt / MigrationBlockedとしてfail closedにする。

#### Send operation

    pub struct SendOperationRecordV1 {
        pub principal_id: String,
        pub operation_id: SendOperationId,
        pub app_data_generation_id: AppDataGenerationId,
        pub exact_request_binding_hmac_sha256: HmacSha256Digest,
        pub receipt: SendAcceptanceReceipt,
        pub status: SendExecutionStatus,
        pub obligation_ids: Vec<String>,
        pub revision: PersistedRevision,
    }

exact_request_binding_hmac_sha256 はapp-data generationごとexactly oneの`AgentOperationBindingKeyV1.hmac_sha256_key`を使い、`LP("send-operation-exact-request-binding/v1") || LP(principal_id) || LP(app_data_generation_id) || LP(operation_id) || LP(canonical_exact_command_bytes)`をHMAC-SHA256した値である。このkey authorityはsend / Stop / quit / session lifecycleの4 domainが別domain prefixで共用する。missing / duplicate / generation mismatch / owner ACL failure時は4 commandのadmissionを閉じ、default keyを再生成しない。`LP`はu32 BE lengthとraw bytesである。canonical_exact_command_bytesはAgentSendTargetのchat_session_id、worktree_path、permission_mode、plan_mode、backend_id、model_idをfield順に展開し、content、image bytes / media type、mentions、editor context、active-turn policyを順序込みで続ける。principal、operation_id、generationはcanonical_exact_command_bytesの内側には入れず、HMAC envelopeとrecord authorityで束縛する。WebSocket outer request ID、server生成session / turn / queue ID、時刻、受理後session state、actual dispositionはbindingへ含めない。

send binding codecの固定KATは、key bytes `00..1f`、principal `principal_1`、generation `app_1`、operation `op_1`、canonical exact command bytes `01020304`を入力し、canonical preimage 83 bytes、HMAC-SHA256 `74ad9247b5f271fc4e31f4fddf7c45cf35d413b1b35202d532095b163f9545db`である。Phase 0、F3、import、backup、restore、Tauri / WebSocket parity testはpreimage bytesとdigestの両方を固定し、principalだけを変えたvectorが一致しないことも検査する。

    pub struct RustOwnedSendCallerAttemptV1 {
        pub principal_id: String,
        pub operation_id: SendOperationId,
        pub canonical_exact_command_bytes: Vec<u8>,
        pub state: RustOwnedSendCallerAttemptState,
        pub revision: PersistedRevision,
    }

    pub enum RustOwnedSendCallerAttemptState {
        DispatchPending,
        AwaitingAuthoritativeResult,
    }

caller-attempt journalはowner-only、crash-atomic、boundedであり、controller到達後の最初のRust stepでcanonical commandのsize上限と同じpermitを取得し、normal send usecaseのdispatch前にcommitする。journal permitまたはcommit failureは`RejectedBeforeCommit`へ閉じる。Accepted recordの代替authorityにはせず、send queryへ公開せず、journalだけを根拠にprovider effectを開始しない。startup workerはjournalのsame operation / exact commandをnormal send usecaseへ再提示してauthoritative resultを解決する。

`AgentOperationBindingKeyV1`とbindingはpublic DTO、receipt、log、telemetryへ出さず、bindingをconstant-time比較する。`AgentOperationBindingKeyV1`はcurrent app-data generationのauthority lifetime全体でexactly oneを保持し、各binding recordは対応operation / caller decisionのretention期間中保持する。plain content hashを保存しない。D3のlegacy-named physical columnへ写す場合も意味はこのopaque exact bindingであり、public semantic identityではない。

commit前staging recordはpublic AgentSendOperationViewではない。rollbackまたはcleanupが確認済みならqueryはNotFound、writer outcome未解決ならOutcomeUnknown、acceptance closureがrootから到達すればAcceptedだけを返す。

#### Terminal and Stop

    pub struct TerminalRecordV1 {
        pub session_id: String,
        pub turn_id: String,
        pub terminal_result: TurnResult,
        pub final_parts_ref: String,
        pub assistant_message_id: Option<String>,
        pub session_revision: PersistedRevision,
        pub permission_settlement: Option<PermissionSettlement>,
        pub queue_state: QueueTerminalState,
        pub stop_resolutions: Vec<StopResolutionRecordV1>,
    }

    pub struct StopResolutionRecordV1 {
        pub operation_id: StopOperationId,
        pub resolution: StopResolutionResult,
    }

    pub struct StopCallerRequestBindingRecordV1 {
        pub principal_id: String,
        pub app_data_generation_id: AppDataGenerationId,
        pub request_id: String,
        pub operation_id: StopOperationId,
        pub exact_request_binding_hmac_sha256: [u8; 32],
        pub revision: u64,
    }

    pub enum StopOperationStateV1 {
        Accepted,
        ReconciliationRequired { failure: SafeOperationFailure },
        Terminal {
            resolution: StopResolutionResult,
            result: TurnResult,
        },
    }

    pub struct StopOperationRecordV1 {
        pub operation_id: StopOperationId,
        pub app_data_generation_id: AppDataGenerationId,
        pub terminal_commit_obligation_id: String,
        pub receipt: StopAcceptanceReceipt,
        pub accepted_expected_session_revision: u64,
        pub deadline_permit: Option<StopDeadlinePermitRef>,
        pub state: StopOperationStateV1,
        pub revision: u64,
    }

terminal key は session_id と turn_id の組で一意である。Stop、watchdog、close、Fatal、provider completionのcandidateは同じterminal row / direct recordのexpected revisionを競合し、winnerだけがterminal closureをcommitする。

caller binding indexのkeyは`(current-installation principal, app_data_generation_id, Stop, request_id)`で、Stop acceptanceと同じclosureへprincipalをcanonical record内にも保持する`StopCallerRequestBindingRecordV1`とbackend `StopOperationId`正本の`StopOperationRecordV1`を入れる。bindingは`HMAC-SHA256(AgentOperationBindingKeyV1.hmac_sha256_key, LP("stop-operation-exact-request-binding/v1") || LP(principal_id) || LP(app_data_generation_id) || LP(request_id) || LP(operation_id) || LP(canonical_stop_command_bytes))`、`canonical_stop_command_bytes = LP(session_id) || LP(target_turn_id) || U64BE(expected_session_revision)`である。same caller keyはprincipalとこの3 field全てを比較し、一fieldでも異なればPayloadConflictとする。別caller key / same unresolved session・turnのjoinは同じbackend operation IDを写す新binding recordを作り、初回`accepted_expected_session_revision`を変更しない。

Stop binding codecの固定KATは、key bytes `00..1f`、principal `principal_1`、generation `app_1`、request `stop_req_1`、backend operation `stop_op_1`、session `session_1`、turn `turn_1`、expected revision `1`を入力し、canonical preimage 129 bytes、HMAC-SHA256 `9aea744029168a755e77bf7fa763f84df36b2167f7b1bc7fc727e75a26590d3c`である。codec unit testはpreimage bytesとdigestの両方を固定する。

`StopOperationRecordV1`はbackend `StopOperationId`を正本keyとし、同IDをStop専用TerminalCommit obligation IDへ一対一にする。Accepted / ReconciliationRequiredでは`deadline_permit=Some`、TerminalではNoneを必須とし、Terminal stateはStopResolutionResultと同じTurnResultを持つ。完全なterminal closureまでpermitを保持し、Succeeded / Supersededと同じcommitで解放する。OutcomeUnknownは保存stateへ追加せず、未解決caller-slot / operation transactionからqueryで導出する。

#### Session lifecycle operation

    pub struct SessionLifecycleCallerRequestBindingRecordV1 {
        pub principal_id: String,
        pub app_data_generation_id: AppDataGenerationId,
        pub request_id: String,
        pub operation_id: SessionLifecycleOperationId,
        pub exact_request_binding_hmac_sha256: HmacSha256Digest,
        pub revision: PersistedRevision,
    }

    pub struct SessionLifecycleOperationRecordV1 {
        pub operation_id: SessionLifecycleOperationId,
        pub app_data_generation_id: AppDataGenerationId,
        pub obligation_id: String,
        pub receipt: SessionLifecycleAcceptanceReceipt,
        pub deadline_at: String,
        pub state: SessionLifecycleOperationState,
        pub revision: PersistedRevision,
    }

caller binding keyは`(principal, app_data_generation_id, SessionLifecycle, request_id)`である。bindingは`HMAC-SHA256(key, LP("session-lifecycle-exact-request-binding/v1") || LP(principal_id) || LP(app_data_generation_id) || LP(request_id) || LP(operation_id) || LP(canonical_lifecycle_command_bytes))`、inner bytesは`LP(session_id) || U64BE(expected_session_revision) || LP("close" | "archive-open" | "archive-closed" | "switch-backend") || LP("none" | "some") || [Someの場合だけLP(backend_id)]`である。principal、generation、request ID、operation IDをinner bytesへ重複させない。

固定KATはkey bytes `00..1f`、principal `principal_1`、generation `app_1`、request `lifecycle_req_1`、operation `lifecycle_op_1`、session `session_1`、expected revision `1`、action `close`、backend option `none`を入力し、inner command 38 bytes、full preimage 149 bytes、HMAC-SHA256 `b623c791f1a3f40579ba9713507ab507bdc844dee12d95e4408d673b17eb2217`である。

physical F3 mappingではbackend operation IDとkind 7 `SessionClose`のobligation IDをbyte同一にし、caller binding tableのoperation kind 3からpending obligationまたはterminal resultへdirect解決する。`SessionLifecycleOperationRecordV1.obligation_id`も同じbytesでなければdecode / importを拒否する。binding、pending / result、owner lifecycle、activeならterminal、queue pauseをacceptance / completion closureで閉じるため、別のoperation tableを追加しない。ArchiveClosedのeffectは0件でも同じterminal result recordを作り、query identityを保持する。bindingはresult retention中削除せず、Completed queryはcurrent session projectionから再構築しない。

#### Obligations and recovery

    pub enum ObligationKind {
        TurnExecution,
        QueueExecution,
        PermissionDelivery,
        ProviderEstablish,
        TerminalCommit,
        BackendRecovery,
        SessionClose,
        WorkflowShutdown,
        RecoveryPublication,
    }

永続obligationは正本語彙の`ObligationStateRecordV1 = Pending(PendingObligationRecordV1) | Terminal(ObligationResultRecordV1)`を使う。pending lifecycleは`Prepared | Pending | EffectReserved | ReconciliationRequired | Failed`、external effect capabilityは`EffectResolutionCapability`であり、Terminalをpending stateへ混ぜたり、未定義の`ObligationRecordV1 / EffectCapability`を別schemaとして作らない。

pending-only indexはobligation ID順のbounded inventoryである。owner filter、three partition、shutdown associationをsecondary keyに持たせ、session event履歴やdirectory scanなしでfirst pageを返す。page snapshotはroot revisionとfilterへ束縛し、cursorはsnapshot、filter、last key、expiryをMACする。

`RecoveryActionAttempt`はaction ID、bound obligation / shutdown target、origin revision、kind、effect identity、state、保存済みreceipt hashを持つ。Completed stateはclosed `outcome`、`completed_result_schema_version`、1..=64 KiBの`serialized_safe_result`、そのSHA-256を必須とし、bytes内にaction ID、closed outcome / classification、resource revision、safe resource viewをfixed-orderで持つ。他stateではこの4 fieldを持たない。physical file codecはD3の`Phase0RecoveryActionAttemptV1`へ明示変換する。Completedはimmutableかつshutdown detail retentionから独立して保持し、nonterminalだけをsame attemptへjoinする。resource resultだけを先にterminal化せず、kind-specific owner closureとattempt completionを同じcommitへ入れる。

#### Feedback

FeedbackStore はsession IDごとのordered entryとglobal unresolved countを一つのprocess stateとして持つ。SessionMetaをscope authorityに使わない。feedbackはdurable Noticeではなく、application restartを跨ぐ一般的Notice authorityは#1393に残す。

FeedbackStoreはprocess-memoryのbounded command feedbackであり、Phase 0 closure、bootstrap、F3 importのparticipantに含めずrestart保持を保証しない。entry identity、revision、`failure: SafeOperationFailure`、actionsだけを保持する。`failure.correlation_id`を同じfailureのfeedback表示とlogを結ぶ一意なidentityとし、kind / retryable / label / detailをfeedback直下へ複製しない。別sessionまたは別operationの成功で消すglobal clear APIは設けない。

#### Shutdown

    pub struct ApplicationQuitCallerRequestBindingRecordV1 {
        pub principal_id: String,
        pub app_data_generation_id: AppDataGenerationId,
        pub request_id: String,
        pub operation_id: ApplicationQuitOperationId,
        pub exact_request_binding_hmac_sha256: [u8; 32],
        pub revision: u64,
    }

    pub enum ApplicationQuitOperationLocatorV1 {
        ShutdownPlan {
            plan_id: String,
            epoch: u64,
        },
        BootstrapFlight {
            bootstrap_id: String,
        },
    }

    pub struct ApplicationQuitOperationDirectRecordV1 {
        pub operation_id: ApplicationQuitOperationId,
        pub app_data_generation_id: AppDataGenerationId,
        pub locator: ApplicationQuitOperationLocatorV1,
        pub revision: u64,
    }

    pub enum BootstrapApplicationQuitFlightStateV1 {
        Settling,
        Exited {
            observed_boot_id: String,
            observed_at: String,
        },
        ReconciliationRequired {
            failure: SafeOperationFailure,
        },
    }

    pub struct BootstrapApplicationQuitFlightRecordV1 {
        pub operation_id: ApplicationQuitOperationId,
        pub app_data_generation_id: AppDataGenerationId,
        pub bootstrap_id: String,
        pub coordinator_boot_id: String,
        pub exit_intent: ShutdownExitIntent,
        pub accepted_at: String,
        pub durability_cutoff_at: String,
        pub global_deadline_at: String,
        pub state: BootstrapApplicationQuitFlightStateV1,
        pub revision: u64,
    }

physical shutdown schemaはD3正本の`Phase0ShutdownPlanRootV1`、`Phase0ShutdownPreparedPageV1`、`Phase0ShutdownTargetAuthorityV1`、`LatestShutdownAttemptRefV1`、`LatestActivatedShutdownPlanRefV1`、`LatestRetiringShutdownPlanRefV1`、`Phase0ShutdownDetailsV1`、`Phase0ShutdownCompactionStateV1`、`Phase0ShutdownDetailDetachBatchV1`、`Phase0ShutdownPageDetachBatchV1`、`ShutdownPlanCompactArchiveV1`を使う。canonical query modelはInterface節の`ApplicationShutdownProjection / ShutdownTargetView / ObligationResult`であり、未定義の`ShutdownPlanRecordV1 / ShutdownTargetRecordV1 / ShutdownTargetState / ShutdownTargetResult`を別schemaとして作らない。

    pub struct ShutdownPlanCompactProjectionV1 {
        pub phase: ApplicationShutdownPhase, // Completed | Failed | Cancelled only
        pub prepared_count: PersistedCount,
        pub effect_reserved_count: PersistedCount,
        pub terminal_count: PersistedCount,
        pub durability_cutoff_at: String,
        pub global_deadline_at: String,
    }

    pub struct ShutdownPlanCompactArchiveV1 {
        pub summary: ShutdownSummary, // details is always Compacted
        pub compact_projection: ShutdownPlanCompactProjectionV1,
        pub source_root_sha256: Sha256Digest,
        pub source_root_revision: PersistedRevision,
        pub activation_ancestor_sha256: Option<Sha256Digest>,
        pub pages_sha256: Sha256Digest,
        pub preexisting_snapshot_sha256: Option<Sha256Digest>,
        pub archived_at: String,
    }

`Phase0ShutdownTargetAuthorityV1`は`shutdown-target-authority/v1(plan_id, epoch, target_key)`の決定pathで単独所有する。`Phase0ShutdownPreparedPageV1`はordered target refと`target_authority_sha256`だけを持ち、build / query / dispatchはtargetごとにauthorityを1件point lookupしてdigestとplan / epoch / target key / ordinal / obligation / expected owner revisionを検証する。prepared pageを唯一のauthority列挙索引とし、authority directory scanやresource indexへfallbackしない。Phase 0 resource inventoryは作らない。

quit caller binding indexのkeyは`(current-installation principal, app_data_generation_id, ApplicationQuit, request_id)`であり、principalをcanonical binding recordにも保持する。bindingは`HMAC-SHA256(AgentOperationBindingKeyV1.hmac_sha256_key, LP("application-quit-exact-request-binding/v1") || LP(principal_id) || LP(app_data_generation_id) || LP(request_id) || LP(operation_id) || LP(canonical_quit_command_bytes))`、`canonical_quit_command_bytes = LP("exit" | "restart") || I32BE(exit_code)`とする。同じcaller keyのmodeまたはexit codeが異なればPayloadConflictであり、別caller keyが進行中flightへjoinするときは、提示されたexact intentと同じbackend operation IDへの新しいbindingをfirst intentを変更せず保存する。初回acceptanceはbinding、direct record、normal planまたはbootstrap flight recordを同一transactionで到達可能にする。distinct caller joinは`application-quit-caller-join/v1`のbinding-only closureを使い、existing operation / locator / first intent / flightをread guardにしてnew binding以外を変更しない。Stopのdistinct caller joinも`stop-caller-join/v1`で同じ原則を使い、existing Stop operation / target / first accepted revision / terminal obligationを変更しない。

quit binding codecの固定KATは、key bytes `00..1f`、principal `principal_1`、generation `app_1`、request `quit_req_1`、backend operation `quit_op_1`、mode `exit`、exit code `0`を入力し、canonical preimage 112 bytes、HMAC-SHA256 `6a34bd12ce2691c1912e31d4e0f797cd51e28a67fdf5dc03714f18782e49dfda`である。codec unit testはpreimage bytesとdigestの両方を固定する。

`ApplicationQuitOperationId`はcaller request IDから導出しないbackend発行opaque IDである。`ApplicationQuitOperationDirectRecordV1`をこのIDのdirect lookup正本とし、normal admissionでは`ShutdownPlan { plan_id, epoch }`、bootstrap中では`BootstrapFlight { bootstrap_id }`へ必ず一意に解決する。normal branchはhash-valid live rootまたはimmutable compact archiveのclosed unionから`ApplicationQuitProjection::Shutdown`を構築し、双方存在時はsemantic parityを必須にし、archive-onlyでもTerminal / Compactedを返す。bootstrap branchは`BootstrapApplicationQuitFlightRecordV1`から`ApplicationQuitProjection::Bootstrap`を構築する。known operation queryはlocator不在またはlocator先union双方不在を`Current(None)`へ落とさずInternalとし、保存transaction outcome未解決だけを`OutcomeUnknown`とする。

一planは最大4096 target、1 pageは最大128 targetかつencoded 1 MiBである。target pageはimmutable、phaseとresultはplan root / target result closureで更新する。open active / Idle sessionとrunning workflowだけをnew targetへ含め、related runtime / childはowner targetへ従属させる。closed / archived / unowned recoveryはpreexisting snapshotへ固定する。

latest-attempt pointer、latest-activated pointer、plan rootを分離して持ち、same-boot / previous-bootのnonterminalをCurrent(None)へ落とさない。全target terminal時の最後のtarget resultとplan Completedは同じclosureで確定する。

`LatestRetiringShutdownPlanRefV1`は最大1 planだけを指す。new root-initはeligible terminal prior latestがdetails Availableかつretiring Noneならprior plan ID / epoch / source root / revisionをretiringへsetするCASとnew Preparing root / latest-attempt advanceを同じclosureへ置く。retiring Someなら`PreviousShutdownCompactionPending`でnew durable object / effectを0件、prior details Compactedならarchive / compact root / nested summary / activation ancestor parityを検証してretiringを変更しない。latest planとretiring planを合わせたdetail set上限は2である。

`ShutdownPlanCompactArchiveV1`はCompacted `ShutdownSummary`、closed `ShutdownPlanCompactProjectionV1`、source root hash / revision、`activation_ancestor_sha256`、ordered page-set hash、original preexisting snapshot hash、archive timeを保持する。compact projectionはterminal phase `Completed | Failed | Cancelled`、prepared / effect-reserved / terminal counts、durability cutoff、global deadlineだけを持つ。ArchiveSwitchはAvailable source rootからsummaryのdetailsだけをCompactedへ変換し、identity、exit intent、final state、counts、outcome、safe failure、activation ancestor、cutoff / deadlineをexactに保存する。archive-only `ShutdownPlanPage`はplan revision / root hashをsource pairへ固定し、projectionのplan / epoch / intent / target・preexisting count / failureをsummary、残りをcompact projectionから構成し、snapshot None、actions空、entries空、next cursorなしにする。compact shellの存否でpage identityを変えず、summaryからphase・三count・deadlineを推測しない。pre-activation Failed / Cancelled terminal closureはdetails Available、full refs / snapshotを保持し、archive insert、detach、GCを行わない。

#### Bootstrap

    pub struct LegacyBootstrapCursorV1 {
        pub source_entry_ordinal: u64,
        pub source_entry_id: String,
        pub source_record_ordinal: u64,
        pub substep_ordinal: u64,
    }

    pub struct Phase0BootstrapProjection {
        pub bootstrap_id: String,
        pub phase: Phase0BootstrapPublicPhase,
        pub imported_source_count: PersistedCount,
        pub total_source_count: Option<PersistedCount>,
        pub read_only: bool,
        pub safe_failure: Option<SafeOperationFailure>,
    }

    pub enum Phase0BootstrapPublicPhase {
        InspectingSource,
        Importing,
        Verifying,
        Activating,
        Failed,
    }

bootstrap中は全phaseでread_only=trueとし、new mutation / provider effectをBootstrapInProgressで拒否する。legacy dataはread-only表示を維持し、source bytesを自動書換えしない。bootstrap queryはmutation admissionが閉じていても利用できる。

`LegacyBootstrapCursorV1`のcanonical bytesは`LP("legacy-bootstrap-cursor/v1") || U64BE(source_entry_ordinal) || LP(source_entry_id) || U64BE(source_record_ordinal) || U64BE(substep_ordinal)`である。`source_entry_id`はraw UTF-8の1..=1024 bytes、3 ordinalは0..=9223372036854775807とし、entry ordinal / IDは固定済みsource inventoryの同じentryに一致しなければMigrationBlockedとする。cursorは最後に処理した位置ではなく次にcommitすべき最小work unitを指し、substepは一logical record内のraw-copy / normalize / emit段階を再開する。`None`はsource inventoryが空、または全entry・record・substepのfinalized closureが到達可能な場合だけ許す。

### Database

#### Phase 0 file store

Phase 0 は現行 app-data 配下にversioned reliability rootを追加する。論理配置は次のとおりで、具体path、envelope、page / participant limit、sync順はD3正本を参照する。

| Store | Key | Purpose |
| --- | --- | --- |
| authority pointer | singleton | Legacy / Phase0のsingle authority |
| transaction inventory | root revision | committed closure reachability |
| caller request binding index | current-installation principal + generation + operation kind + caller request ID | Stop / quit / session lifecycle exact request replay / conflictとbackend operation ID解決 |
| operation direct index | generation + operation kind + operation ID | send exact request replay / conflict、send / Stop / quit decision lookup。session lifecycleはoperation IDとbyte同一のkind 7 obligation directへ解決する。principalはkey componentではなくauthorization scopeとして検証する |
| terminal direct index | session ID + turn ID | terminal uniqueness |
| obligation state / pending index | obligation ID / ordered key | startup recovery |
| shutdown latest pointers / pages | plan ID、epoch、page | current / history / target lookup |
| bootstrap checkpoint | bootstrap ID | crash-resumable source cursor |

large payloadはmanifestからcontent-addressではないopaque private refで参照し、public resultには入れない。permission exact payloadはowner access policyとsizeを検証できるprivate recordへ置き、redacted summaryから復元しない。

#### Commit and replay

single blocking writerがclosureを次の順で処理する。

1. expected root revision、participant revision、payload boundsを検証する。
2. participant filesとmanifestをstagingへwriteし、required file / directory syncを行う。
3. transaction inventoryのcopy-on-write rootをexpected revision CASで切り替え、root fileとparentをsyncする。
4. fresh readbackでroot reachabilityを確認してCommittedを返す。
5. legacy projection / direct recordをmaterializeし、完了後にcompact receiptへ置換する。

step 3前のrollback確認済みfailureはBeforeCommit、step 3到達確認はCommitted、writer継続・receiver drop・sync結果不明はOutcomeUnknownである。OutcomeUnknown時は同transaction handleへjoinし、fresh readbackまたはrestart後のexclusive writer lockで決着するまでabsenceを未commitの証拠にしない。

startupはpublic admission前にreachable closureをreplayし、direct indexとmaterialized filesのparityを確認する。unreachable stagingはeffect authorityにならず、遅延workerはroot expected revisionとgeneration fenceを満たさなければpublishできない。

#### Legacy bootstrap and F3

bootstrapはsource inventoryを一度固定し、bounded batchごとにPhase 0 stagingへ変換し、record count、public projection、known event result、owner relationを照合する。crash後はlast committed source cursorから再開する。unknown additive event bytesはraw-preservation envelopeへ保持し、解釈不能なrequired eventはscope quarantineまたはglobal Failedとして公開する。

Issue #1499のbootstrapが自動回復へ載せるのは、legacy dataからsame identity、exact payloadまたはsafe observation、owner / runtime guardを一意に復元できる作業だけである。これらを証明できないlegacy dangling turnやfork状態を推測でInterrupted(Crash)へ終端せず、Paused / Failed / ReconciliationRequiredとしてeffect 0件で表示する。dangling turn全体の修復とfork recovery state整理は#1406へ残す。

Phase 0→F3はD3のone-shot importを使う。import中はread-only、parity成功後にauthority pointerをSQLiteへ切り替え、live mutation後はPhase 0へrollbackしない。public APIは同じdomain / usecase contractのままgatewayだけを差し替える。

### UI/UX

- send開始時にcomposer snapshotとcaller-generated operation IDを対応付け、同じattempt中は再生成しない。
- Accepted receipt受領時だけ対応snapshotをclearする。RejectedBeforeCommit、OutcomeUnknown、PayloadConflictでは保持する。
- response喪失は同じoperation IDのqueryまたはsame-payload retryを提示し、別operation IDで自動送信しない。
- Accepted後のReconciliationRequired / Failedはreceiptと別行にせず、同じ送信operationのlatest statusとして表示する。入力を復元しない。
- pending recoveryはsession / workflow ownerまたはClosedSession / ArchivedSession / UnownedRuntime partitionへ表示し、backendが返したactionsだけを有効にする。
- feedbackは対象sessionに表示し、identity単位のdismiss / retryを行う。capacity failureやstale revisionをgeneric toastへ潰さない。
- view closeはsession operationを起こさない。session close、archive、backend switch、application quitはdecision tableの別actionとして表示する。
- shutdownはfirst accepted intent、phase、counts、deadline、safe failure、available actionsをbackend projectionのまま表示する。Current(None)は正常なshutdown不在だけに使う。

### Algorithm

#### Normal send

1. controllerはoperation ID、DTO bounds、numeric encodingを検証しcanonical commandへ変換する。
2. usecaseはinstallation generationとoperation IDのgateを取得する。
3. owner-only keyでexact canonical command bytesのHMAC-SHA256 bindingを計算する。
4. existing decisionがある場合、binding一致ならAcceptedまたはOutcomeUnknownをreplayし、不一致ならPayloadConflictを返す。
5. target、configuration、active-turn policyを検証し、human input、turn / queue identity、provider dependency、immutable receiptを一つのclosureとして組み立てる。
6. closure commit前failureはRejectedBeforeCommit、writer結果不明はOutcomeUnknownとする。
7. Committed確認後だけAcceptedを返し、必要なexternal effectをdispatchする。
8. effect開始直前にoperation、obligation、owner revision、runtime generationを再検査する。staleならeffect 0件でReconciliationRequiredへ残す。

same operation gateはTauri / WebSocket並行requestを一winnerへ収束させる。server生成IDとmutable session stateをbindingへ含めないため、受理後state変化はPayloadConflictを起こさない。

#### Terminal closure

terminal candidateはsession / turn identity、expected revision、runtime epoch、command generationを持つ。off-lockでcandidate payloadを作り、terminal gate内でcurrent ownerとterminal absenceを再確認する。winner closureへfinal parts、assistant message、terminal result、session state、permission settlement、queue state、関連Stop resolutionを全て入れる。

commit後だけnotificationする。notification failureはsnapshot再取得で回復しterminalをrollbackしない。late candidate、old runtime event、過去turn eventはidentity / revision fenceでeffect 0件とする。

#### External effect

effectを開始するpathは次の共通順序を使う。

1. stable effect identityとcapabilityを含むobligationをPendingとしてcommitする。
2. owner / runtime / executor fenceを再確認しEffectReservedとclaimをcommitする。
3. provider / workflow / OS adapterへ同じeffect identityを渡す。
4. resultまたはsafe observationをowner stateと同じclosureへcommitする。
5. result不明はReconciliationRequired、readback可能ならReadAgain、idempotent retry可能ならRetrySameEffectを提示する。

capabilityとstable keyの不正な組合せはInvalidEffectIntent、effect 0件である。blind retryは行わない。

#### Stop deadline

Stop acceptanceはterminal gate、provider session lock、runtime event lockを待つ前にdeadline permitを確保する。Accepted時刻を起点にabsolute 10秒timerを独立taskで動かし、provider interruptは別taskで行う。watchdogはruntime event lockを待たずterminal candidateを提出できる。

10秒時点でstorageが利用可能ならInterrupted(Timeout) candidateをterminal closureへ送る。保存不能ならnormal Idleへ進めず同じStop / TerminalCommit obligationをReconciliationRequiredに保つ。late provider resultはturn identityとterminal revisionで拒否する。

#### Recovery discovery and action

startupはpending indexだけを読み、event全履歴または全session directoryをscanしない。current association filterは`(plan_id, epoch, obligation_id)` rangeだけを走査し、decoded entryのassociationを再検証する。range外またはassociation不一致を混ぜず、同じplan IDの別epochも別集合として扱う。same-boot claimはleaseとgenerationを、restart後claimはexclusive writer lockとold owner不在を確認してreclaimする。provider establish dependencyがterminal successになるまでdependent sendを開始しない。

page queryは開始時にimmutable root revisionとread leaseを固定し、途中のcommitを同じpage chainへ混ぜない。plan固定snapshot queryはrequestのplan / epoch / snapshot refを保存済みplanへ照合してから指定partitionだけを読み、0件も成功したempty pageとして確定する。snapshot error、cursor error、details compaction、unknown partitionをcurrent queryの実行へ切り替えない。shutdown snapshotが同時commitとの競合で2秒以内に一貫したrootを固定できなければQueryBusy、固定後のbounded foldが2秒を超えればDeadlineExceededを返し、partial count / entry / cursorを返さない。

actionはaction attempt direct lookupを最初に行う。Completedなら保存済みsafe result bytesとhashだけを検証してreceipt / resource viewを返し、current resource resolverを呼ばない。Absentならcurrent revision、action availability、capability、effect identityを検証してattemptをreserveする。external I/O後の結果はkind-specific closureでowner stateと同時確定する。writer結果不明はActionOutcomeUnknownとしてsame action lookupだけを許す。

#### Failure feedback

request DTOのvalidated session IDをscopeとし、SessionMeta読込前にfeedback capacityをreserveする。raw errorをSafeOperationFailureへ変換して同identityをinsertまたはrevision更新する。dismiss / retryはexpected revision CASを使い、別entryへ影響しない。既存identityのresolution retryはcapacity permitを追加取得せず、再failureなら同じCAS closureでfailure / actionsを置換し、revisionを+1、global unresolved countを±0としてcommitする。

#### Close, archive, and backend switch

decision tableをRustのclosed policy matchへ変換する。view closeはfrontend only、session operationはusecase commandだけが行う。active close / archiveはSessionClosed terminal closureを先に、Idle close / archiveはsynthetic terminalなしでlifecycle closureを行う。runtime close obligationはclosure後にdispatchする。

normal session closeのacceptance closureは、close operation identity、expected session / runtime generation、activeならfinal partsを含む一意なSessionClosed terminal、Closed lifecycle、既存queueを保持したPaused、`ObligationKind::SessionClose`のPending recordとstable effect identityを同じrootへ入れる。Idleならterminal participantだけを省き、他participantは同じである。root commit確認前はruntime closeを0件とし、commit後だけobligation dispatchへ渡す。commit直後、effect直後、result closure直前のcrashはstartup pending indexから同じobligationを回収し、capabilityに従うreadbackまたはsame effect identityだけを使うため、runtime closeは最大1件、active terminalは1件、Idle synthetic terminalは0件に保つ。effect結果が一意でなければClosedとqueue pauseを戻さずReconciliationRequiredへ残す。

backend switchはIdleとpending work absenceを同じrevision guardで検証し、queue pauseとdesired backendをcommitしてold runtime closeを開始する。結果不明時はold effective backendを維持しnew backendを開始しない。

normal session close、open / closed archive、backend switchはrequest起点のabsolute 10秒deadlineを持つ。deadlineまでに完了を確認できなければ同じoperation identityの結果未確認を返し、late completionはowner / runtime revision fenceを再検査する。late resultからsessionをreopenせず、terminal、runtime close、backend startを重複させない。

#### Shutdown

1. 全native / Tauri / WebSocket ingressをcanonical ShutdownExitIntentへ変換する。
2. request identity bindingを検査し、current flightがあればrequest identityと提示intentが別でもfirst operation ID / intentへjoinする。same identity / different intentだけをPayloadConflictとする。
3. new root-initはprior latest、scope fence、admission / store guard、retiring pointerをfresh snapshotで検査する。priorがnonterminalまたはscope fenceありなら`PreviousShutdownReconciliationRequired`、eligible terminal details Availableでretiring Someなら`PreviousShutdownCompactionPending`をnew plan / effect 0件で返す。Availableかつretiring Noneならpriorをretiringへreserveし、new Preparing root / latest-attempt pointerと同じclosureで確定する。Compactedならarchive parityを検証してretiringを変更しない。
4. T0でnew mutation admissionを閉じ、open session / running workflow targetとpreexisting recovery snapshotを固定する。
5. 最大4096 targetを128件以下 / 1 MiB以下のimmutable pagesへ書き、全page参照付きPrepared planをcommitする。
6. 一targetでも準備不能なら明示effect 0件を確認してoriginal candidate数ではなくrootから取得可能にした成功済みtarget count、preexisting recovery count、safe failure、成功済みpage / target authority / snapshot refを持つ`AbortedBeforeActivation(details=Available)` terminal rootをcommitし、15秒以内にadmissionを再開する。個別page closureはpage / authorityだけをstaging commitしPreparing root / latest-attemptを変更せず、このterminal closureだけが成功済みprefixとpointerを同時に到達可能にする。このclosureではarchive、detail detach、GCを行わない。
7. Prepared rootをActivatedへCASできたことを確認した場合だけtarget obligationをreserveしeffectを開始する。
8. T0+13秒以後は新しいtarget effectを開始しない。完了targetを保持し、未完了targetの公開stateは必ずReconciliationRequiredへ残す。process exitとの結合で作用結果を確認できない事実はそのtargetの`SafeEffectObservation::ExitCoupledOutcomeUnknown { plan_id, epoch }`として併記し、effect identityはtarget authority / state hashで束縛してpublic observation fieldへ複製せず、public state variantにも追加しない。
9. Activated後またはactivation outcome未解決のままT0+15秒へ達した場合はabortせず、fixed intentのone-shot permitでexitする。
10. restartはCompleted targetをskipし、未完了targetを同じplan / epochで公開する。明示actionなしにeffectを再実行しない。
11. root-init / Prepared / activation path外のlow-priority compactorはretiring planだけを三段階で処理する。`ArchiveSwitch`はquery / backup / import lease 0、source root / revision、pending / obligation-state fold revision、latest / retiring pointerを再検査し、Compacted archiveと、original page suffix・authority ordinal / prefix・pending materializationをcleanup-only authorityとして持つcompact root、retiring pointer、必要なsame-plan latest-attempt CASを同じclosureへ置く。detail fileは削除せず、commit直後からqueryをarchive-onlyへ切り替える。
12. Phase 0 `DetailDetachChunk`はstable target / authority key順に最大64 record、decoded 4 MiB、50 msのordered batch overlayとcompact-root cursor / pointerを先にcommitし、post-commit materializerがfileを冪等削除する。partial physical deleteはsame pending batchから再開し、direct absenceは完了証拠だけに使う。future F3は同stepを`InventoryDetachChunk`としてkind 5 / 9 row state、owner delete、link delete、distinct semantic root CASへ写し、row / item absenceで再開する。
13. Phase 0 `FinalizeDetach`はdetail materialization完了とpage query lease 0を確認し、最大4 page、4 MiB、50 msのpage batch overlayをcommit後materializeする。全page absence後の0-page dedicated closureだけstage Complete / retiring Noneへ進める。future F3 SQLはlast atomic page batchまたはpage 0 transactionでclearできる。各logical commit / materialization / final clear前後のcrashはarchive、compact-root ordinal / prefix / residual suffix / pending transaction、pointerから再開し、ArchiveSwitch後のqueryをAvailableへ戻さない。
14. future F3 source freezeは、latest terminal Availableかつretiring Noneならmigration-exclusive reservationでpointerだけをNone→Someにしてから三段階を完了し、terminal Available 0、retiring None、residual / pending materialization 0を必須にする。このfuture import / backup / cutover手順はD3設計gateであり、#1499 runtimeがF3 cutoverを実行する受入条件ではない。

projection builderはplan root、effect reservation / dispatch / observation count、terminal fence、admission、store health、coordinator boot IDを同じsnapshotから読み、Interface節の全predicateが真の場合だけ`RetryQuit`を追加する。known quit operation queryはdirect authorityからnormal planまたはbootstrap flightを一度だけ選び、unknown / corrupt / acceptance writer unknown / referenced transaction unknownをそれぞれNotFound / Internal / outer OutcomeUnknown / Accepted内OutcomeUnknownへ写す。normal current queryや他branchをfallbackに使わない。

bootstrap中のquitはnormal shutdown target / planを作らない。acceptance closureでbackend opaque `ApplicationQuitOperationId`、caller binding、`BootstrapFlight` locator、`Settling` flight recordを保存し、current batchをcommitまたはrollback-confirmedへcheckpointして同じ15秒deadlineとfixed intentでprocess exitする。次回bootで旧`coordinator_boot_id`のprocess不在を確認したら、初回acceptanceとは別の`bootstrap-application-quit-transition/v1` closureがexpected flight revisionをCASし、同じflight recordを`Exited`または`ReconciliationRequired`へ一方向に確定する。same caller key replayとoperation ID queryはnormal planへfallbackせず同じBootstrap projectionを返す。

#### Automatic bootstrap

source inventory固定前はInspectingSource、bounded conversion中はImporting、parity checkはVerifying、authority pointer writer開始後はActivatingである。各batchの`LegacyBootstrapCursorV1`とstaging rootを同じcheckpointへ保存する。cursorが指すentry / record / substepより前だけがcommittedであることを再開時に検証し、途中substepを先頭から重複emitしない。

authority pointerのwriter結果不明はLegacy / Phase0を推測せずActivatingを維持する。fresh bootでpointerを解決し、Legacyなら同じcursorから再開、Phase0ならreachable closure replayとparity確認後にnormal admissionを開く。

#### F1b and fault matrix

production runtime event apply pathへClaude / Codex wire fixtureを入力する。wire→public eventとpublic event→session resultのgoldenを別testにし、test-only reducerをexpected stateの生成へ使わない。

D1 #1445、F1 #1383、L1 #1402、L2 #1403、L4 #1405、L6 #1407、L7 #1408、L8 #1409、L10 #1411、S10a #1398、P2 #1414、X1 #1417の既存公開fixtureをregression suiteへ固定し、#1499適用前後の利用者可視resultを比較する。

fault matrixはsend acceptance、permission、provider establish、streaming、terminal、Stop、recovery publication、close、shutdown、bootstrapについて、writer開始前、root cutover前後、result返却前、notification前、process restartを網羅する。各caseでpublic stateが変更前または完全確定後だけ、external effectが0または1、identityが同じであることを検査する。

shutdown compaction matrixはdetail 0 / 1 / 63 / 64 / 65件、decoded 4 MiB-1 / exact / +1、page 0 / 1 / 4 / 5件、page bytes 4 MiB-1 / exact / +1、fake clock 50 ms境界を固定する。ArchiveSwitch、各logical batch commit、各item post-commit materialization、0-page final clearのbefore / after crash、active query lease、source root drift、authority residual、別retiring中のFailed / Cancelled、restartを組み合わせ、query一方向切替、compact-root cursor / prefix、partial materialization resume、最大2 detail set、dedicated final clearをPhase 0 fault testで検査する。future migration reservation、terminal Available 0 freeze、backup / restore / F3 import parityはD3 contract fixtureとして設計をlintし、#1499 runtimeへ実行routeを追加しない。

### Infra

新しいserverやremote serviceは追加しない。loopback WebSocketは既存local API process、auth token、shutdown lifecycleを共有する。

blocking file I/Oはtokio runtime上で直接行わず、bounded single-writer workerへ送る。normal、critical terminal / Stop、shutdownのlaneを分け、critical requestをnormal backlogでstarveさせない。queue countとdecoded byte permitの双方を取得できないrequestはwriter開始前にCapacityExceededとする。

fake clock、fault-injectable store、recording fake provider / workflow / process exit portをtest supportへ置く。実provider process、CLI、networkはF1bで起動しない。

structured telemetryはoperation kind、opaque correlation identity、phase、duration、safe failure kind、queue depthだけを記録する。content、image bytes、permission payload、exact request binding、HMAC key、filesystem pathは記録しない。

### Traceability

| Requirement | Behavior | Design section / type / algorithm | Fault / contract verification |
| --- | --- | --- | --- |
| R-001 | B-001, B-002, B-003, B-004, B-005, B-006, B-007, B-008, B-009, B-099 | Architecture「Authority and commit points」、Interface「Normal send」、Data Model `SendOperationRecordV1`、Algorithm「Normal send」 | acceptance root前後、response喪失、restart、Tauri / WebSocket並行、existing / new session / queued disposition、cross-principal replay / queryのfault matrixでreceipt非開示、NotFound、identity、message / turn / queue / effect各最大1を固定する |
| R-002 | B-010, B-011 | Interface「Common identity and safe failure」「Normal send」、Data Modelのexact request binding、Algorithm「Normal send」step 3–4 | exact payload各fieldのone-at-a-time mutationと受理後session state変化を入力し、PayloadConflict / same receiptとeffect 0を分離する |
| R-003 | B-008, B-012, B-013 | Interface `SendAgentMessageResult`、UI/UXのcomposer snapshot ownership、Algorithm「Normal send」step 6–7 | Accepted、RejectedBeforeCommit、OutcomeUnknown、Accepted後failure、送信中追加入力のcomponent / presenter testでclear対象と自動send 0を固定する |
| R-004 | B-014, B-015, B-016, B-017, B-018, B-019, B-020, B-021, B-095 | Architecture「Mutating path classification」、Interface「Pending recovery and actions」、Data Model `ObligationKind`、Algorithm「External effect」「Close, archive, and backend switch」 | obligation commit前後、permission payload欠損、provider establish依存、effect直後crash、session close各境界で受理前effect 0、same effect identity、未保存part非表示を検査する |
| R-005 | B-007, B-014, B-018, B-019, B-022, B-023, B-036, B-063, B-064, B-088, B-095 | Architecture「Authority and commit points」、Database「Commit and replay」、Algorithm「Terminal closure」「Recovery discovery and action」「Shutdown」 | root cutover前後、writer result喪失、restart、known quit locator破損を注入し、変更前 / 完全確定後 / 規定OutcomeUnknownだけとInternal非fallbackを全surfaceで比較する |
| R-006 | B-022, B-023, B-024, B-025 | Data Model `TerminalRecordV1`、Algorithm「Terminal closure」、UI/UXのqueue pause mirror | normal / Stop / close / archive / quit terminalの各participant writeと通知前にfaultを注入し、parts、message、terminal、session、permission、queueの同時可視性を検査する |
| R-007 | B-026, B-027 | Data Modelのsession / turn terminal unique key、Algorithm「Terminal closure」 | Stop、watchdog、close、Fatal、completionの全winner順と旧turn遅延eventを競合させ、terminal 1件とwinner result不変を検査する |
| R-008 | B-028, B-029, B-030, B-031, B-087 | Interface「Stop」、Data Model `StopOperationRecordV1` / caller binding KAT、Algorithm「Stop deadline」 | fake clock、permanent interrupt hang、identity / payload / 32・33 capacity境界、固定HMAC KATで10秒結果、InvalidRequest / PayloadConflict / StopCapacityExceeded、stale result effect 0を検査する |
| R-009 | B-032, B-033, B-034, B-080 | Data Model `StopOperationStateV1` / TerminalCommit obligation、Algorithm「Stop deadline」「Recovery discovery and action」 | Stop acceptance保存failure、terminal closure failure、10秒、storage復旧、restart / manual競合でcapacity保持、ReconciliationRequired、terminal / resolution各1を検査する |
| R-010 | B-035, B-036, B-037, B-038, B-039, B-040, B-090 | Interface `PendingRecoveryFilter` / page / cursor、Data Model pending-only secondary index、Algorithm「Recovery discovery and action」 | 全kind / owner / partition、201件、途中更新、cursor改変 / restart、plan / epoch associationを入力し、bounded page、filter純度、mutation抑止、effect / publication各最大1を検査する |
| R-011 | B-041, B-042, B-043, B-044, B-045, B-046, B-094 | Interface「Failure feedback」、Data Model `FeedbackStore`、Algorithm「Failure feedback」 | data / meta双方破損、33件page、512件capacity、stale revision、retry再failure、UTF-8 byte境界でsame identity revision +1、count ±0、safe field / exact errorを検査する |
| R-012 | B-047, B-048, B-049 | Architecture「Production cutover」、Algorithm「F1b and fault matrix」、Infraのrecording fake ports | Claude / Codex wire fixtureをproduction public interfaceへ通し、wire adaptorとprojectionを別々にmutationし、既存F1維持と外部process / network 0を検査する |
| R-013 | B-050, B-098 | Architecture「Production cutover」、Database「Legacy bootstrap and F3」、Data Model / Algorithm「Shutdown」、Alternatives ConsideredのF3境界、D3正本 | D3 lintで必須13論点、決定 / 却下 / failure / migration / bound、one-shot cutoverを検査する。future handoffにmigration-only reservation、terminal Available 0・retiring None・residual / pending materialization 0 freeze、managed backupとの差異を要求し、#1499 runtimeへのF3 cutover / managed backup route混入0を検査する |
| R-014 | B-051, B-052, B-053, B-054, B-055, B-056, B-095, B-101, B-102, B-103 | Interface / Algorithm「Close, archive, and backend switch」、`SessionLifecycleOperationId` / `ObligationKind::SessionClose`、close / quit decision table | 全surface行schema、active / Idle close・archive、closed archive、backend switch、same-key replay / conflict、別key join / PendingOperation、cross-principal query、response喪失、10秒hang、restartを入力し、同じreceipt / saved outcome、NotFound、terminal有無、queue、runtime effect最大1を検査する |
| R-015 | B-039, B-057, B-058, B-059, B-060, B-061, B-062, B-087, B-088, B-091, B-092, B-096, B-097, B-100 | Interface「Close and shutdown」`ApplicationQuitOperationId` / locator / `GetApplicationQuitOperationApplicationError`、Data Model「Shutdown」、Algorithm「Shutdown」 | 全graceful surface、same / different caller keyとintent、root writer結果不明、normal / bootstrap locator matrix、RetryQuit predicate、4096 / 4097・page byte境界、previous / retiring planを入力しsingle flight、top-level OutcomeUnknown、最大2 detail set、effect各最大1を検査する |
| R-016 | B-063, B-064, B-065, B-066, B-067, B-089, B-096, B-097 | Interface `ShutdownSummary` / `GetPendingRecoverySnapshotRequest` / `GetPendingRecoverySnapshotApplicationError` / shutdown projection、Data Model / Algorithm「Shutdown」、close / quit decision table「Deadline」「Shutdown readback」 | fake clockでpre-activation failure、activation outcome unknown、T0+13 / +15、exit-coupled child、snapshot error matrix、details Available→Compactedの途中crashと再開を入力し、64 record / 4 page・4 MiB / 50 ms、整理完了前の全detail可読、整理完了後のsummary可読とdetail `DetailsCompacted`、current plan pointer不変、非fallbackを検査する |
| R-017 | B-068, B-069 | Architectureのbounded index、Cross-cutting「Bounds」「Performance」、Algorithmのbounded recovery query、Infraのrelease benchmark fixture | 10 / 1000000件で1000 sample、同時commit、2秒超過を測定しp95比、50 ms / 20 ms / 50 ms、QueryBusy / DeadlineExceeded、partial 0を検査する |
| R-018 | B-038, B-062, B-070, B-071, B-072, B-073, B-074, B-075, B-076, B-088, B-089 | Interface「Tauri and WebSocket」「Numeric encoding」、Data Model「Bootstrap」、Database「Legacy bootstrap and F3」、Algorithm「Automatic bootstrap」 | legacy cursor全境界、crash再開、bootstrap quit / locator、snapshot、WS auth / bounds / reconnect、integer domain、shutdown authority破損を両surfaceで同じfixtureから検査する |
| R-019 | B-077 | Algorithm「F1b and fault matrix」、Cross-cutting「Compatibility」 | B-077 trace matrixの各rowに記載した正本path / exact anchorとcheck / testを記載inputで実行し、expected resultおよびmessage / terminal / queue / notice / external effect重複0を確認する。D1 #1445はdesign-only contract checkでありruntime fixtureを要求しない |
| R-020 | B-026, B-034, B-078, B-079, B-080 | Data Model `TerminalRecordV1` / `StopResolutionRecordV1`、Algorithm「Terminal closure」「Stop deadline」 | Stop winner / superseded全競合、terminal保存failure、restart / retryでterminal participantとresolution各1、capacityのcommit時解放だけを検査する |
| R-021 | B-019, B-021, B-039, B-081, B-082, B-083, B-084, B-085, B-086, B-093 | Interface recovery action closed types、Data Model `RecoveryActionAttempt` completed safe bytes、Algorithm「Recovery discovery and action」 | 5 action kind、response喪失、restart、revision更新、details compaction、resolver failure、invalid / stale identity、last target、writer unknownでexact replay、effect 0、plan terminalを検査する |

## Alternatives Considered

- payload本文だけのdedupe: 同文の意図的な複数送信とretryを区別できないため採用しない。caller operation identityとexact request bindingを使う。
- 公開二段階send: #1499の既存send互換を壊し、response喪失解決に不要な公開lifecycleを増やすため採用しない。single send内部のclosure commitだけを使う。
- provider effect後にintentを保存: effect直後crashで同じeffectを安全に回収できないため採用しない。obligationをeffect前にcommitする。
- terminal event、message、metaを別commitにする: 部分terminalを公開するため採用しない。terminal closureを一つのcommit pointにする。
- event全履歴または全session directoryをstartup scanする: 履歴件数依存と未発見を生むため採用しない。pending-only indexを使う。
- surfaceごとのquit処理: intentとdeadlineが競合するため採用しない。全surfaceをsingle coordinatorへ接続する。
- tokio timeoutで同期file writeをcancelする: writerがcommitを続ける可能性を未受理と誤判定するため採用しない。OutcomeUnknownとidentity lookupを使う。
- #1499でF3 SQLiteまで実装する: milestone境界を越えるため採用しない。#1499はD3契約とPhase 0 bridgeを確定し、将来のF3 #1385がone-shot cutoverを実装・実行する。

## Cross-cutting concerns

- Crash consistency: root reachabilityとrequired syncだけをcommit根拠にし、manifest存在、future完了、notification成功を根拠にしない。
- Idempotency: send、Stop、quit、session lifecycleはstable caller identityとexact request bindingを持ち、同key異payloadはPayloadConflict、same key / payloadはsame decisionである。recovery actionはbackend発行のself-binding action IDを使い、未発行または改変identityはNotFound、stale / unavailable / target revision変更はclosed rejectionへ写す。recovery action用PayloadConflictを追加しない。
- Security: send / Stop / quit / session lifecycle bindingはapp-data generationごとexactly oneのowner-only `AgentOperationBindingKeyV1.hmac_sha256_key`を別domain prefixで共用し、public DTO、log、telemetryへ出さない。key missing / duplicate / generation mismatch / ACL failureは4 commandのadmissionを閉じ、default keyを再生成しない。bindingとBearerはconstant-time比較し、raw errorとprivate payloadを返さない。
- Concurrency: operation gate、terminal gate、obligation claim、shutdown epochは別scopeにし、async provider call中にsession / event / shutdown lockを保持しない。
- Bounds: pending page 200件 / encoded 4 MiB、feedback page 32件 / global 512件、Stop 32件、shutdown target 4096件 / page 128件・1 MiB、WebSocket boundsをadmission前に検査する。
- Performance: pending first 200件p95 50 ms、identity query p95 20 ms / p99 50 ms、大規模fixture p95比1.25以下を同一release buildで1000 sample測定する。
- Compatibility: legacy public transcript / terminal / lifecycle / owner relationをparity fixtureで固定し、bootstrap完了までread-onlyとする。Tauri / WebSocketは同じpresenter goldenを共有する。
- Observability: phase、deadline、attempt、safe failure、effect countをcorrelation identityで追跡し、private contentを含めない。

## Risks

- Phase 0 file store上のmulti-record closureは複雑である。#1499の実装完了gateはPhase 0 root / manifest / participantのfault injectionとTauri / WebSocket parityであり、F3 one-shot cutoverのruntime実装完了を含めない。D3でcutover契約と検証手順を確定し、実装・実行はF3 #1385のgateとする。
- uninterruptible OS I/Oによりworker task自体がdeadline後も残り得る。old generationのroot CASをfenceし、OutcomeUnknownを未受理へ推測しない。
- storage全損中はAccepted Stopのterminalを10秒以内に書けない。Accepted factとdeadline permitを保持し、ReconciliationRequiredとして通常Idle / queue drainを抑止する。
- external WebSocket callerがoperation IDとexact payloadをrequest前にdurable保存しなければresponse喪失後のidentity queryはできない。built-in desktop callerはRust-owned caller-attempt journalでこれを満たし、external callerについてはprotocol preconditionとして明示し、server側のpayload scanや別identity生成で補わない。
- accepted operation recordとcompleted action receiptはretry保証のため増加する。#1499では意味を変えるtime-based GCを導入せず、将来retention policyは別Issueで決める。
- graceful eventを配信しないhard kill、power loss、OS強制終了は15秒coordinatorを通らない。次回起動のpending recoveryとshutdown readbackで回収する。
