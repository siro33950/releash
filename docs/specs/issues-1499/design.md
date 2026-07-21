# Design

## The actual design

### Architecture

#1499はF2 #1384とF3 #1385を内包し、暫定file-store bridgeを作らず次の恒久境界を同じreleaseで導入する。

1. agent message partとagent / workflow eventはdomain layerが所有する。
2. usecaseはcaller operation identity、terminal arbitration、durable obligation、recovery、shutdownを所有する。
3. bundled SQLite local event storeはagent sessionとworkflowを跨ぐ唯一のmutation authorityである。
4. gatewayはSQLite schema、versioned persistence DTO、legacy migrationを所有する。
5. Tauri / WebSocketは同じusecase / query serviceとpresenterを使い、frontendは結果をmirrorする。

`specs/milestone-84-agent-chat-stabilization/d3-durable-event-store-design.md`の必要な判断は本書へ統合済みである。D3は設計経緯として参照できるが、実装に追加判断を要求する正本ではない。close / quitの利用者可視意味論は`specs/milestone-84-agent-chat-stabilization/close-quit-decision-table.md`と一致させる。

#### Layer ownership

| Layer | Owner | Placement |
| --- | --- | --- |
| domain | `MessagePart`、`AgentSessionDomainEvent`、`WorkflowDomainEvent`、operation / terminal / obligation / recovery / shutdownのclosed type、`LocalEventTransactionRepository` port | `src-tauri/src/domain/agent_session/`、`domain/workflow/`、`domain/local_event/` |
| usecase | send acceptance、terminal arbitration、Stop deadline、effect reservation、recovery、feedback、session lifecycle、shutdown coordinator | `src-tauri/src/usecase/agent_session/`、`usecase/shutdown_coordinator.rs` |
| adaptor/gateway | SQLite repository、schema migration、persistence DTO、legacy reader / importer、bounded query implementation | `src-tauri/src/adaptor/gateway/local_event_store/` |
| adaptor/controller / protocol | Tauri / WebSocket input validation、usecase call、public DTO mapping | `src-tauri/src/adaptor/controller/`、`adaptor/protocol/` |
| infrastructure | provider / workflow / child process effect、native quit ingress | existing runtime / platform modules |
| frontend | caller attempt identity、composer snapshot、backend projectionのmirror、明示action | existing hooks / components |

依存方向はcontroller / gateway → usecase → domainである。domainは`serde`、rusqlite、filesystem、Tauri、WebSocketへ依存しない。usecaseに同義`MessagePart`、SQL row、persistence envelopeを置かない。

#### Commit ownership

| Fact | Transaction participant | Commit後の派生物 |
| --- | --- | --- |
| send acceptance | exact request binding、human input event、turnまたはqueue identity、必要なobligation | immutable receipt、status emit |
| streaming observation | part event、message revision、turn fence | live / reload projection |
| terminal | terminal event、final parts、assistant message、session / permission / queue state、Stop resolution | terminal notification |
| external effect | operation / obligation、effect identity、owner revision、safe observation | pending recovery page |
| session lifecycle | lifecycle operation、session state、必要なterminal / queue pause / close obligation | session / history projection |
| shutdown | caller binding、plan、target / recovery snapshot、phase、result | progress emit、exit permit |

表の各行は一つのSQLite transactionで確定する。notification、WebSocket publish、frontend updateはcommit根拠ではなく、失敗してもidentity queryから同じ結果を再取得できる。

#### Production cutover

1. 起動時にexclusive app-data writer lockを取得し、`LocalStoreAuthorityPointerV1`を読む。
2. 新規installationは空のSQLite generationを作成して`Sqlite` authorityを発行する。
3. `Legacy` authorityはmutation / provider effect admissionを閉じ、legacyをread-only表示しながらstaging SQLiteへ自動migrationする。
4. parity成功後だけauthority pointerを`Legacy`から`Sqlite`へ一度CASする。
5. cutover後はSQLiteだけを読み書きし、legacy dual write、record単位fallback、legacy rollbackを行わない。
6. production compositionはすべての#1499 mutationを同じ`LocalEventTransactionRepository` instanceへ注入する。旧file-store append成功をcommitとみなす経路、全履歴scan fallback、`phase0_*` module / type / tableを残さない。

F4 / F5のprovider wire全体のtyped adapter化、F8の追加query、F10のqueue lifecycle全体は後続である。ただし#1499に必要なproduction event apply、identity lookup、pending recovery、terminal、shutdown queryは今回のSQLite schemaとportで実装する。

#### Implementation order

次はruntime phaseやsubphaseではなく、同じ#1499内の直列依存である。後段taskは前段のproduction codeとcontract testを使い、同義port / checkpoint / schemaを作らない。

1. domain `MessagePart`を単一定義にし、agent / workflow domain eventとlegacy / public DTO converterを確定する。
2. `LocalEventTransactionRepository`、SQLite schema、single writer、idempotency / sequence / direct index、fault harnessを実装する。
3. send / Stop / session lifecycleのcaller journal、operation binding、acceptance / identity queryをSQLite portへ接続する。
4. terminal batch、durable obligation、pending recovery / action、feedbackを接続する。
5. shutdown coordinator、plan / target / snapshot、detail compaction、migration-safe quitを同じSQLite port / schemaへ接続する。step 2のportとstep 4のobligationを前提とし、独立checkpoint authorityを作らない。
6. Legacy→staging SQLite migration、parity、authority pointer cutoverを実装する。step 1–5の全known type / table / projectionが揃う前にcutoverを有効化しない。
7. Tauri / WebSocket / frontendを同じusecase / presenterへ接続し、旧file-store mutationと旧全履歴scan fallbackをproduction compositionから外す。
8. F1b、legacy compatibility、fault / restart / performance matrixを通し、新規installationとupgradeのnormal admissionを有効化する。

### Interface

#### Store port

内向きportは次の責任だけを公開する。SQL、row ID、WAL、serializationを型へ漏らさない。

```rust
#[async_trait::async_trait]
pub trait LocalEventTransactionRepository: Send + Sync {
    async fn commit_batch(
        &self,
        batch: LocalAtomicBatch,
    ) -> Result<CommitBatchResult, CommitBatchError>;

    async fn resolve_commit(
        &self,
        identity: CommitIdentity,
    ) -> Result<CommitResolution, LocalEventQueryError>;

    async fn load_stream(
        &self,
        request: LoadStreamRequest,
    ) -> Result<DomainEventPage, LocalEventQueryError>;

    async fn query(
        &self,
        request: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError>;

    fn subscribe(&self, after: GlobalSequence) -> LocalEventSubscription;
}
```

`commit_batch`だけがmutation入口である。query methodはsnapshot readだけを行い、repair、migration、projection rebuildを暗黙実行しない。subscription lag時は`after`からbounded replayし、保持範囲外なら`ReplayRequired`を返す。

`LocalEventQuery`は`CommitByIdentity | StreamPage | OperationByIdentity | TerminalByTurn | PendingRecoveryPage | PendingRecoverySnapshotPage | RecoveryActionByIdentity | CurrentShutdown | ShutdownPlanPage | LocalStoreMigration`のclosed sumである。`LocalEventQueryResult`も各queryと一対一のclosed sumとし、generic row / JSON / mapを返さない。F8 / F10が追加するqueryは別variantと専用resultをschema migrationと同時に追加する。

#### Atomic batch

```rust
pub struct LocalAtomicBatch {
    pub commit_id: CommitIdentity,
    pub idempotency: IdempotencyBinding,
    pub expected_heads: Vec<ExpectedStreamHead>,
    pub events: Vec<UncommittedDomainEvent>,
    pub state_mutations: Vec<LocalStateMutation>,
}

pub enum CommitBatchResult {
    Committed(CommittedBatch),
    Replayed(CommittedBatch),
}

pub enum CommitBatchError {
    PayloadConflict,
    StreamHeadConflict { current: StreamVersion },
    CapacityExceeded,
    SequenceExhausted,
    StorageUnavailable { failure: SafeOperationFailure },
    OutcomeUnknown { identity: CommitIdentity },
    Corrupt { correlation_id: String },
}
```

同じidempotency key / canonical payloadは同じ`CommittedBatch`へ戻る。同じkey / different payloadは`PayloadConflict`である。`OutcomeUnknown`は別identityを作らず`resolve_commit`または同じbatchの再試行で解決する。

#### Application commands and queries

公開endpointは次の一組である。snake_case名はTauri command、同じ行のrequest / resultはWebSocket V1でも共用する。

| Endpoint | Kind | Main result |
| --- | --- | --- |
| `send_agent_message` | command | `Accepted { receipt, latest_status } | RejectedBeforeCommit | OutcomeUnknown` |
| `get_agent_send_operation` | query | immutable receiptとlatest status |
| `stop_agent_session` / `get_stop_operation` | command / query | Accepted Stop、terminal / resolution、ReconciliationRequired |
| `request_session_lifecycle` / `get_session_lifecycle_operation` | Tauri command / query | close / archive / backend switchのstable operation |
| `list_pending_agent_recovery` | query | current pending-only bounded page |
| `get_pending_recovery_snapshot` | query | shutdown plan固定snapshot page |
| `resolve_pending_recovery_action` / `resolve_shutdown_target_action` | command | action identityに束縛したresult |
| `get_recovery_action` | query | saved action attempt / result |
| `get_local_store_migration` | query | migration progress。通常稼働時は`None` |
| `get_application_shutdown` | query | normal shutdown current projection |
| `request_application_quit` / `get_application_quit_operation` | command / query | `Shutdown | Migration` projection |
| `get_shutdown_plan` | query | plan / history bounded page |
| feedback query / dismiss / retry | query / command | session-scoped bounded feedback |

operation identityはsend / Stop / quit / session lifecycleで1..=128 bytes、`[A-Za-z0-9._:-]`である。WebSocket outer request IDはtransport重複制御だけに使い、operation identityの代用にしない。caller principal、app-data generation、operation kind、caller request ID、canonical exact payloadをinstallation keyでHMAC bindingする。keyはowner-onlyで、default再生成、public DTO、log出力を禁止する。

built-in Tauri callerはfrontendから受け取ったoperation / request IDとexact commandを、usecase dispatchより先にRust-owned `caller_attempts`へ保存する。保存できなければeffect 0件の`RejectedBeforeCommit`、保存結果不明なら同identityの`OutcomeUnknown`とする。response喪失またはUI restart後は同じattemptをidentity queryまたはsame-payload commandへ再接続し、Acceptedまたはdeterministicなpre-commit rejectionを確認した後だけjournal entryをclearする。これはowner-privateなlocal outboxであり、public prepared lifecycle、prepared list、content resolverを作らない。external WebSocket callerは送信前にidentityとexact payloadを自身のdurable stateへ保存することをprotocol preconditionとする。

#### Command result rules

- acceptance前failureはstate / external effectを0件にして`RejectedBeforeCommit`またはendpoint固有rejectionを返す。
- acceptance transactionの結果を確認できない場合は同じoperation identityのtop-level`OutcomeUnknown`を返す。
- acceptance後のeffect / completion結果不明はAccepted receiptを維持しlatest statusを`ReconciliationRequired`とする。
- 別principalによる既存identityのcommand / queryは`NotFound`で、存在を開示しない。
- same identity / different exact payloadは`PayloadConflict`で、既存operationを変更しない。
- direct queryはoperation recordをpoint lookupし、current session projectionから過去resultを再構築しない。

#### Endpoint errors

各Tauri endpointは行ごとのnamed error enumを持ち、WebSocketは同じ集合を共通envelopeへ写す。result内のRejected / Failed / OutcomeUnknown / ReconciliationRequiredをdirect errorへ複製しない。`StorageUnavailable`はbounded failure、`Internal`はcorrelation IDだけを持つ。

| Endpoint | Exact direct error |
| --- | --- |
| `send_agent_message` | InvalidRequest, PayloadConflict(Send), CapacityExceeded, FeedbackCapacityExceeded, MigrationInProgress, ShutdownInProgress, ResponseTooLarge, Internal |
| `get_agent_send_operation` | InvalidRequest, NotFound, QueryBusy, DeadlineExceeded, StorageUnavailable, Internal |
| `stop_agent_session` | InvalidRequest, PayloadConflict(Stop), FeedbackCapacityExceeded, MigrationInProgress, ShutdownInProgress, Internal |
| `get_stop_operation` | InvalidRequest, NotFound, QueryBusy, DeadlineExceeded, StorageUnavailable, Internal |
| `request_session_lifecycle` | InvalidRequest, PayloadConflict(SessionLifecycle), FeedbackCapacityExceeded, MigrationInProgress, ShutdownInProgress, Internal |
| `get_session_lifecycle_operation` | InvalidRequest, NotFound, QueryBusy, DeadlineExceeded, StorageUnavailable, Internal |
| `list_pending_agent_recovery` | InvalidRequest, CursorMismatch, CursorExpired, QueryBusy, DeadlineExceeded, ResponseTooLarge, StorageUnavailable, Internal |
| `get_pending_recovery_snapshot` | InvalidRequest, NotFound, SnapshotMismatch, CursorMismatch, CursorExpired, DetailsCompacted, QueryBusy, DeadlineExceeded, ResponseTooLarge, StorageUnavailable, Internal |
| recovery action command | InvalidRequest, MigrationInProgress, ShutdownInProgress, StorageUnavailable, Internal |
| `get_recovery_action` | InvalidRequest, NotFound, QueryBusy, DeadlineExceeded, StorageUnavailable, Internal |
| `get_local_store_migration` | StorageUnavailable, Internal |
| `get_application_shutdown` | Internal |
| `request_application_quit` | InvalidRequest, PayloadConflict(ApplicationQuit), CapacityExceeded, ResponseTooLarge, Internal |
| `get_application_quit_operation` | InvalidRequest, NotFound, QueryBusy, DeadlineExceeded, StorageUnavailable, Internal |
| `get_shutdown_plan` | InvalidRequest, NotFound, CursorMismatch, CursorExpired, QueryBusy, DeadlineExceeded, ResponseTooLarge, StorageUnavailable, Internal |
| feedback query | InvalidRequest, CursorMismatch, CursorExpired, QueryBusy, DeadlineExceeded, ResponseTooLarge, StorageUnavailable, Internal |
| feedback dismiss / retry | InvalidRequest, StorageUnavailable, Internal |

#### Public closed types

Requirements / Behaviorで使うclosed variantは次で固定する。

```rust
pub enum SendDisposition {
    StartedTurn { turn_id: String },
    Queued { queue_item_id: String },
}
pub enum SendExecutionStatus {
    AwaitingProviderStart { dependency_obligation_ids: Vec<String> },
    Queued { queue_item_id: String, reserved_turn_id: String },
    ProviderStartReserved { obligation_id: String },
    Running { turn_id: String },
    ReconciliationRequired { failure: SafeOperationFailure },
    Failed { failure: SafeOperationFailure },
    Terminal { result: TurnResult },
}
pub enum RecoveryActionKind {
    ReadAgain,
    RetrySameEffect,
    UseObservedResult,
    CancelIfSafe,
    KeepForManualResolution,
}
pub enum RecoveryResultClassification {
    Pending,
    Succeeded,
    ConfirmedNoEffect,
    Ambiguous,
    CancelledBeforeEffect,
    Unchanged,
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
pub enum ApplicationQuitProjection {
    Shutdown(ApplicationShutdownProjection),
    Migration(MigrationApplicationQuitProjection),
}
pub enum LocalStoreMigrationPhase {
    InspectingSource,
    Importing,
    Verifying,
    Activating,
    Failed,
}
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
```

`ApplicationQuitOperationId`はbackend発行でcaller request IDとは別である。normal shutdownは`ShutdownPlan { plan_id, epoch }`、migration中quitは`MigrationFlight { migration_id }`へ一意にlocateする。known-operation queryはlocator破損を`Internal`、acceptance transaction不明をtop-level`OutcomeUnknown`、acceptance後の参照transaction不明をAccepted内`OutcomeUnknown`とし、`Current(None)`や別operationへfallbackしない。

`BoundedNoticeText`はvalue、truncated、optional original bytes / digestを持つ。labelはUTF-8 160 bytes、detailは2048 bytes以下である。filesystem path、secret、raw SQL、provider payload、unbounded raw errorを含めない。`PayloadConflict | NotFound | InvalidRequest | CursorMismatch | CursorExpired | SnapshotMismatch | DetailsCompacted`はdurable failure kindへ水増ししない。

#### Bounds and encoding

- pending recovery: 200 entriesかつencoded 4 MiB以下。
- feedback: 32 entries / page、process unresolved 512件。
- Stop: unresolved distinct target 32件。
- shutdown: 4096 targets、128 targets / page、1 MiB / page、full detail set最大2件。
- WebSocket: 16 connections / process、32 in-flight / connection、60 requests/s burst 120、request / response 16 MiB、outbound 32 responses / 16 MiB。
- persisted revision / epoch / sequence / ordinal / count / offsetは0以上`i64::MAX`以下。wireは先頭ゼロなしASCII decimal string。page limit / byte limitはJSON非負整数、exit codeはJSON signed integer。

cursorはsnapshot identity、filter、last key、expiryをMACする。filter違い / 改変は`CursorMismatch`、restart / expiryは`CursorExpired`で、partial pageを返さない。

#### DTO ownership

domain / usecase typeにserdeを付けない。`adaptor/protocol`のV1 DTOはstructをsnake_case object、enumを`{ "type": "snake_case", ... }`のadjacently tagged objectとしてencodeする。missing / duplicate / unknown fieldを拒否し、`Option`はfieldを省略せず`null | value`で表す。ただしlegacy JSON reader / writerだけは変更前codecのcamelCaseとoptional field omissionを維持する。

Tauri / WebSocket presenterは同じcanonical resultから別transport envelopeへtotal mappingする。`serde_json::Value`、flatten、untagged enum、transport固有defaultをpublic schemaへ入れない。domainの`JsonPayload`はprotocol境界でだけvalidated JSON valueへ変換する。

### Data Model

#### Canonical message part

`src-tauri/src/domain/agent_session/entities/message_part.rs`の`MessagePart`を唯一のsemantic定義にする。usecase側の同義enumは削除し、session、runtime event、projection、test supportはdomain typeをimportする。

```rust
pub enum MessagePart {
    Thinking { content: String, parent_tool_use_id: Option<String> },
    Text { content: String, parent_tool_use_id: Option<String> },
    ToolUse { id: String, tool: String, input: JsonPayload, parent_tool_use_id: Option<String> },
    ToolResult {
        content: String,
        is_error: bool,
        tool_use_id: Option<String>,
        parent_tool_use_id: Option<String>,
        content_ref: Option<ToolOutputRef>,
        summary: Option<ToolOutputSummary>,
    },
    Error { content: String, parent_tool_use_id: Option<String> },
    Permission {
        request: PermissionRequest,
        status: PermissionPartStatus,
        answers: Option<JsonPayload>,
        parent_tool_use_id: Option<String>,
    },
    TaskStatus { task_tool_use_id: String, status: String, description: Option<String>, summary: Option<String> },
    TodoListSnapshot { items: Vec<TodoListItem> },
    SystemNotification {
        notification_type: SystemNotificationType,
        status: String,
        label: String,
        detail: Option<String>,
        hook_id: Option<String>,
    },
    Image { data: String, media_type: String },
    ImageRef { attachment: Attachment },
}
```

gatewayの`StoredMessagePartV1`は変更前JSON tag / field / camelCase / optional omissionをexactに再現する。protocolの`MessagePartDtoV1`は公開schemaを所有する。両者はdomainと明示変換し、known variantでlossy conversionを禁止する。unknown additive stored payloadはenvelope raw bytesを保持し、unknown required variantは`IncompatibleStoredEvent`でfail closedにする。

usecase側`PermissionRequestMsg`、`PermissionPartStatus`、`serde_json::Value` payloadはそのままdomainへ移さない。既存意味をdomainの`PermissionRequest`、domain-owned `PermissionPartStatus`、`JsonPayload`へ移し、legacy / public field名は各DTO converterで維持する。`ToolOutputRef`、`ToolOutputSummary`、`TodoListItem`、`SystemNotificationType`、`Attachment`も既存domain typeを直接使う。

#### Domain events

`AgentSessionEvent`をusecaseから`domain/agent_session/events/AgentSessionDomainEvent`へ移す。既存event variantとF1公開意味論は変えない。payloadの`MessagePart`、permission、tool output、turn / session identityはdomain typeを使う。operation acceptance、obligation、terminal / Stop resolution、session lifecycle、recovery publicationは同じenumのclosed variantとして追加する。

workflowのgateway-owned `WorkflowEvent`は`domain/workflow/events/WorkflowDomainEvent`へ移し、gatewayは`StoredWorkflowEventV1`との変換だけを持つ。`LocalDomainEvent`は`AgentSession | Workflow | Application`のclosed sumである。event type / payload versionはgateway registryが決定し、Rust type nameやserde tagをpersistent identityにしない。

#### Store identities

```rust
pub struct StreamId(String);
pub struct EventId(String);
pub struct CommitIdentity(String);
pub struct StreamVersion(i64);      // 0..=i64::MAX
pub struct GlobalSequence(i64);     // 1..=i64::MAX
pub struct StreamSequence(i64);     // 1..=i64::MAX

pub struct ExpectedStreamHead {
    pub stream_id: StreamId,
    pub expected: StreamVersion,
}
```

stream IDは`agent-session:<session_id>`、`workflow:<execution_id>`、`application`のnamespace付きopaque stringである。一batchは複数streamへeventを追加できる。`expected_heads`はbatchが変更する全streamをexactly once含む。

#### State mutations

`LocalStateMutation`は次のclosed familyで、任意SQLを許さない。

- operation caller binding / direct operation record
- built-in caller attempt journal
- message / session / queue projection
- terminal unique record / Stop resolution
- obligation state / pending index
- recovery action attempt / completed result
- session lifecycle operation
- shutdown plan / target / recovery snapshot / compact archive / latest pointer
- migration checkpoint / parity result

mutationはexpected revisionとnew revisionを持つCASである。eventとstate mutationは同じbatchでcommitする。terminal unique keyは`(session_id, turn_id)`、operation binding unique keyは`(principal, generation, kind, caller_request_id)`、idempotency unique keyは`(generation, operation_kind, idempotency_key)`である。

| Mutation family | Guard | Write |
| --- | --- | --- |
| operation binding | key absentまたはsame HMAC / operation ID | immutable caller binding |
| caller attempt | principal + operation kind + caller key absentまたはsame command hash | encrypted / owner-private exact commandとresolution state |
| operation record | absentまたはexpected revision | receipt不変、latest status / revision更新 |
| session / message / queue projection | expected session / message revision | complete projection row |
| terminal | `(session, turn)` absentまたはsame terminal identity | terminal resultとparticipant digest |
| Stop resolution | Stop operation ID absentまたはsame result | Succeeded / Superseded |
| obligation / pending index | expected obligation revision、pending key parity | state rowとpending indexを同時insert / delete |
| recovery action | action ID absentまたはsame binding / expected revision | attemptまたはimmutable completed result |
| shutdown | plan / epoch / target / latest pointer expected revision | plan、target、snapshot、archive、pointer |
| migration | migration ID / source inventory hash / checkpoint expected revision | phase、next checkpoint、parity result |

guard不一致をlast-write-winsへ変換しない。same semantic resultのreplayは保存済みresult、different resultはtyped conflictまたはintegrity failureである。

#### Persistence envelope

gateway-owned envelopeは次のfieldを必須にする。

```rust
pub struct StoredEventEnvelopeV1 {
    pub event_id: String,
    pub commit_id: String,
    pub stream_id: String,
    pub stream_sequence: i64,
    pub global_sequence: i64,
    pub event_type: String,
    pub payload_version: i64,
    pub occurred_at: String,
    pub payload: Vec<u8>,
    pub payload_sha256: [u8; 32],
}
```

payloadはcanonical CBORを新規SQLite eventのcodecとする。map key順、integer幅、float禁止、duplicate key禁止をcodec testで固定する。content hashはintegrity用でpublic semantic identityに使わない。unknown event type / versionはraw envelopeをそのまま返す`StoredUnknownEvent`として保持し、projectionが意味を必要とするstreamはfail closed、単に通過保存できるmigration / export内部処理はbytesを変更しない。

### Database

#### Physical layout and SQLite configuration

app-data内の配置は次で固定する。

```text
local-event-store/
├── authority-v1.json
└── generations/
    └── <generation-id>.sqlite3
```

`authority-v1.json`は`Legacy { source_generation_id, migration: Option<{ migration_id, staging_generation_id }> } | Sqlite { generation_id, store_id, activated_migration_id }`のclosed unionである。既存app-data generationを`source_generation_id`に使い、完全なsource inventory hashはstaging DBへ保存する。`store_id`は`store_metadata`のimmutable IDと一致させ、DB content hashにはしない。owner-only permission、versioned envelope、checksumを持ち、temporary file write → file sync → rename → parent directory syncでCASする。pointerはmigration stagingのlocatorとcutoverだけに使い、通常batch commitには使わない。

SQLiteはbundled libraryを使い、最低version 3.45をcompile / startup checkする。connection設定は`journal_mode=WAL`、`synchronous=FULL`、`foreign_keys=ON`、`trusted_schema=OFF`、`busy_timeout=250ms`である。schema migrationはexclusive writer lock下、normal admission前に行う。application process内のwriter connectionは一つ、reader poolは最大4 connectionである。

#### Schema

schema version 1のtableとauthorityは次のとおり。

| Table | Primary / unique key | Purpose |
| --- | --- | --- |
| `store_metadata` | singleton | schema / generation / sequence / health |
| `logical_commits` | `commit_id`; unique idempotency tuple | payload binding、state、sequence range、result hash |
| `stream_heads` | `stream_id` | current stream version |
| `events` | `global_sequence`; unique `(stream_id, stream_sequence)`, `event_id` | immutable envelope |
| `operation_bindings` | principal + generation + kind + caller key | exact command replay / conflict |
| `caller_attempts` | principal + generation + kind + caller key | built-in Tauri callerのresponse-loss / UI-restart outbox |
| `operation_records` | kind + operation ID | immutable receiptとlatest status direct lookup |
| `session_projection` | session ID | bounded session / queue / lifecycle read model |
| `message_projection` | session ID + message ID | complete message / parts read model |
| `terminal_records` | session ID + turn ID | terminal uniquenessとcomplete terminal result |
| `stop_resolutions` | Stop operation ID | Succeeded / Superseded result |
| `obligations` | obligation ID | pending / terminal durable work |
| `pending_obligations` | ordered key; unique obligation ID | startup / filtered recovery index |
| `recovery_action_attempts` | action ID | exact replay result |
| `shutdown_plans` | plan ID + epoch | plan root / phase / summary |
| `shutdown_targets` | plan ID + epoch + ordinal | bounded target detail |
| `shutdown_recovery_snapshots` | plan ID + epoch + partition + ordinal | frozen recovery detail |
| `shutdown_compact_archives` | plan ID + epoch | immutable Compacted summary |
| `local_store_migrations` | migration ID | source inventory / checkpoint / parity / phase |
| `legacy_raw_records` | migration ID + source ordinal | raw-preserved legacy bytes |

foreign keyとCHECK constraintでsequence正数、revision非負、closed tag、hash長、plan / epoch associationを検証する。public query用indexはoperation identity、terminal unique key、pending ordered key + owner / partition / shutdown association、shutdown plan / target、event stream / global sequenceに限定する。F8 / F10の追加read modelは後続schema migrationで追加できる。

#### Commit transaction

writerは次の順で`commit_batch`を実行する。

1. queue admission前にbatch件数、decoded bytes、identity、duplicate keyを検証する。
2. `BEGIN IMMEDIATE`を開始する。
3. idempotency tupleをpoint lookupする。same bindingはsaved resultを返し、different bindingはrollbackして`PayloadConflict`。
4. 全expected stream headとstate mutation revisionを検証する。不一致はrollbackしてtyped conflict。
5. `logical_commits(state='preparing')`をinsertし、eventsへ連続global / stream sequenceを割り当てる。
6. state mutation、direct index、projectionを同じtransactionで更新する。
7. event count、participant count、sequence range、result hashを検証し、logical commitを`sealed`へ更新する。
8. SQLite `COMMIT`後、別statementでidempotency keyをfresh readbackして`CommittedBatch`を返す。COMMIT開始からfresh readback完了までのerror / reply喪失はすべて同じcommit identityの`OutcomeUnknown`とする。
9. commit notificationをpublishする。publish失敗はcommit resultを変えない。

readerは`sealed` commitへjoinできるrowだけを見る。SQLite commit前failureは変更前へrollbackする。COMMIT / reply結果不明は`OutcomeUnknown`で、same commit identityの`logical_commits` / idempotency lookupによりCommittedかabsence確定へ収束する。COMMIT開始後のabsenceは、exclusive writer lockを再取得してWAL recoveryが終わるまで未commit根拠にしない。

writer queueはnormal lane 1024 requests / 64 MiB、critical terminal / Stop / shutdown lane 128 requests / 8 MiBを予約する。一batchは4096 events、8192 state mutations、decoded 16 MiB以下である。上限超過はwriter開始前`CapacityExceeded`で、critical laneをnormal backlogでstarveさせない。

#### Projection and bounded reads

transaction内projectionはeventと同じcommitで更新し、public queryはprojection tableをpoint / range lookupする。startup時の通常pathで全event replayしない。integrity checkまたは明示maintenance用rebuildは空のstaging projectionへglobal sequence順でreplayし、source head / count / hash parity後に同じSQLite transactionでactive projection generationを切り替える。

pending recovery first pageは`pending_obligations`のordered index、terminalはunique key、operation statusはdirect keyを使う。query plan snapshotをCIで固定し、`SCAN events`またはsession directory scanを含むplanを失敗させる。

watchはcommit後のin-process broadcastで`commit_id / max_global_sequence`だけを通知する。lagged subscriberはlast global sequenceから最大200 events / 4 MiBをreplayする。notificationをauthorityにしない。

#### Legacy to SQLite migration

`LocalStoreMigrationProjection`は`migration_id / phase / imported_source_count / total_source_count / read_only / safe_failure`を返す。全phaseで`read_only=true`、mutationは`MigrationInProgress`で拒否する。

1. `InspectingSource`: staging SQLite generationを先に作り、Legacy pointerの`migration`へmigration / generation IDをCASしてから、legacy session / workflow rootをstable path順で列挙する。relative path、size、mtime、SHA-256、record countをstaging DBのimmutable inventoryへ保存する。
2. `Importing`: 最大256 source recordsまたはdecoded 16 MiBを一batchとしてstaging SQLiteへ変換する。checkpointは次に処理するsource / record / substepを指す。
3. known eventはdomain eventへdecodeして新envelopeへ保存する。unknown additive recordは`legacy_raw_records`へ元bytesとsource metadataを保存する。required semantic不明、source drift、identity collisionは`MigrationBlocked`。
4. 未完了作業はidentity、exact payload / safe observation、owner revisionを証明できる場合だけobligationへ移す。証明できない作業は`Paused | Failed | ReconciliationRequired`でeffect 0件とし、推測でterminal化しない。
5. `Verifying`: record count、session / workflow public projection、terminal、permission、queue、known event result、operation / owner relation、pending work、shutdown detailをlegacy fixtureと照合する。
6. `Activating`: staging DBを`integrity_check`し、WAL checkpoint / fsync後にauthority pointerをCASする。結果不明はpointerをfresh readし、Legacyなら同checkpoint、SqliteならDB parity確認から再開する。
7. Sqlite authority確認後だけnormal admissionを開く。legacy filesは変更・削除せず残すが、runtime reader / writerは以後参照しない。

migration中quitはnormal shutdown planを作らない。Legacy pointerが指すstaging SQLiteへcaller binding、backend quit operation、`MigrationFlight` locator、current migration checkpointを同じtransactionで保存し、15秒以内にprocess exitする。次bootはLegacy pointerのmigration locatorから同じDBを開き、同じflightを`Exited | ReconciliationRequired`へ一方向に確定する。cutover後は同じDBがSqlite authorityになるため、known-operation queryは前後とも同じ`MigrationApplicationQuitProjection`を返す。

#### Shutdown detail compaction

terminal shutdown planのdetailは次の一transactionで`Available`から`Compacted`へ切り替える。

1. plan / target / fixed recovery snapshotを同じread snapshotで集計する。
2. identity、intent、terminal phase、counts、deadline、outcome、safe failure、source revision / hashを`shutdown_compact_archives`へinsertする。
3. planのdetails stateを`Compacted`へCASする。
4. COMMIT後のqueryはarchiveだけをauthorityにし、entries空、next cursorなし、exact detail queryは`DetailsCompacted`を返す。
5. obsolete target / snapshot rowはbackgroundで64 rows / 4 MiB / 50 msごとに削除する。削除途中でもqueryはAvailableへ戻らない。

新quit受理前に最大2 full detail setを検査する。必要なprior compactionをcommitできない間は`PreviousShutdownCompactionPending`とblocking projectionを返し、新plan / effectを作らない。

### UI/UX

- composer snapshotとcaller-generated operation IDを一attempt中固定し、Accepted receipt時だけそのsnapshotをclearする。
- `RejectedBeforeCommit | OutcomeUnknown | PayloadConflict`では入力を保持し、別identityで自動sendしない。
- Accepted後のFailed / ReconciliationRequiredは同じoperationのstatusとして表示し、入力を復元しない。
- pending recoveryはownerまたは`ClosedSession | ArchivedSession | UnownedRuntime` partitionへ表示し、backend提示actionだけを有効にする。
- migration中はlegacy dataをread-only表示し、progressとsafe failureをWorkspaceのblocking stateとして表示する。未実行nodeの表示規則は別Issueであり本画面へ混ぜない。
- view closeはbackend operationを作らない。session close、archive、backend switch、application quitは別actionとして表示する。
- shutdown / migration / feedbackはbackend projectionをmirrorし、frontendでterminal winner、retryability、Current(None)を推測しない。

### Algorithm

#### Normal send

1. identity / principal / exact payload / capacity / migration / shutdown admissionを検証する。
2. existing bindingをpoint lookupし、same payloadはsaved receipt、different payloadはPayloadConflict、別principalはNotFoundを返す。
3. current session revisionから`StartedTurn | Queued`を一度決める。
4. binding、human input event、message / turnまたはqueue identity、provider establish / turn execution obligationを一batchでcommitする。
5. commit確認後だけAccepted receiptを返し、provider effectをobligation identityで開始する。
6. effect後crashは同obligationをReconciliationRequiredとして回収し、自動で別effect identityを作らない。

#### Terminal closure

turn gateでterminal candidateをcompare-and-setし、winnerだけがfinal parts、assistant message、terminal event、session / permission / queue state、関連Stop resolutionを一batchへ入れる。unique terminal keyとexpected session / turn revisionを同時検証する。loserまたはold turn eventはno-op resultとして保存せず、既存winnerを返す。notification failureはterminalを取り消さない。

#### Stop deadline

Stop acceptanceはtarget / expected revision / queue pause / deadline permit / terminal obligationを先にcommitする。provider interruptを別taskで開始し、fake-clock T0+10秒でwinner未確定なら`Interrupted(Timeout)` terminal batchを試す。保存不能時はStop / capacity permitを保持してReconciliationRequiredへ進め、startup / manual recoveryが同じterminal identityをretryする。late provider resultはturn fenceで無効化する。

#### External effects and recovery

effect前にobligationを`EffectReserved`へcommitし、effect identityをprovider / workflow / OS portへ渡す。resultを確認できた場合だけowner state、obligation terminal、publicationを一batchで確定する。結果不明はobservationとcapabilityを保存してReconciliationRequiredとする。startupは`pending_obligations`だけを読む。

recovery actionはbackend発行action ID、origin revision、effect identityへ束縛する。Completed resultはoutcome / classification / resource revision / canonical result hash / safe resource viewを64 KiB以下で保存し、current resourceから再構築しない。`RetrySameEffect`は同じeffect identityだけを使い、`CancelIfSafe`は`ConfirmedNoEffect`を再検証できるtargetだけに提示する。

#### Session lifecycle

view closeはUI stateだけを変更する。normal close / archive / backend switchはoperation bindingとsession revision gateを使う。active close / open archiveは`SessionClosed` terminalとqueue pauseをterminal batchに含め、Idle close / archiveはsynthetic terminalを作らない。backend switchはIdleかつpending permission / recovery / provider operationなしだけを受理し、old runtime close結果確認前にnew backendを開始しない。10秒以内にCompletedまたはReconciliationRequiredへ進める。

#### Shutdown

first quitはcaller binding、plan identity、intent、T0、15秒deadline、最大4096 target、preexisting recovery snapshotを固定する。全target preparationをbounded pageでcommitした後、一つのtransactionでphaseをActivatedへ進める。Activated前failureはeffect 0件でabort可能、Activated後はabortせずT0+15秒に未完了targetをReconciliationRequiredとして同じplanへ残してexit / restartする。

全graceful surfaceは同じcoordinator ingressを呼ぶ。別request IDはfirst intentを変更せずcurrent flightへjoinする。previous-boot nonterminalまたは未解決shutdownがある場合は新flightを作らない。exit permitはplan / epoch / intent / terminal decisionへ一度だけ束縛し、native exit handlerはpermitなしexitをpreventする。

#### Migration

Database節の7 stepを一つのstate machineとして実装する。migration stateとcurrent source batch checkpointを同じcommitで更新する。authority pointer result不明をLegacy / Sqliteへ推測せずActivatingを維持し、次bootのfresh pointer / DB verificationで決着する。

#### F1b and fault matrix

Claude / Codex wire fixtureはproduction wire adaptor → `apply_runtime_event` → domain event → SQLite store → production projectorを通す。wire→domain eventとdomain event→public resultのgoldenを別testにし、test-only reducerでexpected stateを生成しない。

fault matrixはsend acceptance、permission、provider establish、streaming、terminal、Stop、recovery、session lifecycle、shutdown、migrationのtransaction開始前、participant write後、COMMIT前後、reply喪失、notification前、restartを網羅する。各caseで変更前または全確定後だけ、identity不変、external effect 0または1を検査する。

legacy migrationはsource 0 / 1 / 255 / 256 / 257件、16 MiB境界、unknown additive / required version、source drift、identity collision、pointer CAS前後を検査する。shutdownはtarget 4096 / 4097、page / byte、T0+13 / +15秒、detail Available→Compacted、64-row削除batchを検査する。

### Infra

新しいserver / remote serviceは追加しない。loopback WebSocketは既存local API、Bearer auth、shutdown lifecycleを共有する。

SQLite blocking callはtokio task上で直接行わず、専用single-writer workerとbounded reader poolへ送る。provider / workflow / OS effectをDB transaction中にawaitしない。writer worker停止、reply channel drop、process crashをfault injectionできるようにする。

test supportはtemporary app-data、bundled SQLite、fake clock、fault-injectable writer、recording provider / workflow / process-exit portを使う。実provider process、CLI、network、credentialをF1bで使わない。

telemetryはoperation kind、opaque correlation / commit identity、phase、duration、safe failure kind、queue depth、SQLite error classだけを記録する。message content、image bytes、permission payload、binding bytes、HMAC key、SQL parameter、filesystem pathを記録しない。

### Traceability

| Requirement | Design owner | Verification |
| --- | --- | --- |
| R-001–R-003 | Interface command rules、Algorithm Normal send、UI/UX | response喪失 / replay / principal / payload / composer matrix |
| R-004–R-005 | Commit ownership、Database Commit transaction、External effects | effect前後 / COMMIT前後 / restart fault matrix |
| R-006–R-007 | Data Model terminal unique record、Terminal closure | participant atomicityと全winner順 |
| R-008–R-009 | Stop bound / deadline、pending obligation | fake clock 10秒、storage failure、32 / 33件 |
| R-010 | pending index、bounded read、recovery algorithm | 201件、filter / cursor / snapshot、scan禁止plan |
| R-011 | feedback interface / UI | unreadable session、32 / 512件、revision / truncation |
| R-012 | F1b production composition | Claude / Codex golden、mutation test、external process 0 |
| R-013 | Store port、Schema、Commit transaction、Production cutover | multi-stream atomicity、idempotency、sequence、capacity、legacy access 0 |
| R-014 | Session lifecycle、close / quit decision table | surface全行、10秒、active / Idle / archive / switch |
| R-015–R-016 | quit interface、Shutdown algorithm / compaction | all ingress、15秒、4096 / 4097、Available→Compacted |
| R-017 | indexes、bounds、query plan gate | 10 / 1,000,000件、1000 sample、p95 / p99 |
| R-018 | Legacy migration、DTO encoding、migration-safe quit | checkpoint / pointer crash、surface parity、integer boundary |
| R-019 | compatibility fixtures | B-077の全anchorと既存golden |
| R-020 | terminal batch、Stop resolution | winner / superseded / persistence failure / capacity release |
| R-021 | recovery action attempt | five action kind、exact replay、stale / unavailable / ambiguous |
| R-022 | Canonical MessagePart、domain events、DTO ownership | single-definition check、legacy JSON / SQLite / presenter round-trip |

## Alternatives Considered

- 暫定file storeへatomic manifestを追加して後でSQLiteへ移す: 同じclosureを二度実装し、migration authorityと`phase0_*`命名を残すため不採用。
- F2を後回しにしてusecase eventをSQLite payloadにする: persistence DTOがusecase型へ固定され、F2時に全event変換をやり直すため不採用。
- SQLiteをinfrastructureに隠しgatewayへfile-store互換APIを残す: legacy layoutがportを支配し、multi-stream transactionを表現できないため不採用。
- eventだけを保存して起動時に全履歴foldする: R-017のhistory independenceを満たさないため不採用。必要なdirect / bounded projectionを同transactionで更新する。
- projectionを別transactionで非同期更新する: commit直後にpartial public stateを作るため不採用。
- provider effect後にobligationを保存する: effect直後crashを同じidentityで回収できないため不採用。
- dual write / record単位fallback: authorityが二つになりparity failureを隠すため不採用。
- public Prepare / Commit / Cancel send:既存single-send contractを不要に広げるため不採用。
- managed backup / restoreを同時実装する: #1499のclosure成立に不要で公開data lifecycle判断を増やすため不採用。

## Cross-cutting concerns

- Crash consistency: SQLite COMMITとauthority pointer CASだけをcommit根拠にする。notification、WAL file存在、worker future完了を根拠にしない。
- Idempotency: same key / same canonical bindingはsaved result、different bindingはPayloadConflict。absenceはWAL recovery / writer exclusion後だけ未commit根拠にする。
- Security: app-data / DB / keyはowner-only。bindingはconstant-time比較し、content、secret、path、raw SQLをpublic failure / telemetryへ出さない。
- Concurrency: operation gate、terminal gate、obligation claim、shutdown epoch、migration lockはscopeを分ける。external effect中にDB transactionやsession lockを保持しない。
- Compatibility: legacy JSONとpublic DTOのknown shapeをgolden固定する。migration中はread-only、cutover後はSQLite-onlyである。
- Performance: pending first 200 p95 50 ms、identity query p95 20 ms / p99 50 ms、大規模fixture p95比1.25以下。query planに全event scanを認めない。
- Schema evolution: additive stored payloadはraw-preserve、required semantic changeは新payload versionとtotal converterを必須にする。destructive migrationはstaging generation / parity / pointer cutoverを使う。
- Retention: operation receipt、completed recovery action、terminal、shutdown compact archiveは本Issueでtime-based GCしない。event / privacy retentionは別Issueで決定する。

## Risks

- legacy fixtureには現行codecの暗黙defaultがある。`StoredMessagePartV1` / legacy event DTOを先にgolden固定し、domain統合とmigrationを同じfixtureで検査する。
- SQLite COMMIT errorは実際のcommit有無を表さない場合がある。`OutcomeUnknown`とidempotency readbackを必須にし、別identity retryを禁止する。
- authority pointerはSQLite外の唯一のcutover authorityである。通常mutationには使わず、CAS / fsync / checksum / fresh readbackのfault testを独立させる。
- storage全損中はAccepted Stop terminalを10秒以内に保存できない。Accepted factとcapacityを保持し、ReconciliationRequiredとしてIdle / queue drainを抑止する。
- 4096-target shutdownとmigration importは長時間になる。bounded batch、critical writer lane、15秒decision deadlineを分離し、進捗保存とeffect開始を混同しない。
- graceful eventを配信しないhard kill / power lossはcoordinatorを通らない。次bootのSQLite WAL recovery、pending obligation、shutdown / migration readbackで回収する。
