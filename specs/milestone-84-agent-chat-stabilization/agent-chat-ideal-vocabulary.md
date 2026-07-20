# Agent チャット正規化語彙・データ構造の理想形

作成日: 2026-07-07
更新日: 2026-07-19

milestone 84「Agentチャット安定化」のドキュメント群:

- [agent-chat-instability-audit.md](agent-chat-instability-audit.md) — 問題点インベントリ（全 66 件、要求リスト）
- **agent-chat-ideal-vocabulary.md（本書）** — 正規化語彙・データ構造の理想形
- [agent-chat-ideal-lifecycle.md](agent-chat-ideal-lifecycle.md) — ライフサイクルの理想形（不変条件）
- [agent-chat-ideal-presentation.md](agent-chat-ideal-presentation.md) — UI 表示の理想形
- [d3-durable-event-store-design.md](d3-durable-event-store-design.md) — local atomic event store の物理設計 gate
- [close-quit-decision-table.md](close-quit-decision-table.md) — close / quit surface の正本

本書は「Claude / Codex から届く事象を、何という語彙に正規化するか」の正本を定義する。監査で確定した dropped / divergent 問題群の解消先であり、ライフサイクル・表示の 2 文書はこの語彙を前提とする。問題 ID（CL-x 等）は監査ドキュメントを参照。

## 設計原則

- **V-P1 (no-silent-drop / fail-closed control plane)**: wire 層は届いたメッセージを無言破棄してはならない。parse可能かつcontent-planeと分類でき、projection先だけが未対応の既知・未知message/partは、payload長・digest・分類・上限付きredacted sampleだけを`Notice(kind=UnsupportedMessage)`と構造化ログへdurable記録してsessionを継続できる。既知variantのtyped decode failure、content/controlを分類できないmalformed frame、size上限超過、およびpermission / configuration ack、Goal、provider mode / reviewer、turn completion、応答必須server requestなどcontrol-planeの未知・decode失敗も同じbounded summaryと取得済みの部分protocol identityを`ProtocolIncompatible`に記録し、新規turnをfail-closedでblockする。完全evidenceが必要な場合だけ、secret plaintextをredactしたpayloadを暗号化・per-session quota・object size上限・TTL・参照認可付きevidence storeへ保存し、`ProviderEvidenceRef`で参照する。active turn中ならpending permissionをcancelしprovider interrupt後に`Interrupted(ProtocolIncompatible)`で必ずfinalizeして、spinner/dialogを残さない。「捨てる」は明示的な設計判断としてのみ許され、本書に記録する。
- **V-P2 (parity)**: 同一概念は backend に依らず同一の語彙要素へ写像する。backend 固有の概念（Codex の item 種別等）は新しい part 種を増やすのではなく、既存語彙の kind / フィールドへ写像する。
- **V-P3 (durable 表現可能性)**: UI に表示されるべき全情報は、この語彙（part / turn outcome / usage / notice）で表現でき、durable event として記録できなければならない。transient にしか存在しない表示情報を作らない。
- **V-P4 (additive 進化)**: 永続化される語彙（durable event / read model）の変更は additive-only とし、既存セッションの読み込み互換を壊さない。
- **V-P5 (full-retention 回避)**: 語彙拡張はサマリ・認可付き期限参照（`ToolOutputRef` / `ProviderEvidenceRef`）・スナップショットで表現し、wire の生ペイロード全量やsecret plaintextをdurable event / 構造化ログへ恒久保存しない。
- **V-P6 (Rust-owned configuration)**: Agent の実行設定は Releash の Rust backend が正本を所有する。frontend は capability と確定済み設定の mirror に留め、adapter の受理前に確定表示しない。turn 送信時の frontend 値で設定を上書きしない。
- **V-P7 (lossless persisted integer domain)**: Phase 0 record、backup / restore、F3 import、SQLite `INTEGER`へ保存または保存済み値のguardに使うRust `u64`は、zero-based count / index / ordinal / offsetとAbsentを表すexpected revisionだけ`0..=i64::MAX`、epoch / revision / sequence / claim generation等の1始まりfieldは`1..=i64::MAX`へ制限する。wire requestの対応fieldが`i64::MAX + 1`以上ならRust usecase開始前にtyped `InvalidRequest`、保存済み / import値ならtyped integrity failureまたはscope quarantine、current `i64::MAX`から次値を割り当てるmutationはeffect開始前にtyped `CapacityExceeded`とし、signed cast、wrap、clamp、負値化で続行しない。Phase 0 / F3は同じvalidatorを使い、SQLite signed 64-bitとの往復をlosslessにする。generic transaction-ID numeric codec自体はidentity preimageのbyte contractとして`0..=u64::MAX`をencodeでき、`u64::MAX` known-answerを維持するが、persisted semantic fieldのcallerはcodec呼出前に上記domainを検証するため、このcodec fixtureを`u64::MAX`の保存受理根拠にしない。
- **V-P8 (lossless public integer encoding)**: Tauri / WebSocketの公開request / resultでRust `u64`に写像するepoch / revision / sequence / ordinal / count / offset等のsemantic fieldはJSON numberを使わず、`0`または先頭ゼロのないASCII canonical decimal stringとしてencodeする。public maximumは`9223372036854775807`である。zero-based count / index / ordinal / offsetとAbsent expected revisionは`0`を受理し、epoch / existing revision / sequence等のone-based fieldは`0`を`InvalidRequest`にする。JSON number、negative、leading zero、`+`、exponent、前後空白、`9223372036854775808`以上も`InvalidRequest`でstate / identity / effectを変えない。current maximumから次値を必要とするmutationは`CapacityExceeded`で既存値を維持する。bounded transport controlの`limit: u16` / `max_bytes: u32`はJSON nonnegative integer、shutdown exit codeの`i32`はJSON signed integerとし、canonical decimal stringへ変換しない。各型 / routeの範囲外、fraction、wrong representationは`InvalidRequest`でstate / effect 0件とする。presenterは同じ値を両surfaceへ返し、frontend / protocol adapterはnumber conversion、rounding、wrap、clampを行わずopaqueに往復する。

## 現行語彙と不足の対応

現行の正本（調査時点）:

| 語彙 | 定義場所 | 不足（問題 ID） |
|---|---|---|
| `AgentRuntimeEvent` | `domain/agent_session/gateway.rs:55` | `BackendSessionCleared` 未配線（SD-1）。それ以外の種類は充足 |
| `MessagePart` | `domain/agent_session/entities/message_part.rs:8` ＋ `usecase/agent_session/session/mod.rs:169` に**二重定義**（ST-6） | tool の状態・種別・exit code・image 出力（RG-4/RG-8/CL-6/RG-7）、通知語彙（CX-7/RG-6/CL-5）、エラーの retry 表現（CX-8） |
| `TurnResult` | `domain/agent_session/entities/turn.rs` | `TurnStopReason` が `Refusal` のみで実質未配線（CL-3/CL-4/RG-3）。cost / duration / num_turns 無し（RG-9）。`Failed.error` が非構造化 String（RT-5）。`Interrupted` に token_usage 無し |
| `TokenUsage` | `domain/agent_session/entities/turn.rs:37` | cache 系・context window 上限・cost 無し（RG-9/FE-4） |
| `PermissionRequest` | `domain/agent_session/entities/permission_request.rs` | `PermissionQuestion` に id / is_secret / is_other 無し（CX-1）。整形表示情報無し（SD-6）。決定の実効性表現無し（CL-1）。`Cancelled` が dead code |
| `TodoListItem` | `domain/agent_session/value_objects/todo_list_item.rs` | `completed: bool` のみ。in_progress / priority 無し（RG-5） |
| `SystemNotificationType` | `domain/agent_session/value_objects/system_notification_type.rs` | `Compaction` のみ。運用系通知の受け皿無し（CX-7/RG-6/CL-5） |
| `AgentSessionEvent`（現行legacy persistence schema） | `usecase/agent_session/event_log/events.rs:104` | serde・usecase表示型・`serde_json::Value`へ依存し、新しいdomain eventとして流用できない。`TurnTokenUsage`もinput/outputのみ |
| Agent 実行設定 | `PermissionMode { Ask, Edit, Full }` と `plan_mode: bool` が分離 | Goal / Reasoning effort が無く、mode の provider 写像・更新確定・永続化が一つの設定として扱われていない（#1445〜#1451） |

## 理想形

### 1. MessagePart（統一・拡張）

**V-D1**: `MessagePart` は domain の単一定義を正本とし、usecase 側の重複 enum は廃止する（adaptor/presenter で protocol / frontend DTO へ写像する）。解消: ST-6。

```rust
pub enum MessagePart {
    Text {
        content: String,
        parent_tool_use_id: Option<String>,
    },
    Thinking {
        content: String,
        redacted: bool,                     // Claude redacted_thinking（CL 付録A）
        parent_tool_use_id: Option<String>,
    },
    ToolCall(ToolCall),                     // V-D2: ToolUse + ToolResult を統合
    Error {
        content: String,
        kind: ErrorKind,                    // V-D5
        retryable: bool,                    // CX-8: willRetry
        resolved: bool,                     // 自動リトライで回復したら true に更新
        parent_tool_use_id: Option<String>,
    },
    Permission { request: PermissionRequest },
    TaskStatus { /* 現行維持 */ },
    TodoListSnapshot { items: Vec<TodoListItem> },   // V-D3 で item 拡張
    Notice(Notice),                         // V-D4: SystemNotification の一般化
    Image { data: String, media_type: String },      // 配線して dead code 解消
    ImageRef { attachment: Attachment },
}
```

### 2. ToolCall（単一 part への統合）

**V-D2**: 現行の `ToolUse` / `ToolResult` 2-part 構成を、単一の `ToolCall` part の状態遷移モデルに統合する。

```rust
pub struct ToolCall {
    pub id: String,                  // tool_use_id / codex item id
    pub tool_name: String,           // 表示名（Bash, Edit, mcp__x__y, ...）
    pub kind: ToolKind,              // 表示・集計用の分類
    pub input: JsonPayload,
    pub status: ToolCallStatus,
    pub output: Vec<ToolOutputBlock>,   // text / image 混在（CL-6/RG-7）
    pub content_ref: Option<ToolOutputRef>,      // 大容量出力は参照（V-P5、現行踏襲）
    pub summary: Option<ToolOutputSummary>,
    pub exit_code: Option<i32>,      // RG-8
    pub duration_ms: Option<u64>,
    pub parent_tool_use_id: Option<String>,
}

pub enum ToolKind {
    Command,        // Bash / codex commandExecution
    FileEdit,       // Edit/Write / codex fileChange
    FileRead,
    Search,         // Grep/Glob / codex webSearch は WebFetch と区別
    WebFetch,
    WebSearch,      // CX-11: query / 結果を input / output に保持
    Mcp,            // mcpToolCall（CL-5 の status は Notice 側）
    Task,           // subagent / collabAgentToolCall（CX-10）
    Todo,
    Image,          // imageGeneration / imageView（CX-10）
    Review,         // enteredReviewMode / exitedReviewMode（CX-10）
    Other,
}

pub enum ToolCallStatus {
    Pending,        // 承認待ち（permission と連動）
    Running,        // in-flight。streaming 出力はここに追記（SD-5 の是正）
    Succeeded,
    Failed,
    Denied,         // ユーザー/ルールによる拒否（RG-4: Failed と区別）
    TimedOut,
    Interrupted,    // turn 中断による打ち切り（RG-4）
}

pub enum ToolOutputBlock {
    Text { content: String },
    Image { data: String, media_type: String },  // または ImageRef
}
```

- in-flight 判定は `status == Running` で行う。Codex `outputDelta` は `Running` のまま `output` へ追記し、`ToolResult` 相当への変換で in-flight 判定を壊さない（SD-5）。
- fileChange は承認前 `Pending`、実行中 `Running`、適用後 `Succeeded` と遷移し、開始時点で完了済み diff として描画される問題（SD-5）を解消する。
- denied / timed_out / interrupted は `is_error: bool` に潰さない（RG-4）。既存の `is_error` は写像で `Failed` 系に吸収する。
- **代替案**: 現行 2-part 構成を維持し `ToolUse` に status を足す案。merge ロジック（`merge_part`）の変更が小さい利点はあるが、「開始と結果が別 part」である限り in-flight 判定と状態表示が二箇所に分かれ、SD-5/RG-4 の根本原因が残るため不採用。参照実装（ACP `tool_call`+`tool_call_update`、vibe-kanban `NormalizedEntry`）はいずれも単一エンティティ＋更新モデル。
- durable event は既存の `ToolCallStarted / ToolCallSucceeded / ToolCallFailed / ToolResultRecorded` を残したまま `ToolCallStatusChanged { status, exit_code, at }` を追加し、projector が単一 `ToolCall` に畳む（V-P4）。

### 3. TodoListItem（進行状態の一級化）

**V-D3**: 解消: CX-5 / RG-2 / RG-5。

```rust
pub struct TodoListItem {
    pub text: String,
    pub status: TodoStatus,          // completed: bool を置換（serde 互換は写像で吸収）
    pub priority: Option<TodoPriority>,
}
pub enum TodoStatus { Pending, InProgress, Completed }
pub enum TodoPriority { High, Medium, Low }
```

- Claude `TodoWrite` と Codex `turn/plan/updated`（および plan item）の**両方**をこの語彙へ写像する。ACP の plan entry（content / priority / status）と同型。
- 永続互換: 旧 `completed: bool` は読み込み時に `Completed / Pending` へ写像する。

### 4. Notice（運用系通知の受け皿）

**V-D4**: `SystemNotification`（現行 Compaction 専用）を `Notice` に一般化する。解消: CX-7 / RG-6 / CL-5 / SD-7 / CX-2 の可視化・V-P1 の着地先。

```rust
pub struct Notice {
    pub id: String,
    pub level: NoticeLevel,          // Info / Warning / Error
    pub kind: NoticeKind,
    pub label: BoundedNoticeText,    // 一覧表示用の安全な短文
    pub detail: Option<BoundedNoticeText>, // 展開表示用
    pub status: Option<NoticeStatus>,   // InProgress / Completed / Failed（compaction 等の進行型）
    pub evidence_ref: Option<ProviderEvidenceRef>,
}

pub struct BoundedNoticeText {
    pub value: String,
    pub truncated: bool,
    pub original_bytes: Option<u64>,
    pub digest: Option<String>,
    pub correlation_id: Option<String>,
}

pub enum NoticeKind {
    Compaction,              // 既存を吸収。Failed も表現可能に（SD-7）
    ModelRerouted,           // CX-7
    ConfigWarning,           // CX-7
    Deprecation,             // CX-7
    GuardianWarning,         // CX-7
    RateLimit,               // account/rateLimits/updated（RG-6）
    McpServerStatus,         // CL-5: system/init の mcp_servers 接続失敗等
    UnsupportedMessage,      // V-P1: 変換先の無い既知/未知メッセージの着地先
    ProtocolIncompatible,    // V-P1: control-plane schema drift。session を fail-closed にする
    OversizeDropped,         // framingでcontent-planeと確定できる超過だけ。未分類はProtocolIncompatible
    PersistFailure,          // lifecycle I8: 永続化失敗の可視化
    Diagnostic,              // stall 診断等
}
```

- Notice は transcript 上の part として durable 化する（表示先の振り分け — inline / banner / badge — は presentation 文書で定義）。
- **判断**: session-scoped な別ストリームではなく part として持つ。理由: read model 一本で live / reload 等価（P 原則）を保て、発生時点の文脈（どの turn で何の直後か）が残る。rate limit のような「最新値だけ意味がある」ものは read model 側で latest を導出する。
- Noticeのkind、level、安全なlabel/detail、redaction、boundsはRust converter/usecaseが所有する。frontendからraw error/message付きのgeneric Notice mutationを受けない。
- size上限超過はUTF-8安全な`BoundedNoticeText`へ縮約し、`truncated/original_bytes/digest/correlation_id`で欠落を明示する。無言dropやraw provider/storage errorの表示を禁止する。
- retention/capacityはsession lifecycleに結合し、capacity pressureで別sessionのactive Noticeをevictしない。受理不能ならoriginating operationへtyped failureを返す。

#### SessionOperationFeedback（transient command feedback）

state transition前に失敗したsession操作のfeedbackは、履歴語彙であるdurable Noticeへ偽装しない。Rust-owned command/usecaseが次のtyped snapshotを生成し、frontendはsession単位のmirrorと明示dismissだけを行う。

```rust
pub struct SafeOperationFailure {
    pub kind: SessionOperationFailureKind,
    pub retryable: bool,
    pub label: BoundedNoticeText,          // UTF-8 <= 160 bytes
    pub detail: Option<BoundedNoticeText>, // UTF-8 <= 2_048 bytes
    pub correlation_id: String,            // 1..=128 bytes, [A-Za-z0-9._:-]
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

pub struct SessionOperationFailureFeedback {
    pub feedback_id: String,
    pub attempt_id: String,
    pub session_id: String,
    pub operation: SessionOperationKind,
    pub failure: SafeOperationFailure,
    pub available_actions: Vec<SessionOperationFeedbackAction>,
    pub revision: u64,
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

pub struct SessionOperationSuccess {
    pub attempt_id: String,
    pub session_id: String,
    pub operation: SessionOperationKind,
    pub resolves_feedback_id: Option<String>,
}

pub struct SessionOperationFeedbackSnapshot {
    pub session_id: String,
    pub entries: Vec<SessionOperationFailureFeedback>, // unresolved failures, issued order, 1 page max 32
    pub next_cursor: Option<String>,
    pub total_unresolved: u64,
}

pub struct GetSessionOperationFeedbackRequest {
    pub session_id: String,
    pub cursor: Option<String>,
    pub limit: u16, // 1..=32
}

pub struct DismissSessionOperationFeedbackCommand {
    pub session_id: String,
    pub feedback_id: String,
    pub expected_revision: u64,
}

pub struct RetrySessionOperationFeedbackResolutionCommand {
    pub session_id: String,
    pub feedback_id: String,
    pub expected_revision: u64,
    pub action_id: String,
}

pub enum SessionOperationFeedbackControlResult {
    Applied {
        snapshot: SessionOperationFeedbackSnapshot,
    },
    Rejected {
        rejection: SessionOperationFeedbackControlRejection,
    },
    Failed {
        failure: SafeOperationFailure,
    },
}

pub enum SessionOperationFeedbackControlRejection {
    NotFound,
    RevisionConflict { current_revision: u64 },
    ActionUnavailable,
}
```

`SafeOperationFailure`はoperation / recovery / shutdown projectionに保存するsafe failureの唯一の正本である。`BoundedNoticeText`の`value / truncated / original_bytes / digest`だけを文面へ使い、nested `correlation_id`はNone、failure identityはtop-level `correlation_id`だけに置く。Rust converterがclosed kind、retryable、bounded文面、available actionsを決め、frontendは分類・再試行可否・actionを生成しない。path、secret、raw SQL、provider payload、raw source errorは公開、durable safe result、hash preimageのいずれにも入れない。`SafeOperationFailureV1`は5 fieldと上記exact 19 tagをfield-for-fieldに写し、u64 semantic fieldはcanonical decimal stringとする。未知tag、上限超過、top-level / nested correlation identityの二重化は拒否する。

failure envelopeもclosedである。`StopCapacityExceeded`はStop受理前result、`ShutdownAuthorityMismatch`はshutdown projection、`TargetRevisionChanged / OwnerRevisionChanged / RuntimeGenerationChanged / InvalidEffectIntent`は対象pending resource、`PreviousShutdownReconciliationRequired / PreviousShutdownCompactionPending`はquit受理前resultにだけ現れる。`ExitCoupledOutcomeUnknown`は`SafeEffectObservation`専用でありfailure kindへ入れない。durable identityを持つfailureはembedded result / projectionへ一度だけ写し、同じfailureをtransport errorにも複製しない。`PayloadConflict`はこの19種へ追加せず、同じcaller identityへ異なるexact payloadが提示されたことを示すdeterministic pre-commit typed application errorとして返す。validation / admission / bounded query failureだけをdirect typed errorへ写し、安全に分類できないraw failureだけを`Internal { correlation_id }`とする。

gateway / storage / protocol adaptorが使うprivate error classification supersetとPayloadConflict identityは次の一つだけである。

```rust
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
```

このsupersetはprivate infrastructure classificationだけに使い、usecase methodのreturn型または公開error型にはしない。各usecase methodは次表をdeclarative schemaとして生成したendpointごとのdistinct named enumを直接返し、表にないvariantを型として持たない。private分類からの変換も同じmatrixから生成してrow内variantをtotal matchし、row外分類は契約違反としてoutermost boundaryでcorrelation ID付き`Internal`へ閉じる防御境界に限る。`StorageUnavailable`はbounded failureを必須とし、`Internal`はcorrelation IDだけを持つ。result内のRejected / Failed / OutcomeUnknown / ReconciliationRequiredをdirect errorへ複製しない。

| Endpoint / public error type | Exact variants |
| --- | --- |
| send_agent_message / SendAgentMessageApplicationError | InvalidRequest, PayloadConflict(Send), CapacityExceeded, FeedbackCapacityExceeded, BootstrapInProgress, ShutdownInProgress, ResponseTooLarge, Internal |
| get_agent_send_operation / GetAgentSendOperationApplicationError | InvalidRequest, NotFound, QueryBusy, DeadlineExceeded, StorageUnavailable, Internal |
| stop_agent_session / StopAgentSessionApplicationError | InvalidRequest, PayloadConflict(Stop), FeedbackCapacityExceeded, BootstrapInProgress, ShutdownInProgress, Internal |
| get_stop_operation / GetStopOperationApplicationError | InvalidRequest, NotFound, QueryBusy, DeadlineExceeded, StorageUnavailable, Internal |
| request_session_lifecycle / RequestSessionLifecycleApplicationError | InvalidRequest, PayloadConflict(SessionLifecycle), FeedbackCapacityExceeded, BootstrapInProgress, ShutdownInProgress, Internal |
| get_session_lifecycle_operation / GetSessionLifecycleOperationApplicationError | InvalidRequest, NotFound, QueryBusy, DeadlineExceeded, StorageUnavailable, Internal |
| list_pending_agent_recovery / ListPendingAgentRecoveryApplicationError | InvalidRequest, CursorMismatch, CursorExpired, QueryBusy, DeadlineExceeded, ResponseTooLarge, StorageUnavailable, Internal |
| get_pending_recovery_snapshot / GetPendingRecoverySnapshotApplicationError | InvalidRequest, NotFound, SnapshotMismatch, CursorMismatch, CursorExpired, DetailsCompacted, QueryBusy, DeadlineExceeded, ResponseTooLarge, StorageUnavailable, Internal |
| resolve_pending_recovery_action / resolve_shutdown_target_action / ResolveRecoveryActionApplicationError | InvalidRequest, BootstrapInProgress, ShutdownInProgress, StorageUnavailable, Internal |
| get_recovery_action / GetRecoveryActionApplicationError | InvalidRequest, NotFound, QueryBusy, DeadlineExceeded, StorageUnavailable, Internal |
| get_phase0_bootstrap / GetPhase0BootstrapApplicationError | StorageUnavailable, Internal |
| get_application_shutdown / GetApplicationShutdownApplicationError | Internal |
| request_application_quit / RequestApplicationQuitApplicationError | InvalidRequest, PayloadConflict(ApplicationQuit), CapacityExceeded, ResponseTooLarge, Internal |
| get_application_quit_operation / GetApplicationQuitOperationApplicationError | InvalidRequest, NotFound, QueryBusy, DeadlineExceeded, StorageUnavailable, Internal |
| get_shutdown_plan / GetShutdownPlanApplicationError | InvalidRequest, NotFound, CursorMismatch, CursorExpired, QueryBusy, DeadlineExceeded, ResponseTooLarge, StorageUnavailable, Internal |
| get_session_operation_feedback / GetSessionOperationFeedbackApplicationError | InvalidRequest, CursorMismatch, CursorExpired, QueryBusy, DeadlineExceeded, ResponseTooLarge, StorageUnavailable, Internal |
| dismiss_session_operation_feedback / retry_session_operation_feedback_resolution / SessionOperationFeedbackControlApplicationError | InvalidRequest, StorageUnavailable, Internal |

`SessionOperationFailureFeedback`はworkflow/session stateのmutation authorityでもtranscriptでもなく、reload同値を要求するdurable Noticeとは保存規則を分ける。collectionには未解決failureだけを置き、successをentryとして保存しない。collectionは`feedback_id`をkeyにし、1 pageを32件、process全体の未解決entryを512件に制限する。未解決entryをcapacity都合でevict / coalesceしてはならず、resolvedまたはexpected revision付きdismiss済みのentryだけを削除できる。Loadを含むfeedbackを返し得るdomain read / mutation operationは開始前にfeedback slotを予約し、成功時に予約を解放する。512件上限で予約できなければ対象Session / Workflowへ作用する前に`FeedbackCapacityExceeded`とRust-owned available actionsを直接返し、mutationはexternal effect 0件にする。failure時は予約slotを新しいidentity-keyed entryとして確定する。`SessionOperationSuccess.resolves_feedback_id`が存在するentryと一致する場合だけ既存failureをclearできる。別session、同kindの別attempt、古いsuccessはcurrent failureをclearしない。dismissは`feedback_id + expected_revision`をCASする。

feedback collection自身を空けるためのget / expected-revision dismiss / resolution retryは512-slot admissionから除外するexempt control planeである。これらは新しいfeedback entryを作らず、capacity飽和時にも必ず呼べる。getは`GetSessionOperationFeedbackRequest { session_id, cursor, limit: 1..=32 }`からsnapshotを返す。dismiss成功とretry成功、retry再失敗はいずれも`SessionOperationFeedbackControlResult::Applied { snapshot }`として更新後snapshotを返す。dismiss成功は当該identityをresolvedにして未解決count / slotを1件減らし、retry再失敗は`feedback_id + expected_revision`をCASして同じfeedback IDのattempt / safe failure / actions / revisionを更新し未解決countを増やさない。unknown / stale / action不正は`Rejected { NotFound | RevisionConflict { current_revision } | ActionUnavailable }`としてstate / effect 0件、control自身のstorage failureは`Failed { failure }`として新feedbackを作らず返す。Tauriは`get_session_operation_feedback`、`dismiss_session_operation_feedback`、`retry_session_operation_feedback_resolution`、WebSocketは同じDTO / usecaseへ写像する`GetOperationFeedback / DismissOperationFeedback / RetryOperationFeedbackResolution`を公開し、同じsnapshot / rejection / failureを返す。

labelはUTF-8安全に160 bytes、detailは2048 bytesを上限とし、truncation、original byte数、digest、correlation IDを保持する。failure kind、安全な文面、capacity policyはRustが所有し、frontendがerror文字列を分類しない。Session close / archiveで未解決entryを自動削除せず、resolved / dismissed済みentryと不要になったowner indexだけをbounded cleanupする。永続化そのものの故障を知らせる`PersistFailure` transient bannerはlifecycle I8の例外規則に従う。

#### Send operation（command acceptance identity）

通常sendは既存の`send_agent_message`を使い、WebSocket outer request IDとは別のcaller指定stable operation identityをexact payloadと共に受け取る。Tauri / WebSocketは同じRust command / query serviceを通り、frontendはdomain decisionを所有しない。

```rust
pub struct SendOperationId(String);
// 1..=128 ASCII bytes, [A-Za-z0-9._:-]
// current-installation principal scope。WebSocket outer request_idとは別。

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

pub struct SendAgentMessageCommand {
    pub operation_id: SendOperationId,
    pub target: AgentSendTarget,
    pub content: String,
    pub images: Vec<SendImageInput>,
    pub mentions: Vec<MentionReference>,
    pub editor_context: Option<EditorContext>,
    pub active_turn_policy: ActiveTurnSendPolicy,
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
    ProviderStartReserved {
        obligation_id: String,
    },
    Running {
        turn_id: String,
    },
    ReconciliationRequired {
        failure: SafeOperationFailure,
    },
    Failed {
        failure: SafeOperationFailure,
    },
    Terminal {
        result: TurnResult,
    },
}

pub struct SendObligationStatusView {
    pub obligation_id: String,
    pub kind: ObligationKind,
    pub lifecycle: ObligationPublicLifecycle,
    pub safe_observation: Option<SafeEffectObservation>,
    pub safe_failure: Option<SafeOperationFailure>,
    pub available_actions: Vec<OperationAction>,
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

pub struct OperationAction {
    pub action_id: String,
    pub kind: RecoveryActionKind,
}

pub enum RecoveryActionKind {
    ReadAgain,
    RetrySameEffect,
    UseObservedResult,
    CancelIfSafe,
    KeepForManualResolution,
}

pub enum RecoveryActionResourceRef {
    Obligation { obligation_id: String },
    ShutdownTarget {
        plan_id: String,
        epoch: u64,
        target_key: String,
    },
}

pub enum RecoveryActionDecision {
    Attempt(RecoveryActionAttempt),
}

pub struct RecoveryActionAttempt {
    pub action_id: String,
    pub resource: RecoveryActionResourceRef,
    pub origin_revision: u64,
    pub origin_root_sha256: Option<[u8; 32]>,
    pub origin_state_sha256: [u8; 32],
    pub action_kind: RecoveryActionKind,
    pub status: RecoveryActionAttemptStatus,
    pub outcome: Option<RecoveryActionStoredOutcome>,
    pub classification: Option<RecoveryActionResultClassification>,
    pub serialized_safe_result: Option<Vec<u8>>,
    pub result_sha256: Option<[u8; 32]>,
    pub revision: u64,
}

pub enum RecoveryActionAttemptStatus {
    Prepared,
    EffectReserved,
    Completed,
    OutcomeUnknown {
        transaction_id: String,
        payload_sha256: [u8; 32],
    },
    ReconciliationRequired {
        failure: SafeOperationFailure,
    },
}

pub enum RecoveryActionStoredOutcome {
    Pending { obligation_id: String },
    Terminal { result: ObligationResult },
    Unchanged,
}
```

`AgentSendTarget`は現行`send_agent_message`入力の`chat_session_id / worktree_path / permission_mode / plan_mode / backend_id / model_id`へ総写像できる集約である。canonical commandは`target: AgentSendTarget`を一つだけ持ち、既存Tauri wire DTOだけが互換性のためこの6 fieldをflatに受け取ってadaptorで集約する。exact request bindingはapp-data generationごとexactly oneの`AgentOperationBindingKeyV1.hmac_sha256_key`を使い、`HMAC-SHA256(key, LP("send-operation-exact-request-binding/v1") || LP(principal_id) || LP(app_data_generation_id) || LP(operation_id) || LP(canonical_exact_command_bytes))`とする。`LP`はu32 BE byte lengthとraw bytesの連結、`canonical_exact_command_bytes`はtargetの6 fieldを固定順で展開した後にcontent、images、mentions、editor context、active-turn policyをcanonical encodeしたbytesであり、principal、operation ID、generation、WebSocket outer request ID、server生成IDを内部へ重複して含めない。fixed KATはkey bytes `00..1f`、principal `principal_1`、generation `app_1`、operation `op_1`、canonical command bytes `01020304`でpreimage 83 bytes、HMAC-SHA256 `74ad9247b5f271fc4e31f4fddf7c45cf35d413b1b35202d532095b163f9545db`である。Rustは最初のrequestでcurrent-installation principal、operation ID、exact payloadを不変に束縛する。same principal / operation ID / same payloadは保存済みdecisionをreplayし、same principal / same operation ID / different payloadは`SendAgentMessageResult`のvariantや`SafeOperationFailure`ではなく、`SendAgentMessageApplicationError::PayloadConflict { identity: PayloadConflictIdentity::Send { operation_id } }`として既存stateとexternal effectを変更せず返す。別principalが同じoperation IDをcommandまたはqueryに使った場合は存在を秘匿した`NotFound`とし、receipt、effect、新operationを0件にする。

canonical writer開始前のfailureは`RejectedBeforeCommit`であり、provider I/O、human message、turn / queue、durable operation viewを0件にする。writer開始後に保存結果を確認できない場合は`OutcomeUnknown { operation_id }`を返し、same operation queryまたはsame-payload retryで解決するまで別operationを生成しない。transport layerはpost-usecaseのOutcomeUnknownをgeneric errorへ複製しない。

Accepted receiptはimmutableであり、`input_ref`は受理済み入力authorityを指すbackend発行opaque identityである。clientはこれを生成・解析せず、receipt replayとoperation queryで同じ値を受け取る。provider進捗は`SendExecutionStatus`だけが変化する。provider establish待ちでもreceipt dispositionは`StartedTurn | Queued`のまま、statusだけをAwaitingProviderStartとする。acceptance後にprovider effectやcanonical terminalが未解決になった場合はtop-level Acceptedを維持し、`SendExecutionStatus::ReconciliationRequired`とRust-owned actionsを返す。operation全体のactionは`AgentSendOperationView.available_actions`、個別obligationのactionは各`SendObligationStatusView.available_actions`だけに置き、`SendExecutionStatus`へaction fieldを追加しない。composerはAccepted receiptでだけ対応snapshotをclearし、status failureやquery / emit failureで復活・自動再sendしない。

`get_agent_send_operation / GetOperation`はcurrent-installation principalと`SendOperationId`でdirect lookupする。既知operationは`AgentSendOperationView`、未知IDは`NotFound`を返す。`RejectedBeforeCommit`はdurable viewを作らない。`OutcomeUnknown`をNotFoundまたはAcceptedへ推測変換しない。

canonical型とwire DTOを分離する。`SendAgentMessageCommand / Result`、`AgentSendOperationView`、`RecoveryActionDecision / Receipt`がdomain / usecaseの正本であり、`*V1`はTauri / WebSocket adaptorのfield-for-field mappingだけに置く。adaptor DTOをdomain state、persistence record、lifecycle規則へ逆流させない。

recovery action commandはcurrent resource / shutdown detailsより先にaction IDのdurable decisionをlookupする。同じaction IDのCompletedは保存済みreceipt、classification、canonical safe resultをexact replayし、current stateの前進やdetail compactionを理由に再実行しない。nonterminalはsame attemptへjoinし、OutcomeUnknownはsame transactionをresolveする。decision Absentのfresh actionだけがcurrent revision / root / state guardを検証する。ReadAgain / RetrySameEffectはclaimとreservationをwrite-aheadしてからexternal I/Oへ進み、UseObservedResult / CancelIfSafe / KeepForManualResolutionはkind-specific side stateとCompleted receiptを同じclosureで確定する。

#### Stop / session lifecycle / application quit public contract

```rust
pub struct StopOperationId(String);

pub struct StopAgentSessionCommand {
    pub request_id: String,
    pub session_id: String,
    pub target_turn_id: String,
    pub expected_session_revision: u64,
}

pub struct StopAcceptanceReceipt {
    pub operation_id: StopOperationId,
    pub session_id: String,
    pub target_turn_id: String,
    pub accepted_at: String,
}

pub enum StopResult {
    Accepted { receipt: StopAcceptanceReceipt },
    RejectedBeforeAcceptance { failure: SafeOperationFailure },
    OutcomeUnknown { operation_id: StopOperationId },
}

pub enum StopResolutionResult {
    Succeeded,
    Superseded,
}

pub struct GetStopOperationRequest {
    pub operation_id: StopOperationId,
}

pub enum StopOperationView {
    Accepted {
        receipt: StopAcceptanceReceipt,
    },
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
    OutcomeUnknown {
        operation_id: StopOperationId,
    },
}

pub struct SessionLifecycleOperationId(String);

pub struct RequestSessionLifecycleCommand {
    pub request_id: String,
    pub session_id: String,
    pub expected_session_revision: u64,
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
    pub accepted_expected_session_revision: u64,
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
    RevisionConflict { current_revision: u64 },
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

pub enum ShutdownExitMode {
    Exit,
    Restart,
}

pub struct ShutdownExitIntent {
    pub mode: ShutdownExitMode,
    pub code: i32,
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

pub struct RequestApplicationQuitCommand {
    pub request_id: String,
    pub intent: ShutdownExitIntent,
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

pub struct GetApplicationQuitOperationRequest {
    pub operation_id: ApplicationQuitOperationId,
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
```

Stop / quit commandのcaller指定`request_id`は1..=128 ASCII bytesの`[A-Za-z0-9._:-]`で、current-installation principal scope内のstable identityである。空、129 bytes以上、または許可文字以外を含む値は`InvalidRequest`としてoperation / state / effectを0件にする。Stopのexact payloadは`session_id / target_turn_id / expected_session_revision`の3 field、quitのexact payloadは`ShutdownExitIntent.mode / code`である。同じprincipal / request ID / same exact payloadは同じoperationとresultをreplayし、same request IDでこのうち一fieldでも異なる場合はresult variantや`SafeOperationFailure`ではなく、対応する`request_id`を持つdeterministic pre-commit typed `PayloadConflict` application errorとしてeffect 0件で返す。Stopの別request ID / same unresolved session・turnは既存Stopへjoinし、後続requestのexpected revisionはそのcaller keyのexact bindingに保存するが、最初にAcceptedとなったStopのrevision guardを置換しない。quitの別request IDはcurrent flightへjoinし、first accepted `ShutdownExitIntent`を変更しない。backend発行のopaque `StopOperationId` / `ApplicationQuitOperationId`へcaller identityのvalidation規則を流用しない。

view close以外のsession close、open / closed archive、backend switchはTauri専用`request_session_lifecycle`へ正規化し、caller指定`request_id`はStop / quitと同じ文字・長さ制約を使う。exact bindingは`HMAC-SHA256(key, LP("session-lifecycle-exact-request-binding/v1") || LP(principal_id) || LP(app_data_generation_id) || LP(request_id) || LP(operation_id) || LP(canonical_lifecycle_command_bytes))`、inner bytesは`LP(session_id) || U64BE(expected_session_revision) || LP("close" | "archive-open" | "archive-closed" | "switch-backend") || LP("none" | "some") || [Someの場合だけLP(backend_id)]`である。principal、generation、request ID、operation IDをinner bytesへ重複させない。fixed KATはkey bytes `00..1f`、principal `principal_1`、generation `app_1`、request `lifecycle_req_1`、operation `lifecycle_op_1`、session `session_1`、revision `1`、action `close`、backend `none`でinner 38 bytes、full preimage 149 bytes、HMAC-SHA256 `b623c791f1a3f40579ba9713507ab507bdc844dee12d95e4408d673b17eb2217`である。

same principal / same request ID / same exact payloadは同じreceipt / state / outcomeをreplayし、same key / different payloadは`RequestSessionLifecycleApplicationError::PayloadConflict { identity: PayloadConflictIdentity::SessionLifecycle { request_id } }`としてoperation、queue、terminal、runtime effectを0件にする。別request ID / same unresolved session / same normalized actionは既存operationへjoinし、新bindingへ後続requestのexact payloadを保存するが、first accepted revision guard、action、deadlineを置換しない。SwitchBackendはbackend IDまでをnormalized actionに含める。別actionは`RejectedBeforeAcceptance { rejection: PendingOperation }`、同actionでもsession revisionが先に別操作で変わったfresh requestは`RevisionConflict`、不許可stateは`InvalidState`である。same-key raceはcaller-key winner一件、different-key raceはsession single-flight winner一件とし、loserはwinnerを再読込してsame actionならjoin、different actionならPendingOperationへ閉じる。Accepted後は10秒deadlineまで同じoperationを追跡し、未確定ならreceiptを維持した`ReconciliationRequired`とする。`get_session_lifecycle_operation`はcurrent-installation principalとbackend operation IDのdirect lookupだけを使い、response喪失、restart、同key retryで保存済みreceipt / state / outcomeをexact replayし、current sessionから再構築しない。`BackendSelected.runtime_started`は次sendまで常にfalseである。closed archiveではqueueを変更せず、`Archived { source_was_open: false, queue_paused }`のqueue_pausedは保存済みresulting projectionを返す。session authorizationはoperation lookupより先に検査し、unauthorized command / cross-principal queryは存在を秘匿した`NotFound`で、receipt、session state、effectを変更しない。`SessionLifecycleOperationId`はbackend発行opaque identityでありcaller request IDのvalidationを流用しない。

Stop terminalはinterruptがterminal winnerなら`StopResolutionResult::Succeeded`、normal completion / Fatal / SessionClosed / competing terminalが先勝した場合は`Superseded`である。どちらも同じ`TurnResult`を添え、Stopをprovider業務結果の成功へ読み替えない。

`CurrentApplicationShutdownResult`はnormal shutdown planだけを読む`Current(Option<ApplicationShutdownProjection>) | OutcomeUnknown { failure }`のclosed wrapperである。hash-validなcanonical plan rootの不在を同じbounded snapshotで証明した場合だけ`Current(None)`で、bootstrap-safe quitはnormal planを捏造せず`get_application_quit_operation`の`ApplicationQuitProjection::Bootstrap`から読む。shutdown authorityのcommit結果を確認できず、同じtransactionとplan identityへanchorできる場合だけembedded `OutcomeUnknown`を返す。hash-validなcomplete rootがexactly oneあり、そのrootが所有するplan ID / epoch / exit intentを一意に採用できる一方、pointer等の冗長semantic identityだけが矛盾する場合に限って`Current(Some(ReconciliationRequired))`と`ShutdownAuthorityMismatch`へ写す。storage read / decode / envelope・self-hash / pointer-to-root hash failure、required record欠損、state composite・activation lineage integrity failure、複数rootまたはunanchorable authorityでplan identityを一意にanchorできない場合はprojectionを合成せず、Tauriでは`GetApplicationShutdownApplicationError::Internal { correlation_id }`、WebSocketでは`AgentSessionWsErrorV1::Internal { correlation_id }`として返し、`Current(None)`や`OutcomeUnknown`へ隠さない。canonical `ShutdownExitIntent`がdomain / usecaseの正本で、`ShutdownExitIntentV1`はadaptor DTOのfield-for-field mappingだけである。

resource-isolated send input、operation / resource privacy purge、managed backup / restore、app-data reset、complete privacy authority reset / importは#1499のPhase 0 runtime contractに含めない。必要なdata lifecycleとphysical reclamationはD3 / F3後続設計で独立した公開要件とmigrationを先に確定する。


#### Durable external-effect obligation

Accepted receiptのdispositionは受理時に確定した`StartedTurn | Queued`をimmutableに保つ。ProviderEstablishが未解決でもreceiptは同じdispositionを返し、execution statusだけを`AwaitingProviderStart`とする。dependency success前にstatusを`ProviderStartReserved | Running`へ進めず、receipt dispositionからprovider startを推測しない。

全ProviderEstablish dependencyがdurable `Terminal(Succeeded)`になった後、provider-start obligation / dispatch fenceを先に確定した状態だけが`ProviderStartReserved`、その予約と同じeffectがstarted handleを取得した後だけが`Running`である。別の中間statusは存在せず、Queuedはqueue admission中だけを表す。

send operation command / queryのprincipalはWebSocketのcurrent installation bearerまたはtrusted Tauri IPC callerである。backendはcurrent-installation principalと`SendOperationId`のbindingを検証し、caller supplied ownerやWebSocket outer request IDをoperation authorityにしない。unauthorizedまたはcross-principal lookupではoperationの存在、payload、receipt、statusを漏らさずstate / effectを変更しない。

provider / workflowへ外部作用を起こす全経路は、effect後に回復情報を作るのではなく、stable identityを持つobligationを先にdurable化する。

```rust
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

pub enum PendingObligationState {
    Prepared,
    Pending,
    EffectReserved,
    ReconciliationRequired,
    Failed { failure: SafeOperationFailure },
}

pub struct ObligationClaim {
    pub obligation_id: String,
    pub claim_generation: u64,
    pub claim_token: String,
    pub owner_boot_id: String,
    pub lease_expires_at: String,
}

pub enum ObligationDispatchFenceV1 {
    Session {
        session_id: String,
        origin_owner_revision: u64,
        expected_session_revision: u64,
        command_generation: u64,
        expected_runtime_epoch: Option<u64>,
    },
    WorkflowExecution {
        workflow_execution_id: String,
        origin_owner_revision: u64,
        expected_workflow_revision: u64,
        executor_generation: u64,
    },
    OrphanRuntime {
        runtime_instance_id: String,
        runtime_epoch: u64,
        discovery_generation: u64,
    },
}

pub enum ObligationResult {
    Succeeded,
    CancelledBeforeEffect,
    Superseded,
    FailedTerminal,
}

// Query-only projection. Canonical storage uses ObligationStateRecordV1 below.
pub enum ObligationPublicLifecycle {
    Pending(PendingObligationPublicState),
    Terminal(ObligationResult),
}

pub enum PendingObligationPublicState {
    Prepared,
    Pending,
    EffectReserved,
    ReconciliationRequired,
    Failed,
}

pub enum ObligationOwner {
    Session { session_id: String },
    WorkflowExecution { workflow_execution_id: String },
    OrphanRuntime {
        runtime_instance_id: String,
        runtime_epoch: u64,
        discovery_generation: u64,
    },
}

pub struct ApplicationShutdownAssociation {
    pub plan_id: String,
    pub epoch: u64,
    pub target_key: String,
}

pub enum ObligationStateRecordV1 {
    Pending(PendingObligationRecordV1),
    Terminal(ObligationResultRecordV1),
}

pub struct PendingObligationRecordV1 {
    pub obligation_id: String,
    pub owner: ObligationOwner,
    pub shutdown_association: Option<ApplicationShutdownAssociation>,
    pub kind: ObligationKind,
    pub turn_id: Option<String>,
    pub operation_id: Option<SendOperationId>,
    pub semantic_correlation_sha256: [u8; 32],
    pub state: PendingObligationState,
    pub depends_on_obligation_ids: Vec<String>, // immutable fixed-kind DAG; ordered, max 3
    pub dependency_binding_sha256: Vec<[u8; 32]>, // same length/order as depends_on_obligation_ids
    pub claim_generation: u64,
    pub dispatch_fence: Option<ObligationDispatchFenceV1>,
    pub safe_observation: Option<SafeEffectObservation>,
    pub authoritative_proof: Option<AuthoritativeEffectProofRefV1>,
    pub reconciliation_reason: Option<SafeOperationFailure>,
    pub payload: ExternalEffectObligationPayload,
    pub revision: u64,
}

pub struct ObligationResultRecordV1 {
    pub obligation_id: String,
    pub owner: ObligationOwner,
    pub shutdown_association: Option<ApplicationShutdownAssociation>,
    pub kind: ObligationKind,
    pub semantic_correlation_sha256: [u8; 32],
    pub result: ObligationResult,
    pub safe_observation: Option<SafeEffectObservation>,
    pub safe_failure: Option<SafeOperationFailure>,
    pub completed_at: String,
    pub revision: u64,
}

pub enum ExternalEffectObligationPayload {
    TurnExecution {
        opaque_send_binding_sha256: [u8; 32],
        disposition: SendDisposition,
        effect: ExternalEffectIntent,
        assistant_message_id: String,
        durable_parts_cursor: u64,
        staged_final_parts_ref: Option<String>,
    },
    QueueExecution {
        queue_item_id: String,
        queue_execution_id: String,
        reserved_turn_id: String,
        opaque_send_binding_sha256: [u8; 32],
        runtime_guard: QueueRuntimeGuard,
        effect: ExternalEffectIntent,
    },
    BackendRecovery {
        recovery_id: String,
        publication_obligation_id: String,
        effect: ExternalEffectIntent,
    },
    RecoveryPublication {
        recovery_id: String,
        message_id: String,
        payload_ref: Option<String>,
    },
    TerminalCommit {
        terminal_id: String,
        target_session_revision: u64,
        target_runtime_epoch: u64,
        target_turn_id: String,
        requested_reason: TurnResult,
        absolute_deadline: String,
        stop_deadline_permit: StopDeadlinePermitRef,
        interrupt_effect: ExternalEffectIntent,
    },
    PermissionDelivery {
        response_id: String,
        request_id: String,
        redacted_semantic_response_sha256: [u8; 32],
        private_response_payload: PrivateEffectPayloadRef,
        redacted_response_summary: String,
        effect: ExternalEffectIntent,
    },
    ProviderEstablish {
        launch_or_recovery_id: String,
        effect: ExternalEffectIntent,
    },
    SessionClose {
        close_operation_id: String,
        target_runtime_epoch: Option<u64>,
        terminal_reason: Option<TurnResult>,
        shutdown_scope: Option<OwnedShutdownScopeRef>,
        effect: Option<ExternalEffectIntent>,
    },
    WorkflowShutdown {
        workflow_execution_id: String,
        expected_revision: u64,
        shutdown_scope: OwnedShutdownScopeRef,
        effect: ExternalEffectIntent,
    },
}

pub enum QueueRuntimeGuard {
    CurrentRuntime {
        runtime_instance_id: String,
        runtime_epoch: u64,
        effective_configuration_sha256: [u8; 32],
    },
    ProviderEstablishDependency {
        obligation_id: String,
        launch_or_recovery_id: String,
        effective_configuration_sha256: [u8; 32],
    },
}

pub struct PrivateEffectPayloadRef {
    pub blob_id: String,
    pub schema_version: u32,
    pub byte_len: u64,
    pub integrity_sha256: [u8; 32],
}

pub struct StopDeadlinePermitRef {
    pub admission_slot_id: String,
    pub deadline_service_permit_id: String,
    pub accepted_at: String,
    pub acceptance_committed_at: String,
    pub scheduled_force_at: String,
    pub terminal_commit_deadline_at: String,
}

pub enum ExternalEffectKind {
    TurnStart,
    QueueTurnStart,
    PermissionResponse,
    ProviderCreate,
    ProviderResume,
    ProviderInterrupt,
    RuntimeClose,
    WorkflowShutdown,
}

pub struct ExternalEffectIntent {
    pub effect_id: String,
    pub kind: ExternalEffectKind,
    pub owner: ExternalEffectOwnerRef,
    pub external_correlation_key: String,
    pub idempotency_key: Option<String>,
    pub resolution_capability: EffectResolutionCapability,
    pub process_exit_coupling: ProcessExitCoupling,
}

pub enum ExternalEffectOwnerRef {
    Provider { provider_id: String },
    Runtime { session_id: String, runtime_epoch: u64 },
    Workflow { workflow_execution_id: String },
    OrphanRuntime {
        runtime_instance_id: String,
        runtime_epoch: u64,
        discovery_generation: u64,
    },
}

pub enum EffectResolutionCapability {
    IdempotentRetry,
    AuthoritativeReadback,
    IdempotentRetryAndReadback,
    None,
}

pub enum ProcessExitCoupling {
    None,
    MayChangeOutcome,
}

pub struct AuthoritativeEffectProofRefV1 {
    pub proof_id: String,
    pub kind: AuthoritativeEffectProofKind,
    pub effect_id: String,
    pub external_correlation_sha256: [u8; 32],
    pub schema_version: u32,
    pub private_blob: PrivateEffectPayloadRef,
    pub safe_observation_sha256: [u8; 32],
    pub captured_at: String,
}

pub enum AuthoritativeEffectProofKind {
    EffectStarted,
    Succeeded,
    Terminal,
    AuthoritativeNotFound,
    ConfirmedNoEffect,
    Ambiguous,
}

pub enum SafeEffectObservation {
    ProviderObservation {
        observation_ref: String,
        proof_sha256: [u8; 32],
    },
    ConfirmedNoEffect { proof_sha256: [u8; 32] },
    ExitCoupledOutcomeUnknown { plan_id: String, epoch: u64 },
}

pub enum Phase0ClosureScope {
    Session { session_id: String },
    Workflow { workflow_execution_id: String },
    Application {
        shutdown_plan_id: String,
        shutdown_epoch: u64,
    },
}

pub struct ShutdownPlanRootV1 {
    pub plan_id: String,
    pub epoch: u64,
    pub exit_intent: ShutdownExitIntent,
    pub state: ShutdownPlanState,
    pub activation_ancestor_sha256: Option<[u8; 32]>,
    pub details: ShutdownPlanRootDetailsV1,
    pub target_count: u32, // max 4096
    pub pages_sha256: [u8; 32],
    pub preexisting_recovery_count: u64, // not included in target_count
    pub terminal_summary: Option<ShutdownSummary>,
    pub safe_failure: Option<SafeOperationFailure>, // Some only for Failed / ReconciliationRequired
    pub started_at: String,
    pub durability_cutoff_at: String,
    pub global_deadline_at: String,
    pub revision: u64,
}

pub enum ShutdownPlanRootDetailsV1 {
    Available {
        page_refs: Vec<ShutdownPreparedPageRef>, // max 32
        preexisting_recovery_snapshot: Option<PendingRecoveryInventorySnapshotRef>,
    },
    Compacted {
        archive_sha256: [u8; 32],
    },
}

pub struct LatestShutdownAttemptRefV1 {
    pub plan_id: String,
    pub epoch: u64,
    pub root_sha256: [u8; 32],
    pub state: ShutdownPlanState,
    pub coordinator_boot_id: String,
    pub pointer_revision: u64,
}

pub struct LatestActivatedShutdownPlanRefV1 {
    pub plan_id: String,
    pub epoch: u64,
    pub activated_root_sha256: [u8; 32],
    pub coordinator_boot_id: String,
    pub global_deadline_wall_ms: i64,
    pub pointer_revision: u64,
}

pub struct LatestRetiringShutdownPlanRefV1 {
    pub plan_id: String,
    pub epoch: u64,
    pub source_root_sha256: [u8; 32], // current guarded root; advances during compaction
    pub source_root_revision: u64,    // distinct from immutable archive source pair
    pub pointer_revision: u64,
}

pub struct PendingRecoveryInventorySnapshotRef {
    pub inventory_revision: u64,
    pub root_page_sha256: [u8; 32], // hash of the 3-tree root envelope, not a single primary page
    pub ranges: Vec<PendingRecoveryInventoryRangeRef>, // exactly 3 ordered partitions
    pub record_count: u64,                            // equals preexisting_recovery_count
    pub snapshot_sha256: [u8; 32],
}

pub struct PendingRecoveryInventoryRangeRef {
    pub partition: PendingRecoveryPartition,
    pub first_key: Option<String>,
    pub last_key: Option<String>,
    pub record_count: u64,
    pub range_sha256: [u8; 32],
}

pub struct ListPendingRecoveryRequest {
    pub filter: PendingRecoveryFilter,
    pub cursor: Option<String>,
    pub limit: u16, // 1..=200
}

pub enum PendingRecoveryFilter {
    All,
    Owner(ObligationOwner),
    Partition(PendingRecoveryPartition),
    ShutdownPlan { plan_id: String, epoch: u64 },
}

pub struct PendingRecoveryPage {
    pub inventory_revision: u64,
    pub entries: Vec<PendingRecoveryView>, // max 200 / encoded max 4 MiB
    pub next_cursor: Option<String>,
}

pub enum PendingRecoveryQueryError {
    InvalidRequest,
    CursorMismatch,
    CursorExpired,
    StorageUnavailable { failure: SafeOperationFailure },
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
    pub revision: u64,
}

pub struct GetPendingRecoverySnapshotRequest {
    pub plan_id: String,
    pub epoch: u64,
    pub snapshot: PendingRecoveryInventorySnapshotRef,
    pub partition: PendingRecoveryPartition,
    pub cursor: Option<String>,
    pub limit: u16, // 1..=200
}

pub struct ResolvePendingRecoveryActionRequest {
    pub obligation_id: String,
    pub expected_revision: u64,
    pub action_id: String,
}

pub struct ResolveShutdownTargetActionRequest {
    pub plan_id: String,
    pub epoch: u64,
    pub target_key: String,
    pub expected_plan_revision: u64,
    pub expected_root_sha256: [u8; 32],
    pub expected_target_state_sha256: [u8; 32],
    pub action_id: String,
}

pub struct GetRecoveryActionRequest { pub action_id: String }
pub enum RecoveryActionOperationView {
    InProgress { action_id: String },
    OutcomeUnknown { action_id: String },
    ReconciliationRequired { action_id: String, failure: SafeOperationFailure },
    Completed { result: RecoveryActionResult },
}

pub enum RecoveryActionCommandResult {
    Completed { result: RecoveryActionResult },
    InProgress { action_id: String },
    Rejected { rejection: RecoveryActionRejection },
    ActionOutcomeUnknown { action_id: String },
}

pub enum RecoveryActionRejection {
    NotFound,
    RevisionConflict { current_revision: u64 },
    ActionUnavailable,
    TargetRevisionChanged,
}

pub struct RecoveryActionResult {
    pub action_id: String,
    pub receipt: RecoveryActionReceipt,
    pub resource: RecoveryActionResourceView,
}

pub struct RecoveryActionReceipt {
    pub outcome: RecoveryActionOutcome,
    pub classification: RecoveryActionResultClassification,
    pub resource_revision: u64,
    pub canonical_result_sha256: [u8; 32],
}

pub enum RecoveryActionOutcome {
    Pending,
    Terminal,
    Unchanged,
}

pub enum RecoveryActionResultClassification {
    Pending,
    Succeeded,
    ConfirmedNoEffect,
    Ambiguous,
    CancelledBeforeEffect,
    Unchanged,
}

pub enum RecoveryActionResourceView {
    Pending(PendingRecoveryView),
    ShutdownTarget {
        plan: ApplicationShutdownProjection,
        target: ShutdownTargetView,
    },
}

pub struct PendingRecoverySnapshotPage {
    pub plan_id: String,
    pub epoch: u64,
    pub snapshot_sha256: [u8; 32],
    pub partition: PendingRecoveryPartition,
    pub entries: Vec<PendingRecoveryView>, // max 200 / decoded max 4 MiB
    pub next_cursor: Option<String>,
}

pub enum PendingRecoverySnapshotQueryError {
    InvalidRequest,
    SnapshotMismatch,
    CursorMismatch,
    CursorExpired,
    DetailsCompacted,
    QueryBusy,
    DeadlineExceeded,
    ResponseTooLarge,
    StorageUnavailable { failure: SafeOperationFailure },
    Internal { correlation_id: String },
}

pub enum PendingRecoveryPartition {
    ClosedSession,
    ArchivedSession,
    UnownedRuntime,
}

pub enum ShutdownPlanState {
    Preparing,
    Prepared,
    Activated,
    Quiescing,
    Completed,
    Failed,
    Cancelled,
    ReconciliationRequired,
}

pub struct ShutdownPreparedPageV1 {
    pub plan_id: String,
    pub epoch: u64,
    pub page_index: u32,
    pub first_target_ordinal: u32,
    pub targets: Vec<ShutdownPreparedTarget>, // max 128 joined domain entries; not the physical page body
    pub page_sha256: [u8; 32], // public physical-body hash; excluded from that hash input
}

pub struct ShutdownPreparedPageRef {
    pub page_index: u32,
    pub first_target_ordinal: u32,
    pub target_count: u32,
    pub encoded_bytes: u32,
    pub page_sha256: [u8; 32],
}

pub struct ShutdownPreparedTarget {
    pub target_key: String,
    pub target_ordinal: u32,
    pub target: ShutdownTargetSubjectView,
    pub expected_revision: u64,
    pub obligation_id: String,
    pub prepared_effect: PreparedShutdownEffect,
}

pub enum PreparedShutdownEffect {
    SessionClose {
        close_operation_id: String,
        shutdown_scope: Option<OwnedShutdownScopeRef>,
        effect: Option<ExternalEffectIntent>,
    },
    WorkflowShutdown {
        workflow_execution_id: String,
        shutdown_scope: OwnedShutdownScopeRef,
        effect: ExternalEffectIntent,
    },
}

pub enum OwnedShutdownScopeRef {
    RuntimeInstance {
        runtime_instance_id: String,
        runtime_epoch: u64,
    },
    WorkflowExecutorGroup {
        workflow_execution_id: String,
        executor_generation: u64,
    },
}

pub struct UnresolvedShutdownScopeFenceV1 {
    pub owner: ObligationOwner,
    pub shutdown_scope: OwnedShutdownScopeRef,
    pub obligation_id: String,
    pub plan_id: String,
    pub epoch: u64,
    pub obligation_revision: u64,
    pub obligation_payload_sha256: [u8; 32],
}

pub enum ShutdownTargetSubjectView {
    OpenSession {
        session_id: String,
        activity: OpenSessionShutdownActivity,
    },
    Workflow {
        workflow_execution_id: String,
    },
}

pub enum OpenSessionShutdownActivity {
    Active { turn_id: String },
    Idle,
}

pub enum ApplicationShutdownAction {
    RetryQuit,
}

pub enum CurrentApplicationShutdownResult {
    Current(Option<ApplicationShutdownProjection>),
    OutcomeUnknown { failure: SafeOperationFailure },
}

pub enum ExitPermitAuthorityV1 {
    Shutdown { plan_id: String, epoch: u64 },
    Bootstrap { bootstrap_id: String },
}

pub struct ExitPermitV1 {
    pub authority: ExitPermitAuthorityV1,
    pub exit_intent: ShutdownExitIntent,
}

pub struct ApplicationShutdownProjection {
    pub plan_id: String,
    pub epoch: u64,
    pub exit_intent: ShutdownExitIntent,
    pub phase: ApplicationShutdownPhase,
    pub details: ShutdownDetailsAvailability,
    pub target_count: u64,
    pub prepared_count: u64,
    pub effect_reserved_count: u64,
    pub terminal_count: u64,
    pub preexisting_recovery_count: u64,
    pub preexisting_recovery_snapshot: Option<PendingRecoveryInventorySnapshotRef>,
    pub durability_cutoff_at: String,
    pub global_deadline_at: String,
    pub failure: Option<SafeOperationFailure>,
    pub available_actions: Vec<ApplicationShutdownAction>,
}

pub struct ShutdownSummary {
    pub plan_id: String,
    pub epoch: u64,
    pub exit_intent: ShutdownExitIntent,
    pub outcome: ShutdownPublicOutcome,
    pub details: ShutdownDetailsAvailability,
    pub target_count: u64,
    pub completed_count: u64,
    pub unresolved_count: u64,
    pub preexisting_recovery_count: u64,
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
    pub plan_revision: u64,
    pub root_sha256: [u8; 32],
    pub projection: ApplicationShutdownProjection,
    pub summary: Option<ShutdownSummary>,
    pub entries: Vec<ShutdownTargetView>, // max 128 / encoded max 1 MiB
    pub next_cursor: Option<String>,
}

pub struct GetShutdownPlanRequest {
    pub plan_id: String,
    pub epoch: u64,
    pub cursor: Option<String>,
    pub limit: u16, // 1..=128
}

pub struct ShutdownTargetView {
    pub target_key: String,
    pub target_ordinal: u64,
    pub subject: ShutdownTargetSubjectView,
    pub state: ShutdownTargetPublicState,
    pub target_state_sha256: [u8; 32],
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

pub struct ShutdownPlanCompactProjectionV1 {
    pub phase: ApplicationShutdownPhase, // Completed | Failed | Cancelled only
    pub prepared_count: u64,
    pub effect_reserved_count: u64,
    pub terminal_count: u64,
    pub durability_cutoff_at: String,
    pub global_deadline_at: String,
}

pub struct ShutdownPlanCompactArchiveV1 {
    pub summary: ShutdownSummary, // details is always Compacted
    pub compact_projection: ShutdownPlanCompactProjectionV1,
    pub source_root_sha256: [u8; 32],
    pub source_root_revision: u64,
    pub activation_ancestor_sha256: Option<[u8; 32]>,
    pub pages_sha256: [u8; 32],
    pub preexisting_snapshot_sha256: Option<[u8; 32]>,
    pub archived_at: String,
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
```

`ShutdownTargetView.target_key`はplan内のtargetを指す唯一のpublic keyであり、`subject`は同じprepared authorityから復元したtyped owner identityである。`ShutdownTargetSubjectView::OpenSession`はActive / Idle activityを保持する。presenterはkeyからsubjectを推測せず、両者の保存済みparityを検証する。`ShutdownTargetSubjectViewV1`はadaptor DTOに限り、domain / usecase / persistenceの型として使わない。

`get_recovery_action / GetRecoveryAction`はidentity-only readでeffectを開始しない。current resourceより先にdirect decisionをlookupし、nonterminal Attemptを`InProgress | OutcomeUnknown | ReconciliationRequired`、Completedを保存済みcanonical resultから復元した`Completed`へ写す。decision Absentまたは未知IDは`NotFound`、malformed IDは`InvalidRequest`である。Completed resultはdetails compaction / restart / unrelated current state前進に依存せず元responseとbyte-equivalentに再取得できる。

`resolve_pending_recovery_action / resolve_shutdown_target_action`はclosed `RecoveryActionCommandResult`を返す。Completedはcanonical `RecoveryActionResult`、既存nonterminal attemptへのjoinはInProgress、fresh actionのguard拒否は`Rejected { NotFound | RevisionConflict | ActionUnavailable | TargetRevisionChanged }`、writer開始後のcommit結果不明だけは`ActionOutcomeUnknown { action_id }`である。`ActionOutcomeUnknown`をWebSocket / Tauri errorへ複製せず、same action queryまたはsame command retryで同decisionへ収束させる。

`RecoveryActionReceipt`の有効なoutcome / classification pairは`Pending + Pending`、`Pending + ConfirmedNoEffect`、`Pending + Ambiguous`、`Terminal + Succeeded`、`Terminal + CancelledBeforeEffect`、`Unchanged + Unchanged`の6組だけである。effect startまたはackをSucceededへ、AmbiguousをConfirmedNoEffectへ読み替えない。

`ShutdownPreparedPageV1`、`ShutdownPreparedPageRef`、`ShutdownPreparedTarget`は正本vocabularyとRust query serviceが使うjoined domain modelであり、Phase 0 physical envelopeと同名schemaとしてserializeしない。physical page bodyの`Phase0ShutdownPreparedPageV1`がcanonical encodeするのは`schema_version=1`、plan ID、epoch、u32 page index、`first_target_ordinal`と、ordered `Phase0ShutdownPreparedTargetRefV1 { target_key, target_ordinal, obligation_id, expected_owner_revision, target_authority_sha256 }`だけである。raw Session / Workflow owner、Active turn、scope、exact effect intent、provider proof、private payload、self hash、target count、first / last target key、encoded bytesをpageへ置かない。exact target / activity / prepared effectは1〜65,536 bytesの`Phase0ShutdownTargetAuthorityV1`が決定path `shutdown-target-authority/v1(plan_id, epoch, target_key)`で単独所有し、plan / epoch / page index / target key / ordinal / obligation ID / expected owner revisionとexact target / effectをcanonical encodeする。page refの`target_authority_sha256`はそのcanonical bytes全体のSHA-256である。

domainの`ShutdownPreparedTarget`はphysical refからauthorityを1 key point lookupし、digest、plan / epoch / page index / target key / ordinal / obligation ID / expected revisionをbyte一致させた後だけ復元するjoined valueである。authorityが存在し全field一致する場合だけpublic `ShutdownTargetView.subject`へ結合し、authority欠損または不一致をpage、memory、directory scanから補完せずCorruptとする。`Phase0ShutdownPreparedPageRefV1`とF3 page rowだけがbodyから導出したtarget count、first / last target key、encoded bytes、page SHA-256を持つ。domainの`page_sha256`とrefの同fieldは検証済みref / rowから復元し、hash inputはcanonical physical body全bytesだけとする。rootのdomain `page_refs: Vec<ShutdownPreparedPageRef>`とphysical ref列について、bodyとref / rowのplan、epoch、index、first ordinal、count、encoded bytes、first / last key、hashおよびpage間の連続性をdecode時に検証する。Phase 0→domain→Phase 0、domain→Phase 0→domain、F3 row→domainでpage refとauthorityのknown-answer byte parityを保証する。

shutdown target identityはbyte-exactに固定する。`target_key`はUTF-8 componentを`u32` big-endian byte length＋raw bytesでencodeした`["application-shutdown-target/v1", target tag, stable owner id]`の連結をSHA-256し、その32 bytesをbase64url no-padで表す。target tagはOpen Sessionが`open-session`、Workflowが`workflow-execution`であり、stable owner idはそれぞれsession ID、workflow execution IDである。active / Idleやturn IDはtarget keyへ含めず、同ownerを別targetにしない。`obligation_id`は`LP("application-shutdown-obligation/v1") || LP(plan_id) || epoch.to_be_bytes() || LP(target_key)`のSHA-256をbase64url no-padにした値である（`LP`は同じ`u32` big-endian length prefix）。

`ShutdownPreparedPageV1`のplan ID / epochはrootと一致し、page indexは0から連続、target ordinalはroot全体で0から`target_count - 1`まで重複なく連続し、各pageの`first_target_ordinal`は先頭entryおよび`ShutdownPreparedPageRef.first_target_ordinal`と一致する。target enumeration、pageのfirst / last、ordinal、index、public page、cursorは全てbase64url no-pad `target_key`のUTF-8 byte順に統一し、raw owner material順を別authorityにしない。target keyはstrictly increasingで、page境界でも重複・逆行を許さない。各physical refのtarget key / obligation IDは上記導出を再計算し、authorityがPresentならauthorityのidentity / digestともbyte一致させる。page refのindex / first ordinal / count / encoded bytes / hash、rootのplan / epoch / page count / target count / ordered page Merkle hashを全て検証する。欠落、重複、不連続、別plan / epoch、hash不一致はtyped corruptionとしてmutation admissionを閉じ、directory順やentry位置から補完しない。

target authorityのowner invariantもclosedである。Open Session authorityは`ObligationOwner::Session`のsession IDと一致し、`PreparedShutdownEffect::SessionClose`およびobligation payloadの`shutdown_scope / effect`はboth `Some`またはboth `None`だけを許す。`Some`ならscopeは保存済み`RuntimeInstance`、effect ownerは同Sessionの`Runtime`でruntime epochもscopeと一致する。`None / None`はlive runtimeが無いlocal closureだけで、`EffectReserved`へ進めない。Workflow authorityは`ObligationOwner::WorkflowExecution`、target / payloadのworkflow execution ID、`WorkflowExecutorGroup.workflow_execution_id`、`ExternalEffectOwnerRef::Workflow.workflow_execution_id`が全てbyte一致し、scope / effectを必須とする。authority build、Prepared root validation、reservation / action、Phase 0→F3 importの全境界でpoint lookupしたsame canonical bytesを再検証し、不一致をpage、owner、memoryから推測またはID差替えで修復しない。

`ExternalEffectIntent`のresolution capabilityは入力表示ではなくeffect前の安全性contractである。`external_correlation_key`は全intentでUTF-8 1..=512 bytesを必須とする。`IdempotentRetry`または`IdempotentRetryAndReadback`はstableな`idempotency_key=Some`（UTF-8 1..=512 bytes）を必須とし、`AuthoritativeReadback`または`IdempotentRetryAndReadback`はprovider-nativeのstable correlationから一意のauthorityを取得できるadapterだけが選べる。`None`は`idempotency_key=None`を必須としautomatic retry / readbackを許可せず、結果不明を`ReconciliationRequired`へ送る。common builder、Phase 0 / F3 import、claimの全境界で同じvalidatorを使い、capabilityとkey / adapter capabilityが一致しないintentはeffect 0件でrejectまたは既存stateをquarantineし、provider I/O後にcapabilityを格上げしない。

authoritative readback / observed result / no-effect判定は、provider / runtime / workflow adaptorが認証済みresponseまたはcanonical eventから作る`AuthoritativeEffectProofRefV1`だけをauthorityとする。clientはproof、provider outcome、evidence refを送信できずaction IDだけを選ぶ。private blobは1 byte以上1 MiB以下とし、schema version、byte length、content SHA-256、owner accessを検証する。resolverはproofを1 key direct lookupし、current intentのeffect ID、external correlationのSHA-256、current safe observationのcanonical SHA-256をrefとbyte比較する。proof kindは`EffectStarted / Succeeded / Terminal / AuthoritativeNotFound / ConfirmedNoEffect / Ambiguous`の6値だけで、ackからSucceeded、AmbiguousからConfirmedNoEffectを推測しない。stale ref、digest不一致、未知kindはeffect 0件でquarantineし、safe observation文字列からproofを捏造しない。

proof ref、pending revision、`SafeEffectObservation`は同じclosureで確定する。public projectionはsafe observationとdigestだけを返し、proof ID、private blob ref、raw evidenceを返さない。terminal closureはproof IDやprivate blobをcompact resultへ複製せず、proofに由来するsafe observationの`proof_sha256`を保持する。private blobはPending recordまたはaction attempt等の参照が全て消えた後だけGCでき、参照中はplan detail compactionを理由に削除しない。Completed actionのcanonical safe resultは別のretention契約で保持するため、same action replayはcurrent proof blobの再構築に依存しない。

external I/Oはdurable `EffectReserved`だけでは開始しない。唯一のRust-owned dispatcherが`EffectDispatchGate(effect_id)`を取得し、I/O invocation直前にstate-by-IDからexact `Pending(EffectReserved)`、terminal Absence、record revision、claim generation / token / owner boot / nonexpired lease、payload内effect ID / canonical intent hash、owner / shutdown association、`ObligationDispatchFenceV1`のorigin / current owner revisionとcommand / runtime / executor generationを再読込する。scope-bearing SessionClose / WorkflowShutdownでは同じassociation / obligationを指すunresolved scope fenceのobligation revision / payload hashも確認し、external recovery action経由ならattempt top-level claim generationの一致も確認する。shutdown targetではcurrent process coordinator、Activated lineage、page hash / ordinal、cutoff前に加え、page refからpoint lookupしたtarget authorityのdigest、plan / epoch / page index / key / ordinal / obligation ID / expected revisionを同じgate内で検査する。authorityが欠損または不一致ならeffectを開始しない。final check後のgate内handoffはnon-asyncかつnon-blockingで、filesystem / socket / process I/O、provider response、Session lock、runtime-event lockその他のlock待ちを一切行わない。immutable commandをcancellation-shielded owned driverへ移し、external I/O開始権と一体のstarted handleをeffect registryへtry-registerするだけとする。registry登録不能ならdriverをI/O開始前に破棄してeffect 0件のtyped failureとし、登録成功後の結果待ちはgate解放後にabsolute deadlineの残予算だけで行う。terminal / resolution / reclaim / claim invalidationも同じgateとregistryを取得し、registered started handleとのfirst-winnerを一意に判定するため、check後・call前にclaimやfenceだけが消える窓、未検査queueへの遅延first dispatch、handoff待ちでStopの10秒budgetを消費する経路を作らない。guard不一致はeffect 0件で終了し、Stop interrupt、send / queue、permission、provider establish / recovery、Session close、Workflow shutdown、recovery actionの全call siteはこの一経路だけを使う。

通常send / queued start、permission response、provider create / resume、backend recovery、normal completion / Fatal / Stop terminal、normal Session close / runtime closeを伴うopen archive、workflow shutdownは、それぞれclosed enumの`ObligationOwner`を持つ上記obligationをexternal I/O前に作る。`RecoveryPublication`だけはexternal effect obligationではなく、BackendRecovery completion closureでmessage payload / marker authorityと共に`Pending`として作るlocal publication obligationである。domain ownerは`Session / WorkflowExecution / OrphanRuntime`の実ownerを保持し、application quitが作るSession / Workflow target obligationだけはimmutableな`ApplicationShutdownAssociation { plan_id, epoch, target_key }`を別fieldに持つ。normal close / recoveryは`shutdown_association=None`であり、quit targetは`Some`を必須とする。旧plan completionはownerだけで受理せずこのassociationとcurrent plan / epochをCAS fenceし、plan queryもassociationをdirect lookupして別planのobligationを混ぜない。dependencyはcreation後immutableなfixed-kind DAGでdepth 1に限定する。`TurnExecution / QueueExecution`だけが同じownerかつ同じoperationに属するdistinctな`ProviderEstablish`を0..=3件持て、`RecoveryPublication`だけが同じrecovery identityの`BackendRecovery`をexactly 1件持つ。他のkindは空を必須とする。kind / owner / operation・recovery identity、重複、self-reference、件数をcreation時に検証し、claim時は最大3件をdirect lookupして再検証する。任意graph traversalや推測したdependency追加を行わない。direct lookupした全dependencyが`Terminal(Succeeded)`になるまでclaim不能であり、`CancelledBeforeEffect / Superseded / FailedTerminal`およびpending `Failed / ReconciliationRequired`はdependencyを満たさない。ProviderEstablishのObserved結果と`Terminal(Succeeded)`をdurableに同時確定する前にTurnExecution / QueueExecutionをclaim / EffectReservedへ進めない。establish結果不明時は両identityを保ったままreconciliationする。claimはpending stateへ複製せず独立lease recordにし、terminal transitionはpending / claimを削除する同じmanifestで4値のcompact resultを作る。manual actionも証拠に従い4値のいずれかへ確定し、generic `ManuallyResolved`を追加しない。

kind / payload / effect / owner / optional correlationの組合せは次のclosed matrixだけを許す。表の`Some` / `None`はwire decode後のcanonical recordでも必須であり、builder、manifest replay、Phase 0→F3 import、claim、action resolverは同じexhaustive validatorを呼ぶ。通常入力の不一致はeffect 0件でtyped reject、既存/import dataの不一致はquarantineし、kindやownerからpayloadを推測して補修しない。

| `ObligationKind` | exact payload / external effect | exact owner / shutdown association | `turn_id` / `operation_id` | allowed pending lifecycle / dependency |
|---|---|---|---|---|
| `TurnExecution` | `TurnExecution` / `TurnStart`、Provider owner | `Session` / None、Session dispatch fence runtime epoch Some | Some exact turn / Some exact send operation | Prepared・Pending・EffectReserved・ReconciliationRequired・Failed / same-owner・same-operation `ProviderEstablish` 0..=3 |
| `QueueExecution` | `QueueExecution` / `QueueTurnStart`、Provider owner | `Session` / None、Session dispatch fence runtime epoch Some | Some＝`reserved_turn_id` / Some exact send operation | 同5 lifecycle / same-owner・same-operation `ProviderEstablish` 0..=3 |
| `TerminalCommit` | `TerminalCommit` / `ProviderInterrupt`、same Session / runtime epochのRuntime owner | `Session` / None、Session dispatch fence | Some＝`target_turn_id` / None | EffectReserved・ReconciliationRequired・Failedだけ / empty |
| `PermissionDelivery` | `PermissionDelivery` / `PermissionResponse`、current Provider owner | `Session` / None、Session dispatch fence | None / None | 5 lifecycle / empty |
| `ProviderEstablish` | `ProviderEstablish` / `ProviderCreate`または`ProviderResume`、payloadと一致するProvider owner | `Session` / None、Session dispatch fence | None / Some exact operation | 5 lifecycle / empty |
| `BackendRecovery` | `BackendRecovery` / `ProviderCreate`または`ProviderResume`、payloadと一致するProvider owner | `Session`またはexact `OrphanRuntime` / None、owner型と一致するdispatch fence | None / None | 5 lifecycle / empty |
| `SessionClose` | `SessionClose` / pendingなら`RuntimeClose`、same Session / exact runtime epochのRuntime owner | exact `Session` / application quit targetだけSome、Session dispatch fence | None / None | scope / effect both Someだけ5 lifecycle。both NoneはPending禁止でdirect Terminal(Succeeded) / empty |
| `WorkflowShutdown` | `WorkflowShutdown` / `WorkflowShutdown`、same workflowのWorkflow owner | exact `WorkflowExecution` / application quit targetだけSome、Workflow dispatch fence | None / None | 5 lifecycle / empty |
| `RecoveryPublication` | `RecoveryPublication` / `ExternalEffectIntent`・dispatch fence・provider proofなし | source BackendRecoveryと同じ`Session`または`OrphanRuntime` / None | None / None | Pending・ReconciliationRequired・Failedだけ、EffectReserved禁止 / same recoveryのBackendRecovery exact 1 |

`ExternalEffectIntent.owner`と`ObligationDispatchFenceV1`もmatrixのowner authorityへ閉じる。Session系Provider effectはそのSessionのeffective provider / runtime authority、RuntimeCloseは同Sessionとpayload `target_runtime_epoch / shutdown_scope`、WorkflowShutdownは同workflow execution / executor generation、OrphanRuntime recoveryはruntime instance / epoch / discovery generationがbyte一致しなければならない。`ReconciliationRequired / Failed`は上記identity / payloadを保持するrecovery state、4値Terminalはcompact resultであり別kindへ遷移しない。

optional fieldの不変条件はstateを跨いで変えない。Turn / Queueは`turn_id`と`operation_id`が常にSome、TerminalCommitはturn Some / operation None、PermissionDeliveryは両field None、ProviderEstablishはturn None / operation Some、残り4 kindは両field Noneである。Queueのturnはpayload `reserved_turn_id`、TerminalCommitのturnは`target_turn_id`とbyte一致する。`SessionClose { effect: None, shutdown_scope: None }`はcompact Terminalだけに存在でき、Pendingへdecode / importしない。RecoveryPublicationはlocal claimを持てるがEffectReservedへ進めない。これら以外のSome補完、Noneへの欠落、state-dependent書換えを禁止する。

Pending / Terminalのcompact dependency検証に使う`semantic_correlation_sha256`は、各componentをu32 BE byte length＋raw bytes（u64は8-byte BE component）で連結した`["obligation-semantic-correlation/v1", Rust variant ASCII kind tag, canonical owner, canonical association, kind-specific IDs]`のSHA-256である。owner componentsはSession=`["session", session_id]`、WorkflowExecution=`["workflow-execution", workflow_execution_id]`、OrphanRuntime=`["orphan-runtime", runtime_instance_id, runtime_epoch, discovery_generation]`、associationはNone=`["none"]`、Some=`["application-shutdown", plan_id, epoch, target_key]`とする。kind-specific componentsはTurnExecution=`[record_app_data_generation_id, operation_id bytes, turn_id]`、QueueExecution=`[record_app_data_generation_id, operation_id bytes, queue_execution_id, reserved_turn_id]`、TerminalCommit=`[terminal_id, target_turn_id]`、PermissionDelivery=`[response_id, request_id]`、ProviderEstablish=`[record_app_data_generation_id, operation_id bytes, launch_or_recovery_id]`、BackendRecovery=`[recovery_id]`、SessionClose=`[close_operation_id]`、WorkflowShutdown=`[workflow_execution_id]`、RecoveryPublication=`[recovery_id, message_id]`である。`SendOperationId`自身はcaller opaque valueだけを持ち、app-data generationはそのrecordを含むvalidated store authorityから別componentとして取得する。`TurnExecution / QueueExecution / ProviderEstablish`は`PendingObligationRecordV1.operation_id=Some(SendOperationId(value))`を必須とし、Noneを別sourceから補完したり、別app-data generationのrecordを同じoperationとして結合したりしない。

Pending creation / importは各ordered dependency IDのcurrent semantic correlation hashを同じ順の`dependency_binding_sha256`（0..=3件、ID列とexact same length）へ固定し、同時にkind / owner / operation・recovery relationを検証する。send由来TurnExecution / QueueExecutionがProviderEstablishを参照する場合は、parentとdependency双方のgeneric `operation_id`がSomeかつcaller opaque valueがbyte一致し、両recordが同じvalidated app-data generation authorityに属し、ProviderEstablish自身のsemantic correlationにも同じgeneration / valueの2 componentが入ることを必須とする。claimはdependency IDを最大3件direct lookupし、compact Terminalに残る`semantic_correlation_sha256`がbindingとbyte一致し、resultがSucceededである場合だけ満たす。Terminalは自身のsemantic correlation hashを保持するがdependency IDs / bindings / payload / private refを保持しないため、compact済みProviderEstablishを別generation / valueのoperationへ流用できない。不一致はcorrupt dependencyとしてclaim / effect 0件でquarantineし、Terminalから失われたpayloadやoperation identityを推測復元しない。

Accepted queued sendのacceptance manifestは`QueueExecution(Pending)`へqueue item / execution identity、acceptance時に採番した`reserved_turn_id`、snapshot hash、provider start用の必須`ExternalEffectIntent`、acceptance時点の`QueueRuntimeGuard`を最初から保存し、pending recordの`turn_id`も同じreserved IDにする。guardは既存runtimeを使う場合のruntime instance / epoch / effective configuration hash、またはpredeclared `ProviderEstablish` obligation / launch-or-recovery ID / effective configuration hashのclosed 2値である。drainはそのturn IDを割り当て直さず、dependency successとguard全fieldがcurrent runtime / effective configurationまたは同一ProviderEstablish resultにbyte一致する場合だけclaim / EffectReservedへ進める。terminal closureは`(session, reserved_turn_id)`からQueueExecution umbrellaを一意にdirect lookupする。immutable pending payloadへdrain時にintent、runtime guard、ProviderEstablish dependencyを後付けしない。acceptance後のruntime喪失、runtime / configuration generation drift、guard不一致、または新しいProviderEstablishが必要になった場合、auto startせずsame QueueExecutionをReconciliationRequired、queueをPausedへ同closureで進め、#1404のCAS付きrebase / cancelだけが新しいexecution identityを作る。既存QueueExecutionへdependencyやguardを追加して起動しない。#1499が実装するのはPending authority、provider start intent、guard、dispatch handoffまでであり、provider start前cancelを含むqueue cancel / CAS / rebaseのstate transitionは#1404が所有する。Phase 0 recovery actionとしてQueueExecutionの`CancelIfSafe`を提示せず、#1499だけで`CancelledBeforeEffect`を作らない。

`ObligationStateRecordV1::Pending`だけがeffect payload、immutable dependencies、claim generation、optional authoritative proof refを保持し、pending inventoryへ入る。`reconciliation_reason`は`state=ReconciliationRequired`の場合だけbounded `Some`を許し、他stateでは必ずNoneである。`PendingObligationState::Failed { failure }`のfailureはFailed state自身の唯一のreasonであり、`reconciliation_reason`へ複製しない。terminal transitionはpending entryとclaimを削除し、state-indexをcompact `ObligationResultRecordV1`へ置換する。Terminal recordはID、owner、immutable shutdown association、kind、semantic correlation、4値result、safe observation、bounded public-safe failure、UTC監査`completed_at`、revisionだけを保持し、turn / operation ID、effect payload、dependencies、claim generation、permission private payload ref、authoritative proof refを保持しない。proofに由来するsafe observationは`proof_sha256`を保持する。`FailedTerminal`は`safe_failure=Some`を必須、`Succeeded / CancelledBeforeEffect / Superseded`は`safe_failure=None`を必須とする。permission terminal commit後は他のlive referenceが無いprivate blobをbounded GCできる。queryはdurable Failedのembedded failure、またはReconciliationRequiredの`reconciliation_reason`をpayloadless public lifecycleとは別のsingle `safe_failure` fieldへ写す。public PendingのPrepared / Pending / EffectReservedはsafe failure None、ReconciliationRequiredは保存済みreasonがあればSome、FailedはSomeを必須とし、failure authorityをfrontendでmergeしない。TerminalをPendingへ戻さない。

Session / Workflow ownerからruntimeをdetachしてowner経由で到達不能にする場合、同じclosure transactionでdetachmentと、`OrphanRuntime { runtime_instance_id, runtime_epoch, discovery_generation }`をownerに持つrecovery obligationの作成またはhandoff、およびpending recovery inventory canonical primaryの`UnownedRuntime` partition insert、actual-owner secondary insert、root envelope更新を確定する。shutdown planはassociationであってruntime ownerではない。先にmemoryだけでorphan化したり、startup / quit時のprocess scanからidentityを捏造したりしない。terminal化または新ownerへの明示handoff時もobligation transition、canonical primaryのdelete / move、actual owner keyが変わる場合のowner secondary更新、両secondary valueのprimary key / revision / payload hash更新、3 tree refを同じtransactionにし、一部indexとdirect obligationを乖離させない。shutdown association自体はimmutableであり、association secondary keyはobligation作成時に一度insertし、terminal / delete時だけremoveする。partition / owner moveをassociation変更として実装しない。

BackendRecoveryはprovider / Session recoveryだけをauthorityとし、そのpayloadがstable recovery ID、provider create / resume / authoritative readback用の必須`ExternalEffectIntent`、独立`RecoveryPublication` obligation IDを所有する。公開message identity、message payload ref、publication状態、markerのauthorityはRecoveryPublication obligationへ一本化し、BackendRecoveryへ複製しない。RecoveryPublicationはBackendRecovery obligation IDをdependencyに持つ。recovery completionはBackendRecovery `Terminal(Succeeded)`とRecoveryPublication `Pending`を同じmanifestでhandoffする。publication workerはlocal claimを取得するが`EffectReserved`へ遷移せず、message公開、publication marker、RecoveryPublication `Terminal(Succeeded)`を同じpublication closureで一度だけ確定する。結果不明時も同じidentity / claimをreadbackし、messageを先に公開した事実からterminalを推測しない。

`TerminalCommit`はAccepted Stop専用であり、normal completion / Fatal / SessionClosed等のlocal terminal producerは作らない。Stop payloadはabsolute deadline、StopDeadlinePermitRef、ProviderInterrupt intentを全てmandatoryとし、StopAcceptance closureでinitial `EffectReserved`＋claimまで確定する。optional fieldでnon-Stop用途と兼用しない。

Stopはrequest ingressでprocess-wide unresolved-admission occupancyからtarget `(session, turn, runtime epoch)`のslotを先取りし、この時点のsame-boot `tokio::time::Instant`を暫定`T0`としてforce-finalizer schedule slotを割り当てる。acceptance commit待ち中もoccupancyを保持するが、StopAcceptance commit成功まではAccepted factとして公開しない。commitが失敗すればAccepted factは存在せずprovider effectも0件である。commit成功時だけT0に対応するUTC `accepted_at`がpublic Acceptedの監査時刻になり、実際にdurabilityを確認したUTC時刻は`acceptance_committed_at`として別に保存する。same bootのforce / 9.5秒 / 10秒判定はwall clockやpersisted stringではなくT0からのmonotonic deadlineだけをauthorityにする。responseがcommit後に遅れてもdeadlineを動かさず、利用者に返したAcceptedから10秒を超えない側へだけ早まる。

distinct未完了targetは最大32件、serialized occupancy / scheduling stateは4 MiB以下とし、same-target duplicateは同じoccupancy / Stop identity / resultへjoinする。各occupancyへ別途force-finalizer schedule slotを割り当てる。healthy storageではStopAcceptance critical manifestとterminal critical manifestへ各125 msを予約し、32 targetsの2 commitで`32 * 2 * 125 ms = 8s`、残りをschedule余白にする。force slotは`accepted_at=T0`に対して`T0 + 1.5s`以上`T0 + 9.25s`以下へ割り当て、`T0 + 9.5s`までにterminal commitを完了できる見込みをacceptance前に検証する。33件目または有効force slotを確保できないStopはprovider interrupt前に`StopCapacityExceeded`を返す。StopAcceptance manifestはtarget turn、runtime epoch、interrupt intent、queue pause、`T0 + 10s` public deadline / epoch fence、両時刻を持つ`StopDeadlinePermitRef`、`TerminalCommit(EffectReserved)` obligationの更新済みclaim generation、独立claim recordを同時に確定し、その後だけAcceptedを返してprovider interruptを起動する。Accepted後の別reserve commitは置かない。`scheduled_force_at`にterminalが無ければruntime event lockを待たないforce-finalizerが`T0 + 9.5s` commit deadlineで実行し、残り0.5秒をquery / emitへ予約する。10秒時点でterminalを保存できずpending `ReconciliationRequired`へ進んだ場合、期限切れforce schedule slotは解放できるがunresolved-admission occupancyは解放しない。occupancyを解放するのは同一TerminalCommitが4値いずれかのterminal resultへdurable確定した時だけである。

canonical terminal gateは`(session_id, turn_id)`ごとにwinner candidateを一つにし、Started turnなら`TurnExecution`、queued由来turnなら`reserved_turn_id`で`QueueExecution` umbrellaをexactly one取得する。同じtargetにAccepted Stopがあればsame-target join indexから単一`TerminalCommit`、両obligation claim、unresolved occupancy、deadline permitも取得する。terminal closureはTerminalRecord、final parts、assistant message、Session / permission / reason別queue state、umbrellaのcompact result、存在するTerminalCommitのcompact result、両pending / claim / index削除、occupancy / permit releaseを一つの`Phase0ClosureTransaction`で確定する。一部だけcommitできる場合は全て公開せず、umbrellaとTerminalCommitをpendingのまま残す。

winnerがそのStop identityの`Interrupted(User | Timeout)`ならumbrellaとTerminalCommitを共に`Terminal(Succeeded)`へ進める。normal completion、Fatal、SessionClosedまたは競合する別terminalが先にwinnerになった場合はumbrellaを`Terminal(Succeeded)`、TerminalCommitを`Terminal(Superseded)`へ進める。umbrellaのSucceededはprovider業務結果の成功ではなく、当該turn effectがcanonical terminalへ一度だけsettleしたことを表す。同payload retryは同じresult / release receiptへ収束し、異payloadのlate candidateはwinnerを書き換えない。closure commit後にだけunresolved occupancyを解放し、emit / notification failureでclosureをrollbackしない。

startupはpublic mutation admission前にpending-only inventoryのStop TerminalCommitからtarget / audit timestamp / UTC deadline / force slotをbounded復元し、`ReconciliationRequired`を含む全pending Stopのsame-target join indexとunresolved-admission occupancyを再構築する。persisted `accepted_at / acceptance_committed_at / scheduled_force_at / terminal_commit_deadline_at`は監査とforeign-boot期限分類だけに使い、新しいprocessで元のmonotonic T0を捏造しない。UTC deadlineを既に過ぎたrecord、clock rollback等で残時間を安全に証明できないrecord、32 distinct targets / 4 MiB超過、重複slot、target / epoch不整合はprovider interruptを再実行せずR-009の`ReconciliationRequired`へ直ちに送る。残時間を証明できるrecordを新bootでscheduleする場合もnew monotonic deadlineをpersisted UTC deadlineより後へ置かず、accepted_atを再採番して10秒を延長しない。terminal result未確定のoccupancyは引き続き32件へ数え、33件目を拒否する。

normal Session close / open archiveはstable close operation IDを持つ。runtimeがある場合、初期lifecycle closureはactiveならSessionClosed terminal、permission settlement、queue pause、Closed / Archived projectionと`SessionClose(Pending)`予約、Idleならsynthetic terminalなしでqueue pause、Closed / Archived projectionと同じPending予約を1manifestで確定する。Pendingはstable runtime instance / epochの`OwnedShutdownScopeRef`とexact effectをboth Someで固定する。`EffectReserved` commit後にscope配下のruntime / child groupを扱うshutdown portへ1回だけ起動し、観測結果を保存する別closureで初めてcompact `Terminal`へ進める。runtimeが無ければ初期lifecycle closureがscope / effectをboth Noneにしたcompact `Terminal(Succeeded)`とprojection等を直接確定し、pending inventory / claim /空のexternal effectを作らない。closed Session archiveはArchived projectionだけでobligation / provider effectを作らない。runtime close結果不明では確定済みClosed / Archived、queue pause、terminalを保ってReconciliationRequiredへ進み、blind retry / reopenしない。backend switchはold runtimeがある場合、初期D1 configuration closureへdesired backend、old effective configuration guard、queue pause、既存itemと`SessionClose(Pending)`予約を入れ、`EffectReserved`後にcloseし、観測結果を保存する別closureでTerminal化した後だけnew effectiveへ進める。old runtimeが無ければ初期D1 closureがscope / effect both Noneのcompact `Terminal(Succeeded)`とnew selected / effective configurationを直接確定し、pending recoveryを残さない。既存queue itemを削除せず、旧backend / configuration snapshotと新configurationが不一致のitemは`NeedsResolution`として自動drainしない。runtime close結果不明ではold effectiveとqueue pauseを保ちnew backendを起動しない。claimはobligation ID、generation、token、owner boot ID、30秒leaseでCASし、reclaim後の旧worker completionを拒否する。結果不明時に自動再実行できるのはstable identityに対するidempotent retryまたはauthoritative readbackを持つeffectだけであり、それ以外は`ReconciliationRequired`へ進める。active / Idle normal close、active / Idle open archive、closed Session archive、Idle backend switchはrequest ingressのsame-boot monotonic T0から10秒をcommand全体のabsolute deadlineとする。初期closureのBeforeCommitは元state / effect 0、OutcomeUnknownはsame operationをresolveし、runtime closeだけがhang / unknownならClosed / Archived、paused queue、activeだけのSessionClosed terminal、switchではold effective backendを保った同じidentityのReconciliationRequiredを10秒以内に返す。closed archiveのqueueは変更せず、late resultから別close / terminal / reopen / new backend startを作らない。

Phase 0 bridgeのcrash-atomic boundaryは`Session`、`Workflow`、`Application`の各scope内で閉じる。Application shutdownだけはopen active / Idle Sessionと進行中Workflowのprepared page、closed / archived Sessionおよびdurable `OrphanRuntime`のpreexisting recovery count / snapshot ref、global root activationをauthorityにし、global activation前のper-target recordやpreexisting recovery snapshotからeffectを開始しない。scope横断の汎用transactionをfile manifestの逐次writeで模倣せず、F3 cutover後は`LocalEventTransactionStore`へauthorityを一度だけ移す。

#### ErrorKind / retry state

**V-D5**: `MessagePart::Error` は非構造化文字列ではなく、少なくとも `kind / retryable / resolved` を持つ。Claude `is_error`、Codex `willRetry`、runtime / persist / provider error を `ErrorKind` へ全域写像し、自動 retry 中は同じ error part を `resolved=true` へ更新する。未知の error kind は `Unknown(String)` として保持し、無言破棄しない。解消: CX-8。

### 5. PermissionRequest（回答の正確な往復と実効性）

**V-D6**: 解消: CX-1 / SD-6 / CL-1 / FE-7。

```rust
pub struct PermissionQuestion {
    pub id: String,                  // CX-1: 回答の対応付けに必須
    pub question: String,
    pub header: Option<String>,
    pub options: Vec<PermissionQuestionOption>,
    pub multi_select: bool,
    pub is_secret: bool,             // CX-1: 秘匿入力
    pub is_other_allowed: bool,      // CX-1: 自由記述の許可
}

// UI入力。validation後はprovider wire向けexact payloadへ変換し、公開event/read modelへ保存しない。
pub struct PermissionAnswerInput(pub BTreeMap<String, Vec<String>>);

pub enum PersistedPermissionAnswer {
    Values(Vec<String>),
    Redacted { answered: bool },
}

pub struct PersistedPermissionAnswers(pub BTreeMap<String, PersistedPermissionAnswer>);

pub enum PermissionRequestBody {
    ToolApproval {
        input: JsonPayload,
        display: Option<ApprovalDisplay>,   // SD-6: 生 JSON を出さない
    },
    PlanApproval { /* 現行維持 */ },
    Question { questions: Vec<PermissionQuestion> },
    PermissionGrant { requested: JsonPayload },
}

pub struct ApprovalDisplay {
    pub command: Option<String>,     // commandExecution の整形表示
    pub diff: Option<String>,        // fileChange の unified diff
    pub file_paths: Vec<String>,
}

pub enum PermissionRequestStatus {
    Pending,
    Responding {
        response_id: String,
        decision: PermissionDecision,
    },
    Resolving {
        reconciliation_id: String,
        resolution_attempt_id: String,
    },
    ReconciliationRequired(PermissionResponseReconciliation),
    Resolved {
        decision: PermissionDecision,        // Allowed / Denied / Cancelled（配線する）
        answers: Option<PersistedPermissionAnswers>,
        resolved_by: PermissionResolvedBy,   // User / Rule / Auto / Backend / System
        effective: bool,                     // CL-1: backend に実際に効いた決定か
    },
}

pub struct PermissionResponseIntent {
    pub response_id: String,
    pub request_id: String,
    pub decision: PermissionDecision,
    pub persisted_answers: Option<PersistedPermissionAnswers>,
    pub has_updated_input: bool,
    pub private_payload_ref: PermissionResponsePrivatePayloadRef,
}

pub struct PermissionResponsePrivatePayloadRef {
    pub private_blob_id: String,
    pub byte_len: u64,
    pub sha256: [u8; 32],
}

pub struct PermissionResponseRejection {
    pub response_id: String,
    pub request_id: String,
    pub reason: String,
    pub raw_control_ref: Option<String>,
    pub rejected_at: String,
}

pub struct ProviderPermissionResponseObservation {
    pub observation_id: String,
    pub outcome: ProviderPermissionResponseOutcome,
    pub raw_control_ref: Option<String>,
    pub observed_at: String,
}

pub enum ProviderPermissionResponseOutcome {
    PendingConfirmed,
    DecisionAccepted { decision: PermissionDecision },
    Cancelled,
    ToolStarted,
    Unknown { reason: String },
    Ambiguous { reason: String },
}

pub enum PermissionReconciliationAction {
    ReadBack,
    ReenterSecret,
    AcceptObserved,
    Cancel,
}

pub struct PermissionResponseReconciliation {
    pub reconciliation_id: String,
    pub response_id: String,
    pub provider_observation: Option<ProviderPermissionResponseObservation>,
    pub allowed_actions: Vec<PermissionReconciliationAction>,
    pub reason: String,
}
```

- `effective: false` は「ユーザーは押したが backend はもう待っていなかった」を表し、履歴上の誤記録（CL-1）を防ぐ。失効（CLI 取り下げ・interrupt）の遷移規則は lifecycle 文書 I7。
- user/rule/autoの回答は、provider adapter向けのexact validated payload（`updated_input / answers / deny message`）を先にowner-only private blobへsyncする。公開manifestはprivate ref / size / SHA-256とredacted `PermissionResponseRequested(PermissionResponseIntent)`、`PermissionDelivery(Pending)` obligationだけを持つ。このpre-I/O commit後に`Responding`へ移し、obligationを`EffectReserved`へCASした後だけproviderへ送る。ack / authoritative observation後だけ`PermissionResolved`をappendする。providerが要求はまだPendingのまま回答だけを明示rejectした場合は、理由付き`PermissionResponseRejected(PermissionResponseRejection)`をappendして同じrequestを`Pending`へ戻し、旧response idを終端する。timeout/restartはresponse idとrequest cancel/tool start observationを相関してeffectiveを判定し、確定不能なら`ReconciliationRequired`として再回答を禁止する。同じresponse idでもidempotencyまたはauthoritative readbackの根拠なしにblind retryしない。
- private blobは共有content-addressed pathへ置かずrandom identityで分離し、POSIXではmode `0600`、Windowsではcurrent user / SYSTEMだけのowner ACLで作成・open時に検証する。exact payload / secret plaintextをevent log、message/session read model、UI、semantic hash、構造化logへ出さない。これは現行app-data privacy boundaryであり、未導入の暗号化at restを保証してはならない。blob refはeffectがObservedまたはresponse / turn terminalになるまで保持し、その後だけGC可能にする。履歴のintent/resultは`Redacted { answered }`だけを持ちfingerprintを作らない。blobが欠落・破損しproviderが同requestをPendingと確認できた場合だけ、新しいresponse idで再入力を要求する。
- `decision_reason` / `description` は現行フィールドを維持し、表示まで配線する（FE-7 は presentation 側）。
- **id の合成**: Claude の AskUserQuestion は wire 上 question id を持たないため、変換層で安定 id（出現順の `q0`, `q1`…）を合成し、`PermissionAnswerInput`をbackendごとのexact validated payload（Codex: idキーの`{answers: {<id>: {answers: [..]}}}`、Claude: 質問順ベース）へ逆写像してprivate blobへ保存する。写像とblob readは各backendのpermission module / gatewayだけが所有する。
- MCP elicitation（CX-2）も本 Question へ写像し「応答義務のある要求」として扱う。elicitation の requestedSchema に Question で表現できないフィールド型がある場合の写像規則は、該当 ISSUE の実装 spec で定義する。

### 6. TurnResult（終了理由の全域化）

**V-D7**: 解消: CL-3 / CL-4 / RG-3 / RG-9 / RT-5。

```rust
pub enum TurnResult {
    Completed {
        stop_reason: TurnStopReason,
        stats: TurnStats,
    },
    Failed {
        error: TurnError,            // RT-5: workflow へ構造化して伝搬
        stats: TurnStats,
    },
    Interrupted {
        reason: InterruptReason,     // 下記closed enumの全variantを表現する
        error: Option<String>,
        stats: TurnStats,            // 中断でも usage を失わない
    },
}

pub enum InterruptReason {
    UserAbort,
    Timeout,
    Crash,
    SessionClosed,
    ProtocolIncompatible,
}

pub enum TurnStopReason {
    EndTurn,
    MaxTurns,        // CL-3: error_max_turns
    MaxTokens,       // CL-4: message_delta.stop_reason
    Refusal,         // CL-4: workflow ModelRefusal failure_signal へ配線
    Unknown(String), // 未知値も落とさない（V-P1）
}

pub struct TurnError {
    pub message: String,
    pub kind: TurnErrorKind,         // Api / Network / Backend / Internal
    pub retryable: bool,
}

pub struct TurnStats {
    pub token_usage: Option<TokenUsage>,
    pub cost_micro_usd: Option<u64>, // RG-9: result.total_cost_usd。Eq derive を保つため micro-USD 整数で保持し表示時に変換
    pub duration_ms: Option<u64>,
    pub num_turns: Option<u32>,
    pub permission_denials: Option<u32>,   // CL-3
}
```

### 7. TokenUsage（cache / context / cost）

**V-D8**: 解消: RG-9 / FE-4（表示は presentation 側）。CX-4 はフィールド名バグの独立修正だが、写像先はこの型に固定する。

```rust
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub context_window_used: Option<u64>,
    pub context_window_max: Option<u64>,
    pub cost_micro_usd: Option<u64>,   // Eq/Copy derive を保つため micro-USD 整数
}
```

`TokenUsage` は実行後・実行中の**使用実績**であり、§9 の `ReasoningEffort` とは別概念である。token 数、cost、context 使用率、時間、turn 数、token budget を「工数」と呼んだり、推論レベルの代用にしてはならない。

### 8. AgentRuntimeEvent（維持・最小拡張）

**V-D9**: ACP への載せ替えは行わず、既存 enum を基礎に必要最小限の拡張を行う。Notice / Todo / ToolCall 更新はすべて `PartsMerged` 経由で流す。変更点のみ:

- `TurnCompleted(TurnResult)` — V-D7 の拡張型に
- `TokenUsageUpdated(TokenUsage)` — V-D8 の拡張型に
- `BackendSessionCleared` — dead code を解消し配線（lifecycle I9 / SD-1）
- `ProviderConfigurationStateObserved(ProviderConfigurationObservation)` — provider wireをadaptorがprovider-neutralなmodel / permission dimensions / reasoning effort / evidence refへ正規化したobservation。usecaseはこれをcanonical observation / reconciliation eventへ接続する。旧`PermissionModeChanged`を置換するが、query read modelや`available_actions`をruntime gateway入力へ含めない
- `PermissionRequested` / `SlashCommandsUpdated` / `KeepAlive` / `Fatal` — 現行維持

`AgentRuntimeEvent`はprovider runtimeからusecaseへ入るbackend gateway型であり、client-facing eventではない。full `AgentSessionConfigurationProjection`はこのenumへ載せず、§11のquery/watch境界がpinned committed sourceと同じevaluation contextから構築した`AgentSessionReadModelDelta::SessionConfigurationChanged`として通知する。

### 9. AgentSessionConfiguration（AgentMode / AgentGoal / ReasoningEffort）

**V-D10（2026-07-15 改訂）**: 2026-07-07 の「`PermissionMode` 3 値＋`plan_mode`」決定を supersede し、mode / Goal / 推論レベルを Rust-owned な Agent 実行設定の一群として扱う。Goal 本体と lifecycle の revision は configuration revision から分離する。

```rust
pub enum AgentMode {
    Ask,
    Edit,
    Plan,
    Auto,
    Bypass,
}

pub struct GoalId(pub String);

pub struct ProviderModelRef {
    pub provider_id: String,
    pub model_id: String,
}

pub struct AgentGoal {
    pub id: GoalId,
    pub objective: String,
    pub status: AgentGoalStatus,
    pub revision: u64,                       // configuration revision とは独立
    pub provider_ref: Option<String>,        // provider event の相関用 opaque id
    pub provider_snapshot: Option<ProviderGoalSnapshot>,
}

pub enum ProviderGoalCorrelation {
    Matched {
        goal_id: GoalId,
        goal_revision: u64,
    },
    Unmatched,
    Ambiguous {
        candidate_goal_ids: Vec<GoalId>,
    },
}

pub struct ProviderGoalSnapshot {
    pub observation_id: String,
    pub provider_goal_ref: Option<String>,
    pub correlation: ProviderGoalCorrelation,
    pub normalized_status: Option<AgentGoalStatus>,
    pub objective: Option<String>,
    pub token_budget: Option<u64>,           // read-only。ReasoningEffort とは無関係
    pub tokens_used: Option<u64>,
    pub time_used_seconds: Option<u64>,
    pub evaluated_turns: Option<u64>,
    pub latest_evaluator_reason: Option<String>,
    pub created_at: Option<String>,
    pub provider_evidence_ref: ProviderEvidenceRef,
}

pub struct ProviderEvidenceRef {
    pub provider_id: String,
    pub scope: ProviderEvidenceScope,
    pub evidence_kind: String,
    pub evidence_id: String,
    pub expires_at: String,
}

pub enum ProviderEvidenceScope {
    Session { session_id: String },
    LaunchAttempt { attempt_id: String },
}

pub enum ProtocolFrameClassification {
    Content,
    Control,
    Unclassified,
}

pub struct BoundedProtocolEvidenceSummary {
    pub payload_length_bytes: u64,
    pub payload_digest: String,
    pub classification: ProtocolFrameClassification,
    pub decode_failure_kind: Option<String>,
    pub redacted_sample: Option<String>, // implementation-defined fixed byte limit以下
    pub provider_evidence_ref: Option<ProviderEvidenceRef>,
}

pub struct GoalCommandAcceptanceEvidence {
    pub observation_id: String,
    pub transition_id: String,
    pub expected_objective_hash: String,
    pub observed_objective_hash: String,
    pub goal_snapshot: ProviderGoalSnapshot,
    pub provider_evidence_ref: ProviderEvidenceRef,
    pub observed_at: String,
}

pub struct GoalPrecommitControlConflictObservation {
    pub observation_id: String,
    pub transition_id: String,
    pub control_kind: String,
    pub provider_evidence_ref: ProviderEvidenceRef,
    pub fail_closed_response_ref: Option<String>,
    pub interrupt_outcome: Option<String>,
    pub observed_at: String,
}

pub enum AgentGoalStatus {
    Active,
    Paused,
    Blocked { reason: String },
    Completed,
    Failed { reason: Option<String> },
}

pub enum GoalOperation {
    Set { goal_id: GoalId, objective: String },
    Edit { objective: String },
    Pause,
    Resume,
    Clear,
}

pub enum GoalAction {
    Set,
    Edit,
    Pause,
    Resume,
    Clear,
}

pub enum ProviderApplicationScope {
    PerTurn,
    Session,
    SessionRestart,
}

pub enum ProviderControlStrategy {
    ProviderNativeRpc,
    ProviderCliCommand,
    ReleashManagedEvaluator,
}

pub enum GoalSideEffect {
    StartsTurn,
    ResetsProviderProgress,
    ReplacesProviderGoalIdentity,
    RestoresProviderGoalState,
    ResetsProviderAccountingBaseline,
}

pub enum GoalCapabilitySupport {
    Native {
        strategy: ProviderControlStrategy,
        scope: ProviderApplicationScope,
        effects: Vec<GoalSideEffect>,
    },
    Emulated {
        strategy: ProviderControlStrategy,
        scope: ProviderApplicationScope,
        effects: Vec<GoalSideEffect>,
    },
    Unsupported { reason: String },
}

pub struct GoalActionAvailability {
    pub action: GoalAction,
    pub support: GoalCapabilitySupport,
    pub schema_supported: bool,
    pub runtime_available: bool,
    pub availability_source: String,
    pub availability_context_hash: String,
    pub checked_at: String,
    pub enabled: bool,                    // lifecycle と managed policy を Rust が評価済み
    pub reason: Option<String>,
}

pub struct GoalActionCapability {
    pub action: GoalAction,
    pub support: GoalCapabilitySupport,
    pub schema_supported: bool,
    pub runtime_available: bool,
    pub availability_source: String,
    pub availability_context_hash: String,
    pub unavailable_reason: Option<String>,
    pub checked_at: String,
}

pub struct GoalCapabilities {
    pub actions: Vec<GoalActionCapability>,
    pub readback: GoalCapabilitySupport,
    pub completion_event: GoalCapabilitySupport,
    pub auto_continuation: GoalCapabilitySupport,
    pub max_objective_length: Option<usize>,
}

pub struct PendingGoalTransition {
    pub transition_id: String,
    pub goal_id: GoalId,
    pub base_goal_revision: Option<u64>,  // Set で current Goal が無い場合は None
    pub originating_launch_attempt_id: Option<String>,
    pub operation: GoalOperation,
}

pub enum ReconciliationAction {
    ReadBack,
    Reapply,
    RollBack,
    AcceptObservedState,
    CleanUp,
    Reuse,
    Recreate,
    Cancel,
}

pub enum ReconciliationResolutionAction {
    Aggregate(ReconciliationAction),
    Permission(PermissionReconciliationAction),
}

pub struct ReconciliationResolutionRequest {
    pub resolution_attempt_id: String,
    pub reconciliation_id: String,
    pub expected_observation_id: Option<String>,
    pub expected_projection_seq: Option<u64>,
    pub action: ReconciliationResolutionAction,
    pub target_hash: Option<String>,
}

pub struct GoalReconciliation {
    pub reconciliation_id: String,
    pub originating_transition_id: Option<String>,
    pub observation_id: Option<String>,
    pub observed: Option<ProviderGoalSnapshot>,
    pub provider_turn: Option<ProviderTurnObservation>,
    pub allowed_actions: Vec<ReconciliationAction>,
    pub reason: String,
}

pub enum ProviderTurnInterruptStatus {
    NotRequested,
    Requested,
    Confirmed,
    Rejected { reason: String },
    TimedOut,
    Unknown { reason: String },
}

pub struct ProviderTurnObservation {
    pub provider_turn_ref: Option<String>,
    pub provider_evidence_ref: Option<ProviderEvidenceRef>,
    pub interrupt_status: ProviderTurnInterruptStatus,
    pub observed_at: String,
}

pub enum GoalSyncState {
    Synced { applied_goal_revision: Option<u64> },
    Applying { transition_id: String },
    RecoveringBackendSession { recovery_id: String },
    ObservationPending { observation_id: String },
    ResolvingReconciliation {
        reconciliation_id: String,
        resolution_attempt_id: String,
    },
    ReconciliationRequired(GoalReconciliation),
}

pub enum GoalTransitionSource {
    User,
    Provider,
    Evaluator,
    System,
}

pub enum GoalTransitionKind {
    Set,
    Edit,
    Pause,
    Resume,
    Blocked,
    Completed,
    Failed,
    Clear,
}

pub enum GoalTransitionResult {
    Applied,
    Rejected,
    Reconciled,
}

pub struct AgentGoalRevisionSnapshot {
    pub goal_id: GoalId,
    pub objective: String,
    pub status: AgentGoalStatus,
    pub revision: u64,
}

pub struct GoalTransitionRecord {
    pub transition_id: Option<String>,
    pub kind: GoalTransitionKind,
    pub result: GoalTransitionResult,
    pub before: Option<AgentGoalRevisionSnapshot>,
    pub after: Option<AgentGoalRevisionSnapshot>,
    pub source: GoalTransitionSource,
    pub occurred_at: String,
    pub originating_launch_attempt_id: Option<String>,
    pub reason: Option<String>,
    pub evidence_ref: Option<String>,
    pub provider_snapshot: Option<ProviderGoalSnapshot>,
}

pub struct AgentGoalState {
    pub current_goal: Option<AgentGoal>,  // terminal Goal も clear / replace までは current
    pub pending_transition: Option<PendingGoalTransition>,
    pub sync_state: GoalSyncState,
}

pub struct SessionGoalProjection {
    pub current_goal: Option<AgentGoal>,  // terminal Goal も clear / replace までは current
    pub pending_transition: Option<PendingGoalTransition>,
    pub sync_state: GoalSyncState,
    pub available_actions: Vec<GoalActionAvailability>,
    pub latest_transition: Option<GoalTransitionRecord>,
}

pub struct GoalHistoryPage {
    pub items: Vec<GoalTransitionRecord>,
    pub next_cursor: Option<String>,
}

// provider / model が公開する値を損失なく保持する。全 provider 共通の固定 enum にしない。
pub struct ReasoningEffort(pub String);

pub enum EffortSelection {
    ProviderDefault,
    Explicit(ReasoningEffort),
}

pub enum EffectiveEffortSource {
    ExplicitSelection,
    ProviderDefault,
}

pub enum EffectiveEffort {
    Known {
        value: ReasoningEffort,
        source: EffectiveEffortSource,
    },
    Unknown {
        selected: EffortSelection,
        expected: Option<ReasoningEffort>,
        reason: String,
    },
}

pub struct ReasoningEffortOption {
    pub value: ReasoningEffort,
    pub display_name: String,
    pub description: Option<String>,
}

pub struct ReasoningEffortCapability {
    pub provider_id: String,
    pub model_id: String,
    pub options: Vec<ReasoningEffortOption>, // provider が広告した順序を維持
    pub default: Option<ReasoningEffort>,
    pub update_timing: ConfigurationUpdateTiming,
    pub schema_supported: bool,
    pub runtime_available: bool,
    pub availability_source: String,          // ProviderApi / PinnedCompatibilityTable / RuntimePolicy
    pub availability_context_hash: String,    // provider/model/deployment/org policy/override を含む
    pub checked_at: String,
    pub unavailable_reason: Option<String>,
    pub authoritative_runtime_validation: bool,
    pub effective_readback_supported: bool,
}

pub enum ConfigurationUpdateTiming {
    Live,
    NextTurn,
    SessionRestart,
}

pub enum SessionControlOperationKind {
    ConfigurationUpdate,
    GoalTransition,
}

pub struct SessionControlOperationLease {
    pub operation_id: String,
    pub kind: SessionControlOperationKind,
    pub acquired_at: String,
}

pub struct AgentSessionConfiguration {
    pub model: ProviderModelRef,
    pub mode: AgentMode,
    pub reasoning_effort: EffortSelection,
    pub revision: u64,
}

pub struct AgentEffectiveConfiguration {
    pub model: ProviderModelRef,
    pub mode: AgentMode,
    pub mode_snapshot: EffectiveModeSnapshot,
    pub reasoning_effort: EffectiveEffort,
    pub revision: u64,
    pub provider_session_generation: u64,
}

// full snapshot ではなく discriminated patch にして、1 command 1 concern を型で保証する。
// model と effort は同じ capability 単位なので、model 変更時は対象 model 向けに検証済みの
// EffortSelection を同じ patch に含める。ProviderDefault の preview/expected は別read modelで返し、
// providerが確認するまで具体的なeffective値としてpatchしない。
pub enum ConfigurationPatch {
    SetModel {
        model: ProviderModelRef,
        reasoning_effort: EffortSelection,
    },
    SetMode {
        mode: AgentMode,
        // mode == Bypass のとき必須。それ以外はNone。
        bypass_confirmation: Option<BypassChallengeConfirmation>,
    },
    SetReasoningEffort(EffortSelection),
}

pub struct PendingConfigurationUpdate {
    pub update_id: String,
    pub base_selected_revision: u64,
    pub target_revision: u64,
    pub patch: ConfigurationPatch,
    pub applies_from: ConfigurationUpdateTiming,
}

pub struct ProviderConfigurationObservation {
    pub observation_id: String,
    pub model: Option<ProviderModelRef>,
    pub permission_snapshot: Option<ProviderPermissionSnapshot>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub provider_evidence_ref: Option<ProviderEvidenceRef>,
    pub observed_at: String,
}

pub struct ConfigurationReconciliation {
    pub reconciliation_id: String,
    pub originating_update_id: Option<String>,
    pub observation_id: Option<String>,
    pub observed: Option<ProviderConfigurationObservation>,
    pub differing_fields: Vec<ExecutionConfigurationField>,
    pub allowed_actions: Vec<ReconciliationAction>,
    pub reason: String,
}

pub enum ConfigurationSyncState {
    Synced,
    Applying { update_id: String },
    AwaitingNextTurn { update_id: String },
    AwaitingRestart { update_id: String },
    RecoveringBackendSession {
        recovery_id: String,
        target_selected_revision: u64,
    },
    ObservationPending { observation_id: String },
    ResolvingReconciliation {
        reconciliation_id: String,
        resolution_attempt_id: String,
    },
    ReconciliationRequired(ConfigurationReconciliation),
}

// query_models owned。committed event / projection dataからquery_service_implが
// AgentSessionConfiguration aggregateを経由せず直接構築する。
pub struct AgentSessionConfigurationSelectionSnapshot {
    pub model: ProviderModelRef,
    pub mode: AgentMode,
    pub reasoning_effort: EffortSelection,
    pub revision: u64,
}

pub struct AgentSessionConfigurationProjection {
    pub selected: AgentSessionConfigurationSelectionSnapshot, // adapter が受理したユーザー選択のread model
    pub effective: AgentEffectiveConfiguration, // provider が現在使用中の値
    pub pending_update: Option<PendingConfigurationUpdate>,
    pub sync_state: ConfigurationSyncState,
    pub available_actions: Vec<ConfigurationActionAvailability>,
}

// command / migration がloadする最小state。query専用available_actionsを含めない。
pub struct AgentSessionConfigurationLoadState {
    pub selected: AgentSessionConfiguration,
    pub effective: AgentEffectiveConfiguration,
    pub pending_update: Option<PendingConfigurationUpdate>,
    pub sync_state: ConfigurationSyncState,
}

pub enum ConfigurationAction {
    SetModel {
        model: ProviderModelRef,
        reasoning_effort: EffortSelection,
    },
    SetMode(AgentMode),
    SetReasoningEffort(EffortSelection),
}

pub struct ConfigurationActionAvailability {
    pub action: ConfigurationAction,
    pub enabled: bool,
    pub reason: Option<String>,
    pub update_timing: ConfigurationUpdateTiming,
    pub requires_bypass_challenge: bool,
}

pub struct TurnStartIntent {
    pub request_id: String,
    pub reserved_turn_id: String,
    pub provider_correlation_key: String,
    pub message_id: String,
    pub immutable_input_ref: String,
    pub input_hash: String,              // prompt/attachments/editor_context を含む
    pub execution_configuration: TurnExecutionConfigurationIntent,
    pub queue_item_id: Option<String>,
    pub queue_execution_id: Option<String>,
}

pub enum TurnExecutionConfigurationIntent {
    ExistingEffective(ResolvedTurnConfiguration),
    ActivateSelected {
        selected: AgentSessionConfiguration,
        goal_id: Option<GoalId>,
        goal_revision: Option<u64>,
        originating_update_id: String,
        canonical_target_hash: String,
        prevalidated_context_hash: String,
        prevalidated_at: String,
    },
}

// shared kernel owned。Workflow contextが意味を所有する相関値を両contextへ運ぶが、
// AgentSession domainからWorkflow domain型を直接importしない。
pub struct NodeDefinitionName(pub String);

pub struct NodeExecutionRef {
    pub workflow_execution_id: String,
    pub node_execution_id: String,       // NodeExecution.id
    pub node_name: NodeDefinitionName,   // NodeDefinition.name / NodeExecution.node_name
    pub node_attempt: u32,               // NodeExecution.attempt
}

pub enum TurnConfigurationSource {
    SessionEffective { revision: u64 },
    QueueItem {
        item_id: String,
        item_revision: u64,
        execution_id: String,
        snapshot_hash: String,
    },
    NodeExecution {
        execution: NodeExecutionRef,
        resolved_configuration_hash: String,
    },
}

pub struct ResolvedTurnConfiguration {
    pub configuration: AgentEffectiveConfiguration,
    pub goal_id: Option<GoalId>,
    pub goal_revision: Option<u64>,
    pub canonical_hash: String,
    pub source: TurnConfigurationSource,
}

pub struct TurnStartReconciliation {
    pub reconciliation_id: String,
    pub originating_request_id: String,
    pub provider_turn: Option<ProviderTurnObservation>,
    pub observed_configuration: Option<ProviderConfigurationObservation>,
    pub allowed_actions: Vec<ReconciliationAction>,
    pub reason: String,
}

pub enum TurnStartState {
    Idle,
    Starting(TurnStartIntent),
    InterruptRequested {
        intent: TurnStartIntent,
        interrupt_request_id: String,
    },
    ReconciliationRequired(TurnStartReconciliation),
    Resolving {
        reconciliation_id: String,
        resolution_attempt_id: String,
    },
}

pub enum AgentSessionConfigurationState {
    Ready(AgentSessionConfigurationLoadState),
    NeedsConfigurationResolution(ConfigurationResolutionProblem),
}

// query serviceがpinned sourceから直接構築するclient-facing read model。
pub enum AgentSessionConfigurationReadModel {
    Ready(AgentSessionConfigurationProjection),
    NeedsConfigurationResolution(ConfigurationResolutionProblem),
}

pub enum ConfigurationResolutionScope {
    Session,
    QueueItem { item_id: String },
    WorkflowExecution { workflow_execution_id: String },
    NodeExecution(NodeExecutionRef),
}

pub enum ExecutionConfigurationField {
    Model,
    Mode,
    ReasoningEffort,
    InitialGoal,
}

pub enum ConfigurationResolutionAction {
    SelectReplacement { field: ExecutionConfigurationField },
    RebaseToCurrent,
    Retry,
    Cancel,
}

pub struct UnresolvedConfigurationField {
    pub field: ExecutionConfigurationField,
    pub raw_payload: Option<String>,
    pub reason: String,
}

pub struct ConfigurationResolutionProblem {
    pub resolution_id: String,
    pub scope: ConfigurationResolutionScope,
    pub fields: Vec<UnresolvedConfigurationField>,
    pub allowed_actions: Vec<ConfigurationResolutionAction>,
}

pub struct QueueExecutionConfigurationSnapshot {
    pub configuration: AgentEffectiveConfiguration,
    pub goal_id: Option<GoalId>,
    pub goal_revision: Option<u64>,
    pub canonical_hash: String,
}

pub enum QueueItemStatus {
    Queued,
    AwaitingBypassConfirmation(QueueExecutionPrepared),
    Starting { execution_id: String },
    Started { execution_id: String, turn_id: String },
    Failed { reason: String },
    Cancelled,
    NeedsResolution(ConfigurationResolutionProblem),
}

pub struct QueuedAgentTurn {
    pub item_id: String,
    pub revision: u64,
    pub message_id: String,
    pub input_ref: String,
    pub snapshot: QueueExecutionConfigurationSnapshot,
    pub status: QueueItemStatus,
}

pub struct QueueProjection {
    pub active_items: Vec<QueuedAgentTurn>,
    pub recent_terminal_items: Vec<QueuedAgentTurn>, // bounded
    pub paused: bool,
    pub seq: u64,
}

pub struct QueueHistoryPage {
    pub items: Vec<QueuedAgentTurn>,
    pub next_cursor: Option<String>,
}

pub struct QueueExecutionPrepared {
    pub item_id: String,
    pub execution_id: String,
    pub expected_item_revision: u64,
    pub snapshot_hash: String,
    pub challenge: BypassConfirmationChallenge,
}

// QueueExecutionRequestedをappendするcommand input。Bypass snapshotではconfirmation必須。
pub struct QueueExecutionStartIntent {
    pub item_id: String,
    pub execution_id: String,
    pub expected_item_revision: u64,
    pub snapshot_hash: String,
    pub bypass_confirmation: Option<BypassChallengeConfirmation>,
}

pub struct QueueItemRebased {
    pub item_id: String,
    pub expected_item_revision: u64,
    pub expected_snapshot_hash: String,
    pub new_item_revision: u64,
    pub new_snapshot: QueueExecutionConfigurationSnapshot,
    pub resolution_id: String,
    pub rebased_at: String,
}

pub struct QueueItemResolutionRequired {
    pub item_id: String,
    pub expected_item_revision: u64,
    pub problem: ConfigurationResolutionProblem,
    pub current_configuration_revision: u64,
    pub current_goal_id: Option<GoalId>,
    pub current_goal_revision: Option<u64>,
    pub detected_at: String,
}

pub struct QueuePaused {
    pub pause_id: String,
    pub cause: String,
    pub interrupted_turn_id: Option<String>,
    pub paused_at: String,
}

pub struct QueueResumed {
    pub resume_id: String,
    pub expected_queue_seq: u64,
    pub resumed_at: String,
}

pub struct QueueItemRequeued {
    pub retry_id: String,
    pub item_id: String,
    pub expected_item_revision: u64,
    pub previous_execution_id: Option<String>,
    pub new_item_revision: u64,
    pub requeued_at: String,
}

pub enum BypassChallengeGuard {
    Session {
        session_id: String,
        selected_revision: u64,
    },
    LaunchAttempt {
        attempt_id: String,
        canonical_draft_hash: String,
    },
    QueueItem {
        session_id: String,
        item_id: String,
        execution_id: String,
        snapshot_hash: String,
    },
    Reconciliation {
        scope: ReconciliationScope,
        reconciliation_id: String,
        resolution_attempt_id: String,
        expected_observation_id: Option<String>,
        expected_projection_seq: Option<u64>,
        action_hash: String,
        target_hash: String,
    },
    NodeExecution {
        execution: NodeExecutionRef,
        resolution_id: String,
        resolved_configuration_hash: String,
    },
}

pub enum ReconciliationScope {
    SessionConfiguration { session_id: String },
    Goal { session_id: String, goal_id: Option<GoalId> },
    LaunchAttempt { attempt_id: String },
    TurnStart { session_id: String, request_id: String },
    Permission { session_id: String, request_id: String },
}

pub struct BypassConfirmationChallenge {
    pub challenge_id: String,
    pub guard: BypassChallengeGuard,
    pub target_mode: AgentMode,            // 常に Bypass
    pub expires_at: String,
    pub nonce: String,
    pub residual_protections: Vec<ResidualProtection>,
    pub managed_policy_revision: String,
    pub issued_at: String,
}

// consume command / intentがchallenge idと対で提示するsecret。
pub struct BypassChallengeConfirmation {
    pub challenge_id: String,
    pub nonce: String,
}

pub enum BypassChallengeState {
    Issued,
    Consumed { intent_id: String, consumed_at: String },
    Expired { expired_at: String },
    Cancelled { reason: String, cancelled_at: String },
}

pub struct BypassChallengeView {
    pub challenge_id: String,
    pub guard: BypassChallengeGuard,
    pub target_mode: AgentMode,
    pub expires_at: String,
    pub nonce: Option<String>, // Issuedかつ認可済みclientにだけ返す
    pub residual_protections: Vec<ResidualProtection>,
    pub managed_policy_revision: String,
    pub issued_at: String,
}

pub struct BypassChallengeProjection {
    pub challenge: BypassChallengeView,
    pub state: BypassChallengeState,
    pub seq: u64,
}

pub struct AgentGoalSpec {
    pub objective: String,
}

pub struct AgentConfigurationDraft {
    pub model: ProviderModelRef,
    pub mode: AgentMode,
    pub reasoning_effort: EffortSelection,
    pub initial_goal: Option<AgentGoalSpec>,
}

pub struct AgentLaunchSubmission {
    pub draft: AgentConfigurationDraft,
    pub preflight_context_hash: String,
}

pub struct PreparedAgentLaunch {
    pub attempt_id: String,
    pub canonical_draft_hash: String,
    pub preflight_context_hash: String,
    pub draft: AgentConfigurationDraft,
    pub bypass_challenge: Option<BypassConfirmationChallenge>,
    pub expires_at: String,
}

pub struct StartAgentLaunch {
    pub attempt_id: String,
    pub canonical_draft_hash: String,
    pub bypass_confirmation: Option<BypassChallengeConfirmation>,
}

pub enum AgentLaunchOrigin {
    Manual,
    NodeExecution {
        execution: NodeExecutionRef,
        resolution_id: String,
        resolved_configuration_hash: String,
    },
}

pub enum RequiredOverride<T> {
    Inherit,
    Set(T),
}

pub enum OptionalOverride<T> {
    Inherit,
    Set(T),
    Clear,
}

pub struct AgentConfigurationTemplate {
    pub revision: u64,
    pub model: RequiredOverride<ProviderModelRef>,
    pub mode: RequiredOverride<AgentMode>,
    pub reasoning_effort: OptionalOverride<EffortSelection>,
    pub initial_goal: OptionalOverride<AgentGoalSpec>,
}

pub enum ConfigurationValueSource {
    LaunchBaseline { revision: u64 },
    WorkflowExecutionDefault { revision: u64 },
    NodeDefinitionOverride { revision: u64 },
}

pub struct LaunchConfigurationBaseline {
    pub revision: u64,
    pub model: ProviderModelRef,
    pub mode: AgentMode,
    pub reasoning_effort: EffortSelection,
    pub initial_goal: Option<AgentGoalSpec>,
}

pub struct ResolvedLaunchConfiguration {
    pub resolution_id: String,
    pub resolution_version: u32,
    pub canonical_hash: String,
    pub baseline_revision: u64,
    pub workflow_execution_default_revision: u64,
    pub node_definition_override_revision: u64,
    pub model: ProviderModelRef,
    pub mode: AgentMode,
    pub reasoning_effort: EffortSelection,
    pub initial_goal: Option<AgentGoalSpec>,
    pub provenance: Vec<(ExecutionConfigurationField, ConfigurationValueSource)>,
}

pub struct WorkflowWaitingConfiguration {
    pub resolution_id: String,
    pub problem: ConfigurationResolutionProblem,
}

pub struct WorkflowWaitingBypassConfirmation {
    pub execution: NodeExecutionRef,
    pub resolution_id: String,
    pub resolved_configuration_hash: String,
    pub challenge: BypassConfirmationChallenge,
}

pub struct NodeExecutionBypassPrepared {
    pub waiting: WorkflowWaitingBypassConfirmation,
    pub expected_workflow_seq: u64,
    pub prepared_at: String,
}

// NodeExecutionLaunchRequestedをappendするusecase input。Bypass解決時はconfirmation必須。
pub struct NodeExecutionLaunchIntent {
    pub execution: NodeExecutionRef,
    pub resolution_id: String,
    pub resolved_configuration_hash: String,
    pub agent_launch_attempt_id: String,
    pub bypass_confirmation: Option<BypassChallengeConfirmation>,
}

pub enum NodeExecutionGateState {
    Ready,
    WaitingConfiguration(WorkflowWaitingConfiguration),
    WaitingBypassConfirmation(WorkflowWaitingBypassConfirmation),
    BypassConfirmationExpired(WorkflowWaitingBypassConfirmation),
    Starting {
        node_execution_id: String,
        node_attempt: u32,
        agent_launch_attempt_id: String,
    },
}

pub enum LaunchStage {
    Validated,
    ProviderResourceRequested {
        correlation_key: String,
        idempotency_key: Option<String>,
    },
    ProviderResourceObserved {
        provider_ref: String,
    },
    InitialConfigurationApplied,
    LocalSessionCommitted {
        session_id: String,
    },
    InitialGoalTransitionRequested {
        transition_id: String,
        goal_id: GoalId,
    },
    InitialGoalCommitted {
        transition_id: String,
        goal_id: GoalId,
        goal_revision: u64,
        turn_id: Option<String>,
    },
}

pub struct LaunchRecoveryCapability {
    pub supports_create_idempotency_key: bool,
    pub supports_lookup_by_correlation_key: bool,
    pub lookup_consistency: Option<ProviderLookupConsistency>,
    pub supports_cleanup_by_provider_ref: bool,
}

pub enum InitialGoalLaunchState {
    TransitionRequested {
        transition_id: String,
        goal_id: GoalId,
    },
    Committed {
        transition_id: String,
        goal_id: GoalId,
        goal_revision: u64,
        turn_id: Option<String>,
    },
    ReconciliationRequired {
        transition_id: String,
        reconciliation_id: String,
    },
    Rejected {
        transition_id: String,
        goal_id: GoalId,
        reason: String,
    },
}

pub enum InitialGoalFailureAction {
    RetryGoal,
    ContinueWithoutGoal,
    CancelSession,
}

pub struct InitialGoalResolutionRequest {
    pub resolution_attempt_id: String,
    pub attempt_id: String,
    pub expected_transition_id: String,
    pub expected_projection_seq: u64,
    pub action: InitialGoalFailureAction,
}

pub struct LaunchInitialGoalRejected {
    pub attempt_id: String,
    pub transition_id: String,
    pub goal_id: GoalId,
    pub reason: String,
    pub allowed_actions: Vec<InitialGoalFailureAction>,
    pub rejected_at: String,
}

pub struct InitialGoalResolutionCompleted {
    pub resolution_attempt_id: String,
    pub action: InitialGoalFailureAction,
    pub next_transition_id: Option<String>,
    pub completed_at: String,
}

pub struct SessionCreated {
    pub session_id: String,
    pub originating_launch_attempt_id: String,
    pub provider_id: String,
    pub provider_session_ref: String,
    pub protocol_identity: BackendProtocolIdentity,
    pub created_at: String,
}

pub struct BackendSessionRecoveryStarted {
    pub recovery_id: String,
    pub session_id: String,
    pub old_provider_session_generation: u64,
    pub reason: String,
    pub started_at: String,
}

pub struct SessionConfigurationReactivated {
    pub recovery_id: String,
    pub configuration: AgentEffectiveConfiguration,
    pub provider_session_generation: u64,
    pub consumed_observation_id: Option<String>,
    pub reactivated_at: String,
}

pub enum GoalReactivationOutcome {
    NoCurrentGoal,
    TerminalGoalUnchanged { goal_id: GoalId, goal_revision: u64 },
    Restored { goal_id: GoalId, goal_revision: u64, provider_goal_ref: Option<String> },
    ObservedUnchanged { goal_id: GoalId, goal_revision: u64 },
}

pub struct SessionGoalReactivated {
    pub recovery_id: String,
    pub outcome: GoalReactivationOutcome,
    pub provider_session_generation: u64,
    pub restoring_turn_id: Option<String>,
    pub consumed_observation_id: Option<String>,
    pub reactivated_at: String,
}

pub struct BackendSessionRecoveryCompleted {
    pub recovery_id: String,
    pub provider_session_generation: u64,
    pub configuration_revision: u64,
    pub goal_revision: Option<u64>,
    pub completed_at: String,
}

pub enum ProviderResourceMatchBasis {
    ProviderRef,
    CorrelationKey,
    IdempotencyKey,
}

pub enum ProviderLookupConsistency {
    Authoritative,
    Eventual {
        minimum_stability_seconds: u64,
    },
}

pub enum ProviderResourceLookupResult {
    Found {
        provider_ref: String,
        matched_by: ProviderResourceMatchBasis,
    },
    NotFound {
        consistency: ProviderLookupConsistency,
        stable_since: Option<String>,
    },
    Ambiguous {
        candidate_refs: Vec<String>,
    },
    Unsupported,
}

pub struct LaunchProviderObservation {
    pub observation_id: String,
    pub lookup: ProviderResourceLookupResult,
    pub configuration: Option<ProviderConfigurationObservation>,
    pub observed_at: String,
}

pub struct LaunchReconciliation {
    pub reconciliation_id: String,
    pub last_completed_stage: LaunchStage,
    pub provider_create_correlation_key: String,
    pub provider_create_idempotency_key: Option<String>,
    pub provisional_provider_ref: Option<String>,
    pub local_session_id: Option<String>,
    pub initial_goal_state: Option<InitialGoalLaunchState>,
    pub recovery_capability: LaunchRecoveryCapability,
    pub observed: Option<LaunchProviderObservation>,
    pub protocol_identity: ObservedProtocolIdentity,
    pub allowed_actions: Vec<ReconciliationAction>,
    pub reason: String,
}

pub enum AgentLaunchAttemptStatus {
    Validating,
    Provisioning,
    WaitingForInitialGoal { transition_id: String },
    WaitingForInitialGoalResolution {
        transition_id: String,
        allowed_actions: Vec<InitialGoalFailureAction>,
    },
    ResolvingInitialGoalFailure {
        transition_id: String,
        resolution_attempt_id: String,
        action: InitialGoalFailureAction,
    },
    ResolvingReconciliation {
        reconciliation_id: String,
        resolution_attempt_id: String,
    },
    ReconciliationRequired(LaunchReconciliation),
    ProtocolIncompatible {
        incompatibility: ProtocolIncompatibility,
        recovery: LaunchReconciliation,
    },
    Completed { session_id: String },
    Cancelled { reason: String },
    Failed { reason: String },
}

pub struct AgentLaunchAttempt {
    pub attempt_id: String,
    pub origin: AgentLaunchOrigin,
    pub canonical_draft_hash: String,
    pub validated_preflight_context_hash: String,
    pub provider_create_correlation_key: String,
    pub provider_create_idempotency_key: Option<String>,
    pub draft: AgentConfigurationDraft,
    pub last_completed_stage: Option<LaunchStage>,
    pub provisional_provider_ref: Option<String>,
    pub local_session_id: Option<String>,
    pub initial_goal_state: Option<InitialGoalLaunchState>,
    pub status: AgentLaunchAttemptStatus,
}

pub struct BackendProtocolIdentity {
    pub executable_version: String,
    pub schema_tag: String,
    pub commit_sha: Option<String>,
    pub schema_hash: String,
    pub experimental_flags: Vec<String>,
    pub initialize_capabilities_hash: String,
}

pub struct ObservedProtocolIdentity {
    pub executable_version: Option<String>,
    pub schema_tag: Option<String>,
    pub commit_sha: Option<String>,
    pub schema_hash: Option<String>,
    pub experimental_flags: Option<Vec<String>>,
    pub initialize_capabilities_hash: Option<String>,
}

pub struct ProtocolIncompatibility {
    pub observed: ObservedProtocolIdentity,
    pub expected_schema_hash: String,
    pub reason: String,
    pub raw_control_ref: Option<String>,
}

pub enum AgentProtocolState {
    Compatible(BackendProtocolIdentity),
    ProtocolIncompatible(ProtocolIncompatibility),
}

pub enum AgentLaunchLifecycleState {
    Prepared(PreparedAgentLaunch),
    Started(AgentLaunchAttempt),
    PreparationExpired {
        attempt_id: String,
        canonical_draft_hash: String,
        expired_at: String,
    },
    PreparationCancelled {
        attempt_id: String,
        canonical_draft_hash: String,
    },
}

pub struct AgentLaunchProjection {
    pub state: AgentLaunchLifecycleState,
    pub seq: u64,
}

pub struct AgentLaunchChanged {
    pub projection: AgentLaunchProjection,
}

pub enum AgentLaunchPreflightState {
    Checking,
    Compatible {
        capabilities: AgentBackendCapabilities,
    },
    ProtocolIncompatible(ProtocolIncompatibility),
}

pub struct AgentLaunchPreflight {
    pub workspace_id: String,
    pub provider_id: String,
    pub context_hash: String,
    pub state: AgentLaunchPreflightState,
}

// ここからのcapability / control strategyはadaptor/gatewayがprotocol identityと
// runtime contextから構築するbackend read model。domain aggregate/eventはwire fieldを所有しない。
pub enum ModeControlStrategy {
    ClaudePermissionMode,
    CodexCompositePolicy,
    CodexCollaborationPreset,
}

pub enum SandboxIntent {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

pub enum ApprovalIntent {
    Interactive,
    Never,
}

pub enum ReviewerIntent {
    User,
    ProviderClassifier,
    AutoReview,
}

pub enum ModeBehaviorNudge {
    KeepWorking,
    ReduceClarifyingQuestions,
}

pub enum ModeEffect {
    Sandbox(SandboxIntent),
    Approval(ApprovalIntent),
    Reviewer(ReviewerIntent),
    BehaviorNudge(ModeBehaviorNudge),
    CollaborationPreset { name: String },
}

pub struct EffectiveModeSnapshot {
    pub normalized_mode: AgentMode,
    pub provider_permission: ProviderPermissionSnapshot,
    pub effects: Vec<ModeEffect>,
    pub residual_protections: Vec<ResidualProtection>,
    pub availability_context_hash: String,
    pub policy_revision: String,
    pub evaluated_at: String,
}

pub enum ModeCapabilitySupport {
    Native {
        strategies: Vec<ModeControlStrategy>,
        scope: ProviderApplicationScope,
        effects: Vec<ModeEffect>,
    },
    Composed {
        strategies: Vec<ModeControlStrategy>,
        scope: ProviderApplicationScope,
        effects: Vec<ModeEffect>,
    },
    Unsupported { reason: String },
}

pub struct ModeCapability {
    pub mode: AgentMode,
    pub schema_supported: bool,
    pub runtime_available: bool,
    pub availability_source: String,
    pub unavailable_reason: Option<String>,
    pub checked_at: String,
    pub support: ModeCapabilitySupport,
    pub requires_launch_opt_in: bool,
    pub residual_protections: Vec<ResidualProtection>,
}

pub struct ResidualProtection {
    pub protection_id: String,
    pub description: String,
    pub source: String,
}

pub struct AgentBackendCapabilities {
    pub provider_id: String,
    pub protocol_identity: BackendProtocolIdentity,
    pub supported_modes: Vec<ModeCapability>,
    pub reasoning_efforts: Vec<ReasoningEffortCapability>,
    pub goal: GoalCapabilities,
    pub launch_recovery: LaunchRecoveryCapability,
    pub turn_steer: TurnSteerCapability,
}

pub enum TurnSteerCapability {
    Unsupported { reason: String },
    Supported {
        idempotency: bool,
        authoritative_readback: bool,
        cancel: bool,
        source: CapabilitySource,
        checked_at: String,
    },
}

pub enum AutoOperationalState {
    NotApplicable,
    InProgress,
    Active,
    ManualFallback { reason: String },
    TimedOut,
    Aborted,
}

pub enum PermissionDimension {
    Sandbox(SandboxIntent),
    Approval(ApprovalIntent),
    Reviewer(ReviewerIntent),
    CollaborationPreset { name: String },
    AutoOperation(AutoOperationalState),
}

pub struct ProviderPermissionSnapshot {
    pub provider_id: String,
    pub dimensions: Vec<PermissionDimension>,
    pub evidence_ref: ProviderEvidenceRef,
    pub observed_at: String,
}

pub struct ProviderPermissionState {
    pub snapshot: ProviderPermissionSnapshot,
    pub normalized_mode: Option<AgentMode>,
    pub residual_protections: Vec<ResidualProtection>,
}
```

#### bounded-context 間の型所有

Workflow contextが意味を所有する`workflow_execution_id / node_execution_id / node_name / node_attempt`は、両contextが依存できる**shared kernel**の`NodeDefinitionName / NodeExecutionRef`だけで運ぶ。`TurnConfigurationSource`、`ConfigurationResolutionScope`、`BypassChallengeGuard`、`AgentLaunchOrigin`などAgentSession側の型がWorkflow domainの`NodeDefinitionName`やNodeExecution entityを直接importしてはならない。

逆方向も同様に、Workflow側の`AgentConfigurationTemplate / WorkflowWaitingBypassConfirmation / NodeExecutionGateState`が必要とするmode / effort / Goal spec / configuration resolutionはshared-kernelのcross-context contract valueとして参照し、AgentSession aggregate/entityを直接importしない。各sibling domainはshared-kernel valueと自aggregate valueの変換を自境界で行う。AgentSession domainとWorkflow domainが互いのdomain型を直接importする双方向配置は採用しない。最終的なRust moduleファイル配置は#1446以降で確定してよいが、所有者はusecase DTOではなくshared kernelとする。

以下は **adaptor/gateway の service / command model** であり、上の domain value / event へ含めない。provider adapter は pin した wire をこの型へ decode し、raw field と control ref を evidence store に保存したうえで、provider-neutral な `ProviderPermissionSnapshot` / `GoalCommandAcceptanceEvidence` へ変換する。domain は `ProviderEvidenceRef` だけでその証跡を参照する。

`ProviderEvidenceRef`が指すevidence storeはadaptor/infrastructure境界のbounded storeとする。full payloadが必要なwriteでもsecret plaintextを保存前にredactし、暗号化at rest、session scopeごとの参照認可、objectごとのTTL、単一object byte上限、per-session aggregate byte quotaを必須にする。object上限またはquota超過はfull bodyをevent/logへfallbackせず明示errorとし、期限切れ・scope不一致のreadも拒否する。durable event / structured log / Noticeに残せるのは`BoundedProtocolEvidenceSummary`の長さ・digest・分類・decode failure種別・固定上限以下のredacted sampleと任意のrefだけで、full bodyを恒久保存しない。具体的な暗号方式・byte上限値・TTL値は#1446以降で決める。

```rust
pub enum ProviderPermissionWireSnapshot {
    Claude(ClaudePermissionWireSnapshot),
    Codex(CodexPermissionWireSnapshot),
}

pub struct ClaudePermissionWireSnapshot {
    pub permission_mode: String,
    pub auto_state: String,
    pub allow_dangerously_skip_permissions: bool,
    pub raw_control_refs: Vec<String>,
}

pub struct CodexPermissionWireSnapshot {
    pub sandbox_policy: String,
    pub approval_policy: String,
    pub approvals_reviewer: String,
    pub collaboration_mode: Option<String>,
    pub permission_profile_id: Option<String>,
    pub auto_state: String,
    pub raw_control_refs: Vec<String>,
}

pub struct ClaudeGoalCommandEvidence {
    pub observation_id: String,
    pub transition_id: String,
    pub command_uuid: String,
    pub executable_version: String,
    pub expected_objective_hash: String,
    pub observed_objective_hash: String,
    pub command_lifecycle_completed_ref: String,
    pub goal_snapshot: ProviderGoalSnapshot,
    pub provider_turn_ref: Option<String>,
    pub raw_control_refs: Vec<String>,
    pub observed_at: String,
}

pub enum ProviderGoalWireSnapshot {
    Claude {
        provider_goal_ref: Option<String>,
        raw_status: String,
        raw_control_refs: Vec<String>,
    },
    Codex {
        provider_goal_ref: Option<String>,
        raw_status: String,
        raw_control_refs: Vec<String>,
    },
}
```

`AgentGoalState` は `AgentGoal` の id/revision、current同時最大1件、pending transition、sync stateの不変条件を所有し、Command側だけで再構築・利用するdomain aggregateである。`SessionGoalProjection`はdomain aggregateではなく、`query_service_impl`がpinned snapshot lease上のcommitted Goal event / projection dataとruntime capability / managed policy dataから直接組み立てるread modelである。Query側は`AgentGoalState`を再構築・経由せず、表示・転送要求で決まる`available_actions`と`latest_transition`をmutation stateへ保持しない。

`AgentBackendCapabilities` は pin した schema version と application scope を含む。Goalとeffortのavailabilityはschema上の存在だけでなく、workspace trust、session、managed policy、deployment/organization上限、capability overrideを含む実行contextで再評価し、source/context hash/checked atを返す。`SessionGoalProjection.available_actions` は raw capability と現在 status、pending transition、managed policy を Rust query が評価した結果である。`AgentSessionConfigurationProjection.available_actions` は Idle、configuration/Goal sync、pending update/transition、control-operation lease、runtime capability、managed policyから、model/mode/effortの各候補についてRust queryが評価し、disabled reason、反映時点、Bypass challenge要否を返す。frontend はどちらの遷移・enablement表も再実装しない。

#### AgentMode の意味と provider 写像

| Releash mode | Claude canonical mode | Codex control intent | Releash の意味 |
|---|---|---|---|
| `Ask` | `default` | read-only sandbox + interactive approval + user reviewer | 読み取り中心。書き込み・危険操作はユーザー判断を要求する |
| `Edit` | `acceptEdits` | workspace-write sandbox + interactive approval + user reviewer | workspace 編集を許可し、それを越える操作は確認する |
| `Plan` | `plan` | `collaborationMode/list` から得た Plan preset + read-only intent | 計画作成専用。独立した mode であり boolean toggle ではない |
| `Auto` | capability が広告する `auto` | workspace-write sandbox + interactive approval + `auto_review` reviewer | provider 側 classifier / reviewer が eligible な要求を自動審査する |
| `Bypass` | `bypassPermissions` | danger-full-access sandbox + approval never | 通常のprovider操作承認を最大限迂回する最危険mode。provider固有の残存保護まで無効とは表現しない |

- 5 mode は完全同値な provider policy ではなく、利用者意図を capability へ正規化する語彙である。adapter は pin した Claude / Codex version の typed schema から実際の field・preset を組み立て、config file 名と app-server wire 名を混在させない。schema enum に存在することと実行時利用可能性を分け、`schema_supported / runtime_available / availability_source / unavailable_reason / checked_at / protocol_identity`を返す。さらにmode専用`ModeCapabilitySupport`でcontrol strategyとsandbox/approval/reviewer/collaboration preset/behavior nudgeの実効effectを返し、Goal用effect型を流用しない。
- Claude `acceptEdits` は file edit 以外の一部 filesystem command も自動許可する。`auto` は account / model / provider / version 条件があり、runtime で利用不能なら `Unsupported` とする。`bypassPermissions` は起動時の dangerous opt-in gate も必要であり、gate 無しの既存 Session では restart-required または disabled とする。一方でexplicit ask/deny rules、`requiresUserInteraction` MCP、root/home削除circuit breaker等のprovider protectionは残り得るため、adapterは`residual_protections`をcapability/effective permission stateに列挙する。Rust challenge は provider gate に追加される Releash policy であって代替ではない。
- Claude Autoはprovider classifierへのdelegationに加え、keep-workingとclarifying question削減のbehavior nudgeを持つ。Codex `auto_review` は sandbox を広げず reviewer を user から別 agent へ替えるだけで、同じbehavior nudgeを合成しない。Releash 自身が provider 共通の安全判定器を持つ、という意味でもない。各差はtyped`ModeEffect`としてUI/turn auditへ出す。review status は `inProgress / approved / denied / timedOut / aborted` を全域写像し、approved / denied だけを `PermissionResolvedBy::Auto` とする。inProgress は activity、timedOut / aborted は未解決のまま manual fallback または Notice とし、自動解決を合成しない。
- Claude Auto が連続 block 等で manual prompt へ一時 fallback しても selected mode は Auto のままである。`AutoOperationalState::ManualFallback` と理由を read model / Notice へ出し、mode 値を書き換えない。
- Codex Plan preset が reasoning effort を含む場合、明示選択済み effort を黙って上書きしない。明示 override が可能なら適用し、不可能なら mode / effort の capability conflict として解決を要求する。
- Claude の `dontAsk` は `Auto` と同義ではない。Claude / Codex の複合的な permission state は adaptor/gateway が provider 固有 raw snapshot として損失なく evidence store に保持し、domain には正規化した `PermissionDimension`、effects、residual protections、provider/evidence ref だけを渡す。正規化不能または `normalized_mode=None` の session は `ReconciliationRequired` として turn を開始しない。
- `Bypass` は通常のprovider操作承認を最大限迂回するintentであり、provider固有のresidual protections、Releash managed policy、workflow human checkpoint、承認 node、停止条件を迂回しない。`BypassConfirmationChallenge` は target mode、期限、nonce に加え、Sessionなら`session_id + selected_revision`、New Agentなら`attempt_id + canonical_draft_hash`、Queueなら`session_id + item_id + execution_id + snapshot_hash`、Workflowなら`NodeExecutionRef + resolution_id + resolved_configuration_hash`、reconciliationならscope＋resolution attempt＋expected observation/seq＋action/target hashへ束縛する。Launch/Queue/Workflowのprepare eventと`BypassChallengeIssued`は同じlocal atomic batchでappendする。`ConfigurationPatch::SetMode { mode: Bypass, .. }`、`StartAgentLaunch`、`QueueExecutionStartIntent`、`NodeExecutionLaunchIntent`はchallenge idとnonceを一体の`BypassChallengeConfirmation`として受け取る。consume usecaseは提示nonceを当該Issued challengeのnonceとconstant-time相当で照合し、未失効の`expires_at`、完全一致するguard、認可済みcaller/session/workspace scope、managed policyを全て再検査できた場合だけ、`BypassChallengeConsumed`を各durable intentと同じlocal atomic batchでappendする。`challenge_id`と、clientから観測可能な`attempt_id` / `canonical_draft_hash` / snapshot hashだけをbearer proofとしてconsumeしてはならない。nonceはIssued中の認可済みclientだけが持つsecretであり、terminal projection/logからredactする。provider I/O中にlockは保持しない。provider失敗後もconsumedのままとし、同一intent id・同一guardのidempotent retryだけが再利用できる。waiting projectionはchallenge全体を保持し、reload後もguard/期限/residual protectionsを復元する。template に `Bypass` を保存しても権限付与にはならない。

#### AgentGoal

- Goal本体とlifecycleのmutation authorityはReleashの`AgentGoalState`、durable authorityはcanonical Goal eventに置き、configuration aggregateから完全に分離する。`AgentGoalState.current_goal`は同時に最大1件で、Active / Paused / Blockedのときだけactiveと呼ぶ。Completed / Failedもclearまたは次のsetまではcurrentとして保持し、backend queryがevent/projection dataから`SessionGoalProjection`へ表示する。Goalのset / edit / transition / clearでconfiguration revisionを進めない。
- `goal_id` と provider の opaque ref で通知を相関し、置換前 Goal の遅延 completion が新 Goal を完了させないようにする。domainの`ProviderGoalSnapshot`へprovider ref、normalized status、`Matched { goal_id, revision } / Unmatched / Ambiguous`、`ProviderEvidenceRef`をdurableに保存し、crash/replay後も相関判定を再現する。raw provider status / control fieldはadaptor/gatewayの`ProviderGoalWireSnapshot`としてevidence storeへ保存する。Unmatched/Ambiguousをcurrent Goalへ適用しない。transition は `source`（User / Provider / Evaluator / System）、理由、時刻、任意の `evidence_ref` を記録する。
- Goal capability は `set / edit / clear / pause / resume / readback / completion_event / auto_continuation / max_objective_length` を項目別に `Native / Emulated / Unsupported(reason)` で返す。各actionはmode同様に`schema_supported / runtime_available / availability_source / availability_context_hash / unavailable_reason / checked_at`を持ち、workspace/session/managed-policy context変更時に再評価する。adapter の適用戦略は `ProviderNativeRpc`、`ProviderCliCommand`、明示した `ReleashManagedEvaluator`、`Unsupported` のいずれかとし、暗黙の prompt 接頭辞で対応済みに見せない。
- Codex `thread/goal/set|get|clear`・goal notification は pin した typed RPC adapter（`ProviderNativeRpc`）で扱う。status は `active → Active`、`paused → Paused`、`complete → Completed`、`blocked / usageLimited / budgetLimited → Blocked` と全域写像し、normalized statusとread-only accountingをdomainの`ProviderGoalSnapshot`に保持する。raw statusはadaptor/gatewayの`ProviderGoalWireSnapshot`とevidence storeに保持し、unknown statusは`normalized_status=None`とその`ProviderEvidenceRef`でGoal reconciliationに入る。`Failed` は Releash / System 固有で Codex native status とは扱わない。objective変更はset RPCによる`Edit` emulationとして`ReplacesProviderGoalIdentity / ResetsProviderProgress`を宣言する。
- Claude で公開確認できる surface は typed Goal RPC ではなく `/goal` CLI command（`ProviderCliCommand`）であり、setとactive Goalのobjective変更はGoal保存/置換と同時にturnを開始する。`Set`は`StartsTurn`、`Edit`は`StartsTurn / ReplacesProviderGoalIdentity / ResetsProviderProgress`を宣言する。pinしたCLI fixtureで`system/command_lifecycle(completed, command_uuid)`とtyped Goal state (`goal_set`/`goal_status`またはactive Goal snapshot)の両方を観測し、要求objective hash一致を確認した adaptor/gateway の`ClaudeGoalCommandEvidence`だけをacceptance evidenceにする。adapterはraw command evidenceをevidence storeへ保存し、domain eventにはprovider-neutralな`GoalCommandAcceptanceEvidence`と`ProviderEvidenceRef`を渡す。content-plane deltaだけをbufferし、evidence後に`ProviderGoalCommandEvidenceObserved + GoalSet/GoalTransitioned + TurnStarted`をatomic appendして公開する。commit前の`can_use_tool`/`request_user_dialog`等の応答必須control-planeはbufferせずfail-closed応答→interruptし、`GoalPrecommitControlConflictObserved`を保存してGoal/turn reconciliationへ送る。shape/order/相関をfixtureで証明できないCLI versionではStartsTurn actionを`Unsupported`にする。
- Claude `/goal` actionを広告するにはCLI versionだけでなく、workspace trust、`disableAllHooks`、managed `allowManagedHooksOnly`等のruntime requirementを確認する。取得不能・不充足なら理由付き`Unsupported`またはdisabledにする。Claude Codeの`--resume / --continue`によるSession復元はGoal Actionの`Resume`とは別で、Goal state復元とaccounting baseline resetを表しても自動でturnを開始したとは扱わない。clear後に`/goal <objective>`でGoal Resumeをemulateする場合だけ、再setに伴う`StartsTurn / ReplacesProviderGoalIdentity / ResetsProviderProgress`を宣言する。
- native pause / resume がない provider で clear / re-set を使う場合、`ResetsProviderProgress / StartsTurn / ReplacesProviderGoalIdentity` 等の意味損失を `Emulated.effects` に列挙し、操作前に表示する。effect を観測・補償できない version では `Unsupported` とする。
- provider 固有 Goal token budget / tokens used / elapsed time / evaluated turn count の設定 UI と accounting 集計は今回の scope 外である。受信値とlatest evaluator reasonは小さなread-only snapshotとして監査表示できるが、ReasoningEffort と結び付けない。
- completion / failure は provider / evaluator / system event が Rust usecase を通して確定する。UI の利用者操作は set / edit / pause / resume / clear とし、完了根拠を捏造しない。利用可能操作は `available_actions` をそのまま描画する。
- user / system の Goal 操作も `GoalTransitionRequested → adapter apply / ack → GoalSet / GoalTransitioned / GoalCleared` の独立 write-ahead protocol を通す。開始時にSession共通`SessionControlOperationLease`を取得し、configurationが`Synced`でpending無しの場合だけ受理する。configuration updateとGoal transitionをIdle内でも直列化し、Claude GoalのStartsTurnとactivationを競合させない。provider reject は旧 current Goal を維持し、timeout、部分成功、ack 後の canonical event append 失敗、provider 競合は `GoalReconciliationRequired` とする。reconciliation自身には新しい`reconciliation_id`を発行し、対応するlocal requestがある場合だけ`originating_transition_id`、provider観測がある場合だけ`observation_id`を持つ。provider-originated driftのために架空のtransitionを合成しない。unresolved request 自体が restart 後の reconciliation 根拠になる。
- `ProviderGoalStateObserved`のappend自体で`ObservationPending { observation_id }`へ遷移して新規turnをblockする。後続のcanonical Goal event、no-change acceptance、または`GoalReconciliationRequired`が同じobservation idをconsumeするまで`Synced`へ戻さない。restart時は未consumed observationを再評価し、観測だけ保存してdrift/completionを無視する窓を作らない。
- Goal による automatic continuation は `AgentMode` と直交する。Goal が active でも mode、permission、queue、workflow human checkpoint を維持し、Plan 等で continuation が制約される場合は capability と停止理由を示す。

#### ReasoningEffort（UI 表示名: 工数（推論レベル））

- `ReasoningEffort` は provider / model が提供する**応答・推論強度の behavioral signal**を表す。実際の使用 token、cost、時間、turn 数、token / cost / time budget、厳密な上限ではない。Claude では thinking だけでなく本文・tool call の詳しさにも作用し得る。
- selected は `EffortSelection::ProviderDefault | Explicit(value)`、effective は `Known { value, source: ExplicitSelection | ProviderDefault } | Unknown { selected, expected?, reason }` と型を分ける。provider default、未取得、非対応、effective 不明を `None` 一つに畳まず、table等から予想できる値は`expected`、providerが確認した値だけを`Known`にする。`TurnStarted` は effective effort と protocol identity を snapshot する。
- 選択肢・説明・既定値は provider / model capability から返す。Codex の `model/list` が返す順序も維持し、`low / medium / high` 等を Releash の普遍 enum として hard-code しない。default は option 単体ではなく provider / model capability に置く。
- Claude integrationでruntime capability/effective readbackを確認できない場合、pinしたCLI version × model tableはschema候補とpreviewの根拠にだけ使い、effective確定根拠にはしない。組織のeffort上限はJSON/stream-json/background agentで無通知clampされ得るため、resolved provider/model/deployment、organization limit、capability overrideを含む実行contextでauthoritative validationまたはeffective readbackができなければ、明示effortを理由付き`Unsupported`にする。`schema_supported / runtime_available / availability_source / availability_context_hash / checked_at / authoritative_runtime_validation / effective_readback_supported`をcapabilityに含め、Claude側のsilent clampに依存しない。
- model 変更は対象 `ProviderModelRef` と、その model 向け `EffortSelection` を一つの `SetModel` patch に含める。Rustは`model.provider_id == current session provider_id`を必須検証し、cross-provider変更を通常のconfiguration patchで扱わない。provider変更は進行turnのfinalize、Goal/queueの明示handoff判断、新しいprotocol preflight/launchを伴う別usecaseとする。ProviderDefault を選んだ場合は広告値または pinned table の default を preview し、effective readback が無ければ `Unknown { selected: ProviderDefault, expected, reason }` として予想値と不明理由を明示する。反映時点が live / next turn / restart のどれかも capability に含める。

#### 設定更新 protocol

外部 provider と local persistence は atomic transaction にできないため、model / mode / reasoning effort の更新を「ack 後に一括保存」とは扱わない。1 command は `ConfigurationPatch` の 1 variant だけを送り、full target snapshot は Rust が base selected revision から導出する。

1. user 起点の execution-affecting 更新は初期実装では `Idle` のみ受け付ける。Session共通`SessionControlOperationLease`を取得し、`base_selected_revision` を CAS 検証してcapabilityとmanaged policyを確認する。`sync_state != Synced`、Goal sync stateが非Synced、Goal transition pending、別control lease中は次の更新を拒否する。`SetMode { mode: Bypass, bypass_confirmation: Some(..) }` はchallenge id / nonce・期限・guard・caller scopeを一回限りのconfirmationとして検証し、`None`またはnonce不一致をprovider I/O前に拒否する。
2. provider I/O の前に `ConfigurationUpdateRequested { update_id, base_selected_revision, target_revision, patch, applies_from }` を event log へ append する。append 成功が durable intent の commit point であり、失敗したら provider へ送らない。
3. `Live` は adapter が provider-native 更新または明示された Releash-managed strategy を直ちに適用する。`NextTurn / SessionRestart` で独立した provider 設定 API が無い場合、adapter は typed capability に基づき staging を受理するだけで、provider 適用済みとは報告しない。複数 provider field が必要なら adapter 内で順序・補償を所有し、部分成功を success として返さない。
4. live の provider ack、または next-turn / restart staging の adapter acceptance 後に `SessionConfigurationSelected` event を appendし、selected configuration の唯一の durable commit point とする。live は `SessionConfigurationActivated` も appendして effective revision を進める。next-turn / restart は pending request を保持したまま `AwaitingNextTurn / AwaitingRestart` とし、実際の provider activation event append まで effective を進めない。
5. providerが独立configuration APIを持つ`AwaitingNextTurn`は、次turnのprovider startより前にselected patchを適用し、activation ackと`SessionConfigurationActivated` appendが完了してからeffective snapshotでturnを開始する。`AwaitingRestart`もrestart/readback後に同じ順序でactivateする。
6. model/mode/effortを`turn/start` payloadでしか適用できないproviderでは、activation前`TurnStarted`を要求しない。`TurnStartRequested.execution_configuration`は、既に確定したSession/queue値なら`ExistingEffective(ResolvedTurnConfiguration)`、pending selectedなら`ActivateSelected { selected, originating_update_id, canonical_target_hash, prevalidated_context_hash }`とし、provider ack前のtargetをeffective型へ詰めない。queue起点はitemのcanonical semantic snapshotとitem revision/execution id/hashを固定し、current selectedから再構成しない。ack/readback後に初めてactual effectiveを`TurnStarted`へ保存する。`SessionConfigurationActivated`は`ActivateSelected`をsession-scopeで実際にactivateした場合だけappendし、queue per-turn overrideは`QueueItemStarted + TurnStarted`だけをatomic commitしてSession effectiveを変更しない。timeout/ack不明は`TurnStartReconciliation`へ移す。Reuse/Accept成功とCancel/CleanUp成功はqueue terminal/message markerまで同じbatchで閉じ、未完了intentは同じinput/correlationで回復する。
7. `SessionConfigurationSelected`前のprovider rejectだけは`ConfigurationUpdateRejected`を記録してpendingを消し、旧selected/effectiveを維持する。selected commit後のNextTurn/Restart activation reject・timeoutはselectedを巻き戻さず、new selected / old effectiveのまま`ConfigurationReconciliationRequired`としてturn / queue drain / workflow resumeをblockする。明示rollbackを選んだ場合は旧effective相当を新しいselected revisionとしてcanonical eventへappendし、revisionを逆行させない。ack後のcanonical event append失敗、部分成功、provider-originated競合も同じreconciliationへ送る。`ProviderConfigurationStateObserved`はmodel/effortに加え、adaptor/gatewayがClaudeのraw modeまたはCodexのsandbox/approval/reviewer/collaboration preset等をprovider固有evidenceとして保存し、正規化した`ProviderPermissionSnapshot`と`ProviderEvidenceRef`をdomainへ渡す。readbackで必要な全dimensionを確認できない場合は安全なrollback/acceptをallowed actionsへ出さない。readback / idempotent reapply / rollback /明示acceptを`ConfigurationReconciled`で確定する。reconciliation自身には`reconciliation_id`を発行し、local request由来のときだけ`originating_update_id`、provider観測があるときだけ`observation_id`を関連付ける。provider-originated driftのために架空のupdateを合成しない。結果がBypassになる解決は汎用`BypassChallengeGuard::Reconciliation`でscope、resolution attempt、observation/seq、action/target hashへ束縛し、fresh challengeとpolicy/gate再検査を必須にする。

idempotence は revision だけでなく `update_id` で判定する。`ProviderConfigurationStateObserved`のappend自体で`ObservationPending { observation_id }`へ遷移し、canonical activation/no-change acceptance/reconciliation eventが同じobservation idをconsumeするまで新規turnをblockする。restart時は未consumed observationを再評価する。provider-originated change は pending update と同値なら ack として扱い、異なる場合は上書きせず reconciliation に入る。`SessionMeta` は event log から再構築できる projection/cache であり、cache 更新失敗は `PersistFailure` と再投影で回復する。canonical event append 失敗と同一視しない。

transientな`BackendSessionCleared`を受けたらresume metadata clearと`BackendSessionRecoveryStarted`を同じlocal atomic commitにし、configuration/Goalを`RecoveringBackendSession { recovery_id }`へ移してturn/queue/workflow resumeをblockする。新provider sessionでselected configurationをreapply/readbackし、readback observation idをconsumeする`SessionConfigurationReactivated`をappendする。GoalのNone/terminal/no-change/restoreを網羅した`SessionGoalReactivated + BackendSessionRecoveryCompleted`を同じ最終batchでappendして初めてSynced/公開し、Goal restoreがStartsTurnなら`TurnStarted`もそのbatchへ含めearly streamをbufferする。Goal readback pathもconsumed observation idを保存する。結果不明はGoal/turnまたはconfiguration reconciliationへ送る。

全ての`TurnStarted`、queue drain、backend resume、workflow resume直前にprovider/model/mode/reasoning effortとGoal continuation capabilityを、deployment/org override、workspace trust、managed policy、provider launch gate/residual protectionsを含む最新availability context hashで再評価する。authoritative effort validation不能やeffective Bypassのpolicy/gate失効は送信せずUnknown/`NeedsConfigurationResolution`/reconciliationへ移す。成功時のprovider permission/effects/residual protections/context hashは`EffectiveModeSnapshot`へ固定し、`TurnStarted`のimmutable execution snapshotとして監査する。

queueではenqueue snapshotのcanonical semantic hashを再評価結果と比較する。provider/model/mode/effort/effects/residual protections/policyの意味が変わればsilent差替えせず`QueueItemResolutionRequired`へ送り、`QueueItemRebased`だけがsnapshotを更新する。`checked_at/evaluated_at`等の観測時刻はsemantic hashから除外し、意味が同じ場合はsource snapshot/hashを維持したまま最新revalidation evidenceをTurnStartedへ保存する。Bypass prepareのchallengeにも最新effects/residual protectionsを含める。

configuration / Goal / launch / turn-start / permission reconciliationの解決操作も共通write-ahead sagaを通す。`reconciliation_id`、expected observation id/projection seq、action、target hashをCAS検証してresolution attemptをreserveする。結果がBypassならscope＋attempt＋observation/seq＋action/target hashへ束縛したchallengeを発行し、provider I/O前に`ReconciliationResolutionRequested { resolution_attempt_id, ... }`＋consumeを同じbatchでappendして`ResolvingReconciliation`へ排他遷移する。provider ack/readback後だけ対象`*Reconciled`をappendする。permissionのReenterSecretはPending確認後に旧responseを終端して新response idを発行しplaintextを保存しない。未完了resolution intentはrestart時に同じattempt id/correlationでreadback/recoveryし、actionやresponseを二重実行しない。

初回launchの部分成功や成否不明は`LaunchReconciliation`へ移す。`attempt_id`から安定生成したprovider create correlation keyと、provider対応時のidempotency keyをrequest前にdurable化する。各`StageAdvanced`はprovider ref、local session id等のstage payloadも保存する。provider readbackはcreate lookupの`Found { provider_ref, matched_by } / NotFound { consistency, stable_since } / Ambiguous { candidate_refs } / Unsupported`とconfiguration observationを`LaunchProviderObservation`へ保存する。Reuseは一意なFoundだけとする。Recreateはauthoritative NotFound、または同じidempotency keyでcreate自体が安全な場合だけallowedにする。eventual NotFoundはstability windowを記録してもidempotency key無しRecreateの根拠にせず、Ambiguous/Unsupported/存在不明でも出さない。reconciliationは`reconciliation_id`、最後に完了した`LaunchStage`、opaqueなprovisional provider ref、local session id、provider観測、観測できた範囲のprotocol identity、provider create recovery capability、`CleanUp / ReadBack / Reuse / Recreate / Cancel`のallowed actionsを保持する。initialize完了前のdecode failureでも部分的な`ObservedProtocolIdentity`とexpected hash/raw control refを`AgentLaunchAttemptStatus::ProtocolIncompatible`へ保存する。`AgentLaunchAttempt`とBypass challengeは同じ`canonical_draft_hash`を共有し、draft変更後のchallenge再利用を拒否する。

provider resource作成とinitial configurationのapply/readbackが完了したらsession idを確保し、`SessionCreated + SessionConfigurationSelected(revision=1) + SessionConfigurationActivated(revision=1) + LaunchStageAdvanced(LocalSessionCommitted)`をlaunch/session stream横断のlocal atomic batchでappendする。initial Goalが無ければ`AgentLaunchCompleted`も同じbatchへ含める。これが新Session configurationのcanonical seedで、SessionMetaは後から再投影する。batch失敗時はlocal Sessionを公開せずlaunch reconciliationへ入り、provider effectiveなのにlocal selected/effectiveが無い窓を旧値fallbackで隠さない。

launch draftの`initial_goal`はconfigurationに混ぜてprovider createへ送らず、`LocalSessionCommitted { session_id }`後に独立Goal sagaへhandoffする。`LaunchStageAdvanced(InitialGoalTransitionRequested)`と`GoalTransitionRequested { originating_launch_attempt_id }`を同じlocal event transactionでcommitしてからprovider I/Oを行う。initial Goalのcanonical Goal event、Claudeならevidence付き`TurnStarted`、`LaunchStageAdvanced(InitialGoalCommitted)`、`AgentLaunchCompleted`も同じtransactionで確定する。それまでは`WaitingForInitialGoal`とする。Goal結果不明は同じtransition idでreconcileする。provider明示rejectはGoal streamの`GoalTransitionRejected`とlaunch streamの`LaunchInitialGoalRejected`を同じtransactionでcommitし、reload後も`WaitingForInitialGoalResolution`へ投影する。

RetryGoal / ContinueWithoutGoal / CancelSessionはexpected transition/attempt seqをCASし、`InitialGoalResolutionRequested { resolution_attempt_id, action }`をappendして`ResolvingInitialGoalFailure`へ移す。RetryGoalは`InitialGoalResolutionCompleted { action: RetryGoal, next_transition_id }`＋新transition idのlaunch/Goal intentを同じtransactionでcommitして`WaitingForInitialGoal`へ移す。ContinueWithoutGoalはresolution completed＋launch completed、CancelSessionはcleanup後にresolution completed＋launch cancelled＋session closedを同じtransactionで確定する。結果不明は同resolution attemptのLaunchReconciliationへ移し、暗黙retryしない。

NodeExecution originでは既存の`NodeExecution.id + resolution_id`からstableなAgent launch attempt idを導出し、相関はshared-kernelの`NodeExecutionRef`として保存する。`NodeExecutionLaunchIntent`から`NodeExecutionLaunchRequested + AgentLaunchAttemptStarted`をworkflow/launch stream横断の同じtransactionで開始する。Bypassだけは先に`NodeExecutionBypassPrepared + BypassChallengeIssued`をcommitして待機し、challenge id / nonce・期限・guard・caller scopeの検証後に共通開始transactionへ`BypassChallengeConsumed`を追加する。完了時は`AgentLaunchCompleted + NodeExecutionAgentBound`、失敗/取消時は`AgentLaunchFailed/Cancelled + NodeExecutionAgentLaunchFailed/Cancelled`を同じtransactionで確定し、retryは新しい`NodeExecution.id / attempt`を使う。

#### Provider 仕様の根拠と pinning

- Claude: [permission modes](https://code.claude.com/docs/en/permission-modes)、[Goal / requirements](https://code.claude.com/docs/en/goal#requirements)、[Effort](https://platform.claude.com/docs/en/build-with-claude/effort)、[organization effort limits](https://code.claude.com/docs/en/model-config#organization-effort-limits) を規範入力とする。
- Codex: [App Server](https://learn.chatgpt.com/docs/app-server)、[Auto-review](https://learn.chatgpt.com/docs/sandboxing/auto-review)、[long-running Goal](https://learn.chatgpt.com/docs/long-running-work) と、[openai/codex app-server README / generated schema](https://github.com/openai/codex/tree/main/codex-rs/app-server) を規範入力とする。
- living docs は調査・意味の根拠、実装 wire の規範は dependency に pin した CLI / SDK tag が生成する schema と fixture とする。ただし schema だけを pin して PATH 上の別 version を起動してはならない。
- initialize 前後に `BackendProtocolIdentity { executable_version, schema_tag, commit_sha, schema_hash, experimental_flags, initialize_capabilities_hash }` を取得し、compiled adapter の compatibility manifest と照合する。Codex schema の experimental flag、Claude/Codex launch gate、runtime capability も identity に含める。
- 不一致や control-plane decode failure は低強調 `UnsupportedMessage` で続行せず、Session確立後はsession-level、確立前はdurable launch attemptの`ProtocolIncompatible`としてfail-closedにする。initialize途中で全identity fieldを取得できない場合も`ObservedProtocolIdentity`の取得済みfieldと`BoundedProtocolEvidenceSummary`、必要時だけ認可・TTL・quota付き`ProviderEvidenceRef`を失わない。version 更新時は mode availability、Goal status / RPC、reasoning effort option、approval / sandbox / reviewer field の差分を D1 と parity fixture で review する。

#### 旧データの移行

| 旧設定 | resolved `AgentMode` | command / migration load用 `AgentSessionConfigurationState` |
|---|---|---|
| `plan_mode = true`（permission modeは任意） | `Plan` | `Ready` |
| `plan_mode = false`, `permission_mode = Ask` / legacy `readonly` | `Ask` | `Ready` |
| `plan_mode = false`, `permission_mode = Edit` | `Edit` | `Ready` |
| `plan_mode = false`, `permission_mode = Full` | 未確定 | `NeedsConfigurationResolution(ConfigurationResolutionProblem { fields: [{ field: Mode, reason: LegacyBypassConfirmationRequired, ... }], ... })` |

`plan_mode = true` を permission mode より優先する。この写像とlegacy Fullのsend block / fresh rechallengeはquery projectionではなくcommand / migration load用`AgentSessionConfigurationState`を参照し、`Ready`にquery専用`available_actions`を要求しない。既知の model 値も `ProviderModelRef` へ移し、selected effort は `ProviderDefault`、effective は pinned table / readback で判定できる場合だけ concrete value、できなければ理由付き `Unknown` とする。`plan_mode = true`、または`plan_mode = false`かつFull以外の既知legacy値は`selected.revision = effective.revision = 1`へidempotentにlazy migrationし、自動write-backしない。`plan_mode = false`のlegacy Full Sessionはcommand側load stateでsendをblockし、fresh challenge、managed policy、runtime availability、provider gateを確認してから新しいrevisionのBypassとしてcommitする。workflow templateのFullはBypass intentへ移せても権限付与ではなく、既存WorkflowExecution/queueを含む各executionで新challengeを必須とする。次の成功した設定writeでcurrent schemaとmigration audit eventを保存する。

mode / model の欠損・未知値を `Edit` 等へ既定化せず、scope、field、raw payload、resolution id、allowed actions を持つ `NeedsConfigurationResolution` として turn / queue drain / workflow resume を block する。migration 対象は SessionMeta だけでなく、既存 queue item、workflow definition、`WorkflowExecutionStarted` snapshot、Tauri / WebSocket DTO を含む。legacy WorkflowExecution に復元可能な snapshot が無い場合も `Edit` へ戻さず、`WorkflowWaitingConfiguration` に置く。

### 9.5 Local atomic event transaction

launch / Session / Goal / workflow / queueを跨ぐ「同じlocal atomic batch」は説明上の比喩ではなく、domainの`LocalEventTransactionRepository` portの背後にあるRust-owned `LocalEventTransactionStore`の1 transactionを意味する。新しいexecution-affecting eventを独立JSON logへ順番にappendしてatomic扱いしてはならない。transaction/schema coreはF3 #1385、history-independent commitはF9 #1494、bounded readはF8 #1491 / F10 #1497が所有する。

```rust
pub enum AgentSessionStreamKind {
    Session,
    Turn,
    LaunchAttempt,
    Goal,
    Queue,
    BypassChallenge,
}

pub enum WorkflowStreamKind {
    WorkflowExecution,
    NodeExecution,
}

pub struct EventStreamKey<Kind> {
    pub kind: Kind,
    pub id: String,
}

pub type AgentSessionStreamKey = EventStreamKey<AgentSessionStreamKind>;
pub type WorkflowStreamKey = EventStreamKey<WorkflowStreamKind>;

pub enum LocalEventStreamKey {
    AgentSession(AgentSessionStreamKey),
    Workflow(WorkflowStreamKey),
}

pub enum LocalEventContext {
    AgentSession,
    Workflow,
}

pub struct AtomicStreamAppend<Key, Event> {
    pub stream: Key,
    pub expected_head_seq: u64,
    pub events: Vec<Event>, // bounded context が定義する closed domain event。JSON/serde 型ではない
}

pub enum LocalAtomicParticipant {
    // launch / Session / Goal / queue stream。event namespace は AgentSession context 内で閉じる。
    AgentSession(AtomicStreamAppend<AgentSessionStreamKey, AgentSessionDomainEvent>),
    // WorkflowExecution / NodeExecution stream。AgentSessionDomainEvent へ結合しない。
    Workflow(AtomicStreamAppend<WorkflowStreamKey, WorkflowDomainEvent>),
}

pub struct LocalAtomicBatch {
    pub batch_id: String,
    pub idempotency_key: String,
    pub participants: Vec<LocalAtomicParticipant>,
}

pub struct LocalAtomicParticipantCommitted {
    pub stream: LocalEventStreamKey,
    pub previous_head_seq: u64,
    pub committed_head_seq: u64,
    pub event_count: u32,
}

pub struct LocalAtomicBatchCommitted {
    pub batch_id: String,
    pub idempotency_key: String,
    pub global_commit_seq: u64,
    pub participants: Vec<LocalAtomicParticipantCommitted>,
    pub committed_at: String,
}

pub struct LocalSnapshotBarrier {
    pub global_commit_seq: u64,
}

pub struct LocalReadSourceKey {
    pub context: String,
    pub source: String,
}

pub struct LocalSnapshotRequest {
    pub required_sources: Vec<LocalReadSourceKey>,
}

pub struct LocalSnapshotLease {
    pub lease_id: String,
    pub barrier: LocalSnapshotBarrier,
    pub required_sources: Vec<LocalReadSourceKey>,
    pub expires_at: String,
}

pub struct LocalWatchRequest {
    pub watch_key: String,
    pub streams: Vec<LocalEventStreamKey>,
    pub snapshot: LocalSnapshotRequest,
    pub after_surface_seq: Option<u64>,
}

pub enum LocalWatchBootstrapPlan {
    Replay { after_global_commit_seq: u64 },
    SnapshotRequired,
}

pub struct LocalWatchFence {
    pub snapshot: LocalSnapshotLease,
    pub bootstrap: LocalWatchBootstrapPlan,
}

pub struct LocalWatchStreamAdvance {
    pub stream: LocalEventStreamKey,
    pub committed_head_seq: u64,
}

pub struct LocalWatchCommitNotice {
    pub global_commit_seq: u64,
    pub streams: Vec<LocalWatchStreamAdvance>,
}

pub struct LocalWatchUpdateFence {
    subscription_id: String,
    update_id: String,
    snapshot: LocalSnapshotLease,
    notice: LocalWatchCommitNotice,
}

impl LocalWatchUpdateFence {
    pub fn snapshot(&self) -> &LocalSnapshotLease { &self.snapshot }
    pub fn notice(&self) -> &LocalWatchCommitNotice { &self.notice }
}

pub enum LocalPersistenceFailureKind {
    Busy,
    CapacityExceeded,
    DeadlineExceeded,
    ReadOnly,
    NoSpace,
    PermissionDenied,
    Corrupt,
    SchemaTooNew,
    MigrationBlocked,
    Io,
}

pub struct LocalPersistenceFailure {
    pub kind: LocalPersistenceFailureKind,
    pub retryable: bool,
    pub retry_after_ms: Option<u64>,
    pub correlation_id: String,
}

pub enum LocalEventTransactionError {
    HeadConflict { stream: LocalEventStreamKey },
    IdempotencyConflict { idempotency_key: String },
    StreamContextMismatch {
        stream: LocalEventStreamKey,
        participant_context: LocalEventContext,
    },
    DuplicateStreamParticipant { stream: LocalEventStreamKey },
    SnapshotExpired { lease_id: String },
    SnapshotSourceNotCovered { source: LocalReadSourceKey },
    ProjectionBehind {
        source: LocalReadSourceKey,
        required_global_commit_seq: u64,
        applied_through_global_commit_seq: u64,
    },
    WatchBootstrapNotFinished { subscription_id: String },
    WatchUpdateOutstanding { subscription_id: String, update_id: String },
    WatchFenceMismatch {
        expected_subscription_id: String,
        actual_subscription_id: String,
    },
    WatchLeaseReleased { lease_id: String },
    WatchLagged { resume_after_global_commit_seq: u64 },
    WatchClosed,
    OutcomeUnknown {
        transaction_id: String,
        payload_hash: [u8; 32],
    },
    PersistFailure(LocalPersistenceFailure),
}

#[async_trait::async_trait]
pub trait LocalEventTransactionRepository: Send + Sync {
    async fn commit_batch(&self, batch: LocalAtomicBatch) -> Result<LocalAtomicBatchCommitted, LocalEventTransactionError>;
    async fn acquire_snapshot(&self, request: LocalSnapshotRequest) -> Result<LocalSnapshotLease, LocalEventTransactionError>;
    async fn release_snapshot(&self, lease: LocalSnapshotLease) -> Result<(), LocalEventTransactionError>;
}

// AgentSession / launch / workflow等の各read repositoryが、この契約を具体query/output型で実装する。
#[async_trait::async_trait]
pub trait LocalCommittedReadRepository<Query, Output>: Send + Sync {
    async fn read_at(&self, snapshot: &LocalSnapshotLease, query: Query) -> Result<Output, LocalEventTransactionError>;
}

pub struct LocalWatchHandle {
    subscription_id: String, // gateway-owned receiverを指すopaque token
    fence: LocalWatchFence,
}

impl LocalWatchHandle {
    pub fn bootstrap_fence(&self) -> &LocalWatchFence { &self.fence }
}

#[async_trait::async_trait]
pub trait LocalWatchRepository: Send + Sync {
    async fn open_watch(&self, request: LocalWatchRequest) -> Result<LocalWatchHandle, LocalEventTransactionError>;
    async fn finish_bootstrap(&self, handle: &mut LocalWatchHandle) -> Result<(), LocalEventTransactionError>;
    async fn receive(&self, handle: &mut LocalWatchHandle) -> Result<LocalWatchUpdateFence, LocalEventTransactionError>;
    async fn finish_update(&self, handle: &mut LocalWatchHandle, update: LocalWatchUpdateFence) -> Result<(), LocalEventTransactionError>;
    async fn close_watch(&self, handle: LocalWatchHandle) -> Result<(), LocalEventTransactionError>;
}
```

- `LocalPersistenceFailureKind`はcommit前と確定できるsafe classificationだけを持ち、`OutcomeUnknown`を含めない。`commit_batch`がwriterを開始した後にcommit成否が不明になった場合だけ、transaction ID、payload hashの順を必須にする`LocalEventTransactionError::OutcomeUnknown { transaction_id: String, payload_hash: [u8; 32] }`へ写像する。`acquire_snapshot / release_snapshot / read_at / open_watch / finish_bootstrap / receive / finish_update / close_watch`はmutation commit authorityを持たないためこのvariantを生成しない。transaction / closureのunknownを`PersistFailure(LocalPersistenceFailure)`または`Phase0ClosureError::BeforeCommit(LocalPersistenceFailure)`へ格下げせず、same transaction identityのbounded outcome lookupで解決する。
- domainに定義する非genericな`LocalEventTransactionRepository` traitが、非同期の`commit_batch(LocalAtomicBatch) -> Result<LocalAtomicBatchCommitted, LocalEventTransactionError>`と`acquire_snapshot / release_snapshot`を内向きwrite/snapshot portとして公開する。read側はcontext-specificな`LocalCommittedReadRepository::read_at`と`LocalWatchRepository::open_watch`を使う。3 portは現行gateway群と同じ`#[async_trait::async_trait]`かつ`Send + Sync`でobject-safeにし、`Arc<dyn LocalEventTransactionRepository>`、具体`Query / Output`を束縛した`Arc<dyn LocalCommittedReadRepository<..>>`、`Arc<dyn LocalWatchRepository>`としてtask間共有できる。`LocalAtomicParticipant`はcontextごとの`AtomicStreamAppend<Key, Event>`を明示的なsum typeで包むため、同じbatchにheterogeneous participantを入れても各`Event`はclosed domain event型のままである。usecase/query serviceはJSON payload、serde、schema version、SQLite、WALを参照しない。
- launch / Session / Goal / queue eventはdomain-ownedの`AgentSessionDomainEvent`、workflow eventはdomain-ownedの`WorkflowDomainEvent`として別namespaceを保つ。`docs/architecture/GLOSSARY.md`で使用禁止の`WorkflowEvent`をdomain語として採用せず、巨大な共通domain event enumへのvariant併合、JSON/type erasure、既存outer-layer schema型の流用を禁止する。新しいbounded contextをtransactionへ参加させるときは`LocalAtomicParticipant`へ明示variantを追加し、repository実装のexhaustive mappingを要求する。
- 現行の`usecase/agent_session/event_log/events.rs::AgentSessionEvent`と`adaptor/gateway/workflow/event.rs::WorkflowEvent`はserde、表示用usecase型、`WorkflowDefinitionYaml`、`serde_json::Value`を含む**legacy persistence schema**であり、domain portへ渡さない。`adaptor/gateway`のrepository実装が`AgentSessionDomainEvent / WorkflowDomainEvent`をvariantごとにmatchし、`PersistenceEventEnvelope { event_type, schema_version, serialized_payload }` command modelへ変換して`infrastructure`のSQLite/WAL transaction clientを呼ぶ。F7/L12 migrationは旧schemaをgatewayでlazy upcastしてdomain projection入力へ変換し、順序を保って新storeへidempotent importする。移行後は旧logへdual-writeせず、未知の旧event/fieldはV-D11に従ってraw保全する。`LocalEventTransactionStore`はこのportの背後のdurable機構であり、usecaseから直接参照しない。
- `AgentSessionStreamKey`は`AgentSessionStreamKind`だけ、`WorkflowStreamKey`は`WorkflowStreamKind`だけを受け取るため、participant variantとstream contextの不一致は通常のdomain構築では型上表現できない。taggedな`LocalEventStreamKey`のcontext variant＋closed kind＋idがgatewayのCAS namespace/headを一意に決める。legacy upcastやcorrupt persistenceなど型境界外から不一致が到達した場合は`StreamContextMismatch`としてrejectし、別namespaceへ推測fallbackしない。
- `commit_batch`は同一`LocalEventStreamKey`を1 batchへ複数回含めることを許可せず、`DuplicateStreamParticipant`でbatch全体をrejectする。したがって1 streamのevent連結順と`expected_head_seq`は単一participantだけが所有し、gatewayは全participantのheadを一度ずつCASしてper-stream seqとglobal commit seqを割当て、typed event payload、batch id、idempotency key、head更新を単一のdurable transactionでcommitする。constructorは重複を早期拒否してよいが、`commit_batch`もこの不変条件を必ず検査する。`LocalAtomicBatchCommitted`はevent payloadを内包・appendしない非再帰なcommit receiptである。SQLite WAL等の実transactionを使い、participant logへの逐次append＋補償で代用しない。
- commit前のbatchはどのquery/projector/watchにも見せない。crashがcommit前なら0件、commit後なら全participantが見える。`batch_id/idempotency_key`の再実行は同じ結果を返し、異なるpayloadならconflictにする。別のPrepared/Committed二相状態を外へ露出しない。
- per-stream event log/read model/cacheはcommitted transactionから再構築するprojection/indexである。event rowとprojection versionは`global_commit_seq`を保持し、各projection sourceは自分に無関係なcommitもskip済みとして順番にconsumeし、全commitを処理またはskipした連続上限だけを`applied_through_global_commit_seq` watermarkとしてdurableに進める。current rowの上書きだけにせずsnapshot以下の最新版を読めるようにする。`acquire_snapshot(LocalSnapshotRequest)`はcommitted global headと全`required_sources`のwatermarkの最小値である**common readable watermark**以下にだけbarrierを置く。同じbatchのeventだけが見えてprojectionが未適用のbarrierは発行しない。`LocalSnapshotLease`はそのbarrierとsource集合を有効期限までpinし、gatewayは未失効leaseの最小barrierをGC horizonとして、そのbarrierを満たすために必要なevent/projection versionをpruneしない。明示releaseはidempotentとし、未release leaseも`expires_at`後に回収できるため、全過去versionのfull-retentionを要求しない。
- AgentSession / launch / workflow等の各context-specific read-side portは、event・projectionを読む全methodを`read_at(&LocalSnapshotLease, query)`契約にする。各portは自sourceがleaseの`required_sources`に含まれ、`applied_through_global_commit_seq >= lease.barrier.global_commit_seq`であることを検証する。source未列挙ならnon-retryableな`SnapshotSourceNotCovered`、追随していなければ`ProjectionBehind`を返してstale projectionを成功扱いしない。複数repositoryを合成するquery serviceは必要な全sourceを列挙して最初に1回だけleaseを取得し、全committed sourceへ同じleaseを渡し、成功・失敗を問わず最後にreleaseする。途中の1 sourceでも`SnapshotExpired`または`ProjectionBehind`になった場合は部分結果を捨て、projection追随をbounded waitした後にfresh leaseで**query全体**をbounded retryする。retry上限後はretryable errorを返し、source単位の再読込やlatest readとの混在、snapshot超のprojection返却を禁止する。capability / managed policyは同じquery evaluation contextで評価し、そのcontext hashとchecked-atを結果へ固定する。
- provider I/O前のintent batchはcommit成功後だけ送信可能。provider ack後のcanonical batchがcommitできなければ、旧stateへ戻ったふりをせず外部observation付きreconciliationへ進む。
- `get_session`、`get_agent_launch`、workflow queryは全sourceを同じsnapshot leaseの`read_at`で読み、同じbatchの一部だけを描画しない。surface固有seqはcommitted batchのper-stream seqから導出する。
- watch開始は`open_watch`が1つのstorage transaction / commit lock内で、surface cursorからreplay可否を決め、requestの全`required_sources`が追随したcommon readable watermarkへsnapshot leaseを取得し、そのbarrierより後のcommitを受けるsubscription/receiverを登録したopaque token付き`LocalWatchHandle`を返す。handle/fenceのfieldは非公開とし、usecase-owned watch serviceはread-only accessorで得たbootstrap fenceだけからsnapshotまたはreplayをquery serviceに構築させ、送信可能なtyped frameへmaterializeした後に同じportの`finish_bootstrap`を呼ぶ。以後`receive`は次のnoticeだけを返さず、全required sourceが当該`global_commit_seq`まで追随した時点で、そのcommitへ**厳密にpinした**leaseを持つ`LocalWatchUpdateFence`を返す。各update fenceは発行元handleの`subscription_id`へ内部的に束縛し、`finish_update`は別handle由来のfenceを`WatchFenceMismatch`で拒否する。watch serviceはaccessorから得たleaseだけでtyped projection/deltaを構築し、同じfenceの`finish_update`でleaseをreleaseしてから次を受ける。receiver registryはgatewayが所有し、usecase/controllerはtokenからregistryやbroadcasterへ直接到達しない。snapshot取得とsubscription登録、notice受信とcommit固定lease取得を別callにしない。
- watch phase不変条件は最低限gatewayがsubscription id単位にruntime enforceする。順序は`bootstrap -> active -> pending-update -> active`であり、bootstrap未完了中の`receive`は`WatchBootstrapNotFinished`、未finishのupdateがある間の次の`receive`は`WatchUpdateOutstanding`、release済みbootstrap/live leaseを`read_at`・`finish_*`・別updateに再利用する操作は`WatchLeaseReleased`でrejectする。実装はこの契約より強いtypestateのconsume型遷移を採用してよいが、同一mutable handleだけを使う実装でもこれらのruntime検査と明示errorを省略してはならない。
- subscription bufferはgateway-ownedの件数・byte上限を持ち、bootstrap中、projection追随待ち、live配信中のいずれもcommit処理をblockしない。上限超過、receiver lag、projection追随のbounded timeoutは通知を黙って捨てず`WatchLagged { resume_after_global_commit_seq }`でsubscriptionをterminalにして登録を解放し、watch serviceは部分bootstrap/deltaを捨てて新しいhandleを開き直す（cursorがretention外ならfull snapshot）。bootstrap中のlease失効、live fenceの`SnapshotExpired / ProjectionBehind`でも`close_watch`してwatch全体をやり直す。
- client disconnect、handler cancel、正常終了を受けたwatch serviceは同じportの`close_watch`を必ず呼ぶ。`close_watch`はhandleをclosedへmarkしてreceiver/subscriptionをregistryから解除し新規enqueueを止め、次にbounded bufferをdrain/dropし、次にoutstanding live update lease、最後に未release bootstrap leaseを解放して、発行済みfence/tokenを無効化する順で全resourceを回収する。この順序と結果はidempotentであり、途中までclose済みでも残りを回収する。serviceのcancellation guardもcloseを起動し、handlerはservice taskをcancelするだけでportを直接呼ばない。process crash時はin-memory receiver registry自体が破棄され、未release leaseは期限で回収される。これにより切断したsubscriptionをgateway registryへ残さず、get→subscribe間の欠落、無制限buffer、commitへのbackpressureを同時に避ける。
- `LocalPersistenceFailure`はsafe classificationだけをdomain/usecaseへ返す。raw SQLite / filesystem error、path、SQL、event payloadはcorrelation ID付き構造化logにだけ残し、UIやdomain errorへ文字列で流さない。`OutcomeUnknown`はidempotency lookupによるcommit解決専用で、provider side effectのblind retryを許可しない。

#### Phase 0 closure bridge（#1499）

```rust
pub struct AppDataGenerationId(String);

pub struct AgentOperationBindingKeyV1 {
    pub schema_version: u16, // 1
    pub app_data_generation_id: AppDataGenerationId,
    pub hmac_sha256_key: [u8; 32],
    pub created_at_utc: String,
}

pub struct StopCallerRequestBindingRecordV1 {
    pub principal_id: String,
    pub app_data_generation_id: AppDataGenerationId,
    pub request_id: String,
    pub operation_id: StopOperationId,
    pub exact_request_binding_hmac_sha256: [u8; 32],
    pub revision: u64,
}

pub struct ApplicationQuitCallerRequestBindingRecordV1 {
    pub principal_id: String,
    pub app_data_generation_id: AppDataGenerationId,
    pub request_id: String,
    pub operation_id: ApplicationQuitOperationId,
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

pub enum ApplicationQuitOperationLocatorV1 {
    ShutdownPlan { plan_id: String, epoch: u64 },
    BootstrapFlight { bootstrap_id: String },
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
    ReconciliationRequired { failure: SafeOperationFailure },
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

pub struct LegacyBootstrapCursorV1 {
    pub source_entry_ordinal: u64,
    pub source_entry_id: String,
    pub source_record_ordinal: u64,
    pub substep_ordinal: u64,
}

pub struct Phase0AuthorityPointerV1 {
    pub revision: u64,
    pub authority: Phase0AuthorityV1,
}

pub enum Phase0AuthorityV1 {
    Legacy {
        app_data_generation_id: AppDataGenerationId,
    },
    Phase0 {
        app_data_generation_id: AppDataGenerationId,
        agent_operation_binding_key_record_sha256: [u8; 32],
        transaction_inventory_revision: u64,
        transaction_inventory_root_sha256: [u8; 32],
        activated_bootstrap_id: String,
        parity_manifest_sha256: [u8; 32],
    },
}

pub struct Phase0BootstrapStateV1 {
    pub bootstrap_id: String,
    pub source_generation_id: AppDataGenerationId,
    pub staging_generation_id: AppDataGenerationId,
    pub source_inventory_sha256: [u8; 32],
    pub stage: Phase0BootstrapStageV1,
    pub next_source_cursor: Option<LegacyBootstrapCursorV1>,
    pub imported_source_count: u64,
    pub imported_logical_record_count: u64,
    pub oversized_copy: Option<LegacyRawCopyProgressV1>,
    pub staged_transaction_inventory_revision: u64,
    pub staged_transaction_inventory_root_sha256: [u8; 32],
    pub staged_public_projection_sha256: [u8; 32],
    pub revision: u64,
}

pub struct LegacyRawCopyProgressV1 {
    pub source_entry_id: String,
    pub next_byte_offset: u64,
    pub expected_byte_len: u64,
    pub expected_sha256: [u8; 32],
    pub rolling_sha256: Sha256StreamingCheckpointV1,
    pub staging_blob_ref: String,
}

pub struct Sha256StreamingCheckpointV1 {
    pub algorithm_version: u16, // 1 = SHA-256/FIPS 180-4
    pub chaining_state_be: [u32; 8],
    pub processed_full_block_bytes: u64,
    pub pending_tail: Vec<u8>, // 0..=63 bytes
}

pub enum Phase0BootstrapStageV1 {
    InventoryFixed,
    Importing,
    Verifying,
    ReadyToActivate,
    Activated,
    Failed { failure: SafeOperationFailure },
}

pub struct Phase0BootstrapParityManifestV1 {
    pub bootstrap_id: String,
    pub source_generation_id: AppDataGenerationId,
    pub source_inventory_sha256: [u8; 32],
    pub source_scope_count: u64,
    pub source_logical_record_count: u64,
    pub quarantined_scope_count: u64,
    pub staged_logical_record_count: u64,
    pub source_public_projection_sha256: [u8; 32],
    pub staged_public_projection_sha256: [u8; 32],
    pub unknown_event_bytes_sha256: [u8; 32],
    pub quarantined_raw_bytes_sha256: [u8; 32],
    pub recovery_action_token_registry_sha256: [u8; 32],
}

pub struct LegacyScopeQuarantinedV1 {
    pub scope: LegacyQuarantineScopeV1,
    pub source_entry_ids: Vec<String>,
    pub raw_source_refs: Vec<LegacyRawSourceRefV1>,
    pub failure: SafeOperationFailure,
    pub revision: u64,
}

pub enum LegacyQuarantineScopeV1 {
    Session { session_id: String },
    Workflow { workflow_execution_id: String },
    OrphanRuntime { runtime_instance_id: String },
}

pub struct LegacyRawSourceRefV1 {
    pub source_entry_id: String,
    pub byte_len: u64,
    pub sha256: [u8; 32],
    pub immutable_blob_ref: String,
    pub owner_access_policy_sha256: [u8; 32],
}

pub struct Phase0BootstrapProjection {
    pub bootstrap_id: String,
    pub phase: Phase0BootstrapPublicPhase,
    pub imported_source_count: u64,
    pub total_source_count: Option<u64>,
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

pub struct Phase0ReadSnapshotRef {
    pub lease_id: String,
    pub transaction_inventory_revision: u64,
    pub transaction_inventory_root_sha256: [u8; 32],
    pub pending_inventory_revision: u64,
    pub pending_inventory_root_sha256: [u8; 32],
    pub obligation_index_revision: u64,
    pub obligation_index_root_sha256: [u8; 32],
    pub latest_activated_pointer_revision: u64,
    pub latest_activated_pointer_sha256: [u8; 32],
}

pub struct Phase0ShutdownReadSnapshotRefV1 {
    pub base: Phase0ReadSnapshotRef,
    pub latest_attempt_pointer_revision: u64,
    pub latest_attempt_pointer_sha256: [u8; 32],
    pub shutdown_scope_fence_revision: u64,
    pub shutdown_scope_fence_root_sha256: [u8; 32],
}
```

caller request binding indexはoperation kindごとに`(current-installation principal, app_data_generation_id, request_id)`を一意keyとし、同じacceptance closureでcaller keyからbackend発行operation IDへの写像を確定する。Stop / quit / SessionLifecycleのexact bindingはそれぞれ次の式だけを使う。

- Stop: `HMAC-SHA256(key, LP("stop-operation-exact-request-binding/v1") || LP(principal_id) || LP(app_data_generation_id) || LP(request_id) || LP(operation_id) || LP(canonical_stop_command_bytes))`
- quit: `HMAC-SHA256(key, LP("application-quit-exact-request-binding/v1") || LP(principal_id) || LP(app_data_generation_id) || LP(request_id) || LP(operation_id) || LP(canonical_quit_command_bytes))`
- SessionLifecycle: `HMAC-SHA256(key, LP("session-lifecycle-exact-request-binding/v1") || LP(principal_id) || LP(app_data_generation_id) || LP(request_id) || LP(operation_id) || LP(canonical_lifecycle_command_bytes))`

`canonical_stop_command_bytes = LP(session_id) || LP(target_turn_id) || U64BE(expected_session_revision)`、`canonical_quit_command_bytes = LP("exit" | "restart") || I32BE(code)`、`canonical_lifecycle_command_bytes = LP(session_id) || U64BE(expected_session_revision) || LP(action_tag) || LP("none" | "some") || [Someの場合だけLP(backend_id)]`である。Close / ArchiveOpen / ArchiveClosedは`none`かつbackend bytes 0、SwitchBackendは`some`かつvalidated nonempty backend ID exactly oneを必須とする。`I32BE`はsigned i32のtwo's-complement big-endian 4 bytes、`LP`はu32 BE lengthとraw bytesである。principal、request ID、operation ID、generationはcanonical binding recordとHMAC envelopeが所有し、inner command bytesへ重複して含めない。decode時はrecordのprincipal / generation / request / operationとdeterministic path preimageをbyte一致させる。`AgentOperationBindingKeyV1`はsend / Stop / quit / SessionLifecycleの4 domainが別domain prefixで共用する、app-data generationごとexactly oneのimmutable owner-only 32-byte authorityである。canonical record全bytesのSHA-256をPhase 0 authority pointer / F3 owner-only store stateのverifierとして保持し、open / startup / import / backup / restoreでfileのdecode→re-encode hashを照合する。fixed KATだけでrandom current keyの真正性を判断せず、verifierはmanifest / filename / public DTO / logへ出さない。生成後に同generation内で再採番せず、missing / duplicate / generation mismatch / ACL failure時は4 commandのadmissionを閉じ、default keyを生成しない。constant-time比較し、key、key digest、bindingをpublic DTO、log、telemetryへ出さない。別request IDが既存flightへjoinする場合も、そのcallerが提示したexact commandと同じbackend operation IDを新しいbinding recordへ保存する。same caller keyのretryはbinding recordを先に引き、writer結果不明なら同じlogical caller slotのtransactionをresolveしてからpayload conflictまたは保存済みdecisionを返す。

binding codecの固定KATは次の3本で、preimage bytesとdigestの両方をunit testで固定する。

- Stop: key bytes `00..1f`、principal `principal_1`、generation `app_1`、request `stop_req_1`、backend operation `stop_op_1`、session `session_1`、turn `turn_1`、expected revision `1` → canonical preimage 129 bytes、HMAC-SHA256 `9aea744029168a755e77bf7fa763f84df36b2167f7b1bc7fc727e75a26590d3c`
- quit: key bytes `00..1f`、principal `principal_1`、generation `app_1`、request `quit_req_1`、backend operation `quit_op_1`、mode `exit`、exit code `0` → canonical preimage 112 bytes、HMAC-SHA256 `6a34bd12ce2691c1912e31d4e0f797cd51e28a67fdf5dc03714f18782e49dfda`
- SessionLifecycle: key bytes `00..1f`、principal `principal_1`、generation `app_1`、request `lifecycle_req_1`、backend operation `lifecycle_op_1`、session `session_1`、expected revision `1`、action `close`、backend option `none` → inner command 38 bytes、canonical preimage 149 bytes、HMAC-SHA256 `b623c791f1a3f40579ba9713507ab507bdc844dee12d95e4408d673b17eb2217`

`StopOperationRecordV1`はbackend `StopOperationId`を正本keyとし、stored `terminal_commit_obligation_id`で同IDをStop専用`TerminalCommit` obligationへ一対一に写す。Accepted closureでexactly one obligationと同時に確定し、import / replayはIDの一意性とkind / owner / target parityを検証する。`Accepted / ReconciliationRequired`だけ`deadline_permit=Some`、`Terminal`だけNoneであり、terminal stateはresolutionと同じ`TurnResult`を必須にする。`OutcomeUnknown`は保存stateへ追加せず、未解決transactionからqueryで導出する。application quitのoperation direct indexはbackend `ApplicationQuitOperationId`から`ApplicationQuitOperationLocatorV1`を一件だけ返す。normal quitはlive rootまたはimmutable compact archiveのclosed unionで解決する`ShutdownPlan`、bootstrap-safe quitは`BootstrapFlight`へ分岐し、caller request IDやbootstrap IDをbackend operation IDとして流用しない。archive-only normal locatorはTerminal / Compactedを返し、liveとarchive双方があればsource root pair、intent、terminal phase、summary / counts / deadline / failureのsemantic parityを必須にする。双方不在または不一致はInternalであり、`Current(None)`や別planへfallbackしない。

`LegacyBootstrapCursorV1`はfixed source inventory内の**次に未commitのlogical unit**を示す。canonical bytesは`LP("legacy-bootstrap-cursor/v1") || U64BE(source_entry_ordinal) || LP(source_entry_id) || U64BE(source_record_ordinal) || U64BE(substep_ordinal)`で、`source_entry_id`は非正規化raw UTF-8 1..=1024 bytes、3 ordinalは0..=9223372036854775807である。decode時はenclosing `source_inventory_sha256`のordered entryを`source_entry_ordinal`で一件引き、そのstable IDと`source_entry_id`をbyte一致させる。cursorはsource-level finalize closureが全substepを確定した後だけ次record / entryへ進め、partial substepでは同じentry / recordの次substepだけへ進める。`next_source_cursor=None`はfixed inventoryが空または全source finalize済みの場合だけで、unknown / decode failure / hash failureを完了へ読み替えない。

F3のSQLite store導入前に#1499が使うschema-versioned redo manifestは、現行file storeの`Session`、`Workflow`、`Application`各scopeをcrash後に全件materializeする互換bridgeであり、`LocalAtomicBatch`または汎用multi-stream event storeではない。Session scopeではmanifestとnew transaction-inventory COW pageを同一filesystemへwrite / file sync / no-replace publishし、必要なancestorをsyncした後、fixed transaction-inventory rootをexpected revision付きでCASする。manifest / COW page / root hashを再読込検証し、root file、root parent、必要な新規ancestorのrequired syncが全て成功した時点だけをcommit pointとする。root未到達のmanifest file単体だけでなく、visible / reachableだがrequired sync未確認のrootもcommittedではない。single `send_agent_message`のacceptance closureはcaller指定operation IDへ束縛したexact payload、immutable receipt、initial execution status、human input、turnまたはqueue、必要なobligationを一つのtransactionとしてoperation direct recordへ確定する。canonical writer開始前の`RejectedBeforeCommit`はdurable operation viewを作らず、writer開始後の結果不明は同operation IDの`OutcomeUnknown`として同transactionを解決する。operation / terminal / obligation direct record、human / assistant message、event、meta / private / index、queue pause、publication markerの必要participantを完全に含める。未materialize manifestはlogical recordごとに最大1件へ制限し、queryはtransaction-inventory rootから到達しdurability確認済みのdeterministic overlay slotだけをdirect recordへ重ねる。send operationはoperation overlay＋direct recordとordered obligation ID最大4件、dependency判定はobligation overlay＋direct result最大3件だけで解決し、query前のmaterialize成功を要求しない。BackendRecovery payloadはprovider recovery effectとRecoveryPublication obligation IDを所有するが、publication message identity / payload / markerは独立RecoveryPublication manifestだけが所有する。

#1499の実装完了gateはPhase 0 root / manifest / participant fault injectionとTauri / WebSocket parityまでである。Phase 0→F3 one-shot cutoverの契約とverificationはD3で確定するが、そのruntime実装・実行は#1499へ含めずF3 #1385の完了gateとする。

Phase 0 bridgeを初めて導入するupgradeは明示migration commandを要求しない。startupはexclusive app-data writer lockを取得し、public mutation、provider / workflow effect、recovery action、Session close、backend switch、normal application shutdown admissionを閉じたまま`Phase0AuthorityPointerV1`をdirect lookupする。bootstrap-safe quit ingressだけは後述のspecial bounded exitとして受理する。pointer未作成はcurrent generationのLegacyを意味するがlegacyへ既定pointerを書き戻さず、専用bootstrap namespaceへimmutable source inventory、state、batch manifest、staging generation、parity manifestをno-replace publishする。legacy session / workflow bytesは一切変更せず、stagingはauthority pointerが切り替わるまでnormal query / mutation authorityではない。

bootstrap source inventoryはordered source path / kind / stable identity / byte length / content SHA-256のMerkle rootへ固定する。pure Rust normalizationのread batchは最大200 source logical recordsまたはdecoded source bytes 16 MiBの先到達側で閉じ、canonical staged payloadをこの16 MiBへ合算しない。staged mutation candidateは別のphysical K tierへ収め、最大K4 17 MiB、normal lane 64 MiBの同時収容上限を独立に検査する。一つのsourceを一candidateで処理できない場合はsource identity / substep ordinal / payload hashから決まるdeterministic substepへ分割し、cursor未前進のmicrocommitとしてcrash-resumableに確定する。全substepとsource-level parityをfinalizeしたcommitだけがnext source cursor、`imported_source_count`、`imported_logical_record_count`を進める。partial substep / staging projectionはpublic query / parity authorityへ出さない。crash / response loss後はexclusive lock下でsource inventory、既存state、完了substep / batch、staged rootをcanonical decode / hash検証し、同じsource / substep IDを再利用して最初の未完了substepから再開する。source変更、global inventory corruption、parity不一致はstagingを昇格せずFailedとしてmutation / effect admissionを閉じる。単一candidateのK4超過、構造破損、missing ref、same stable key different payloadは最小Session / Workflow / orphan scopeだけを`LegacyScopeQuarantinedV1`へ写す。source inventory entryとbyte一致するexact legacy raw bytesをowner-private random-ID immutable blobへ保持し、public view / log / telemetryへpath、bytes、digest、blob refを出さない。

quarantine raw copy / hashは1 MiB chunk、1 bootstrap step合計16 MiBでyieldし、stateへsource entry、byte offset、versioned SHA-256 checkpointを保存する。各chunkをbootstrap ID / source entry / ordinal / random blob identityでno-replace publish・syncし、crash後はcommitted progressより先のchunkをauthorityにせず、offset / processed bytes / tail / staging lengthを検証して同じordinalから再開する。完成時はordered chunkをrandom identityのimmutable blobへassembleしながらfinal byte length / expected SHA-256 / owner access policyを再検証し、no-replace publish / sync後だけquarantine recordをcommitする。raw bytesを完全保存できない場合はscopeだけを省略せずbootstrap全体をFailedにする。`quarantined_raw_bytes_sha256`はquarantine scope / source entry順のexact raw bytesをdomain-separated ordered hashへ固定する。quarantine raw refはlegacy sourceのparity検証、Phase 0 activation、F3 one-shot importに必要なretentionが全て完了するまでGCしない。
legacy normalizationは証拠を補完しない。terminal reason / final parts / assistant message、permission exact bytes、known event、unknown tag / field raw bytesはsourceとbyte-equivalentに保持する。外部作用開始 / 結果を一意に証明できないactive turnはsame identityのReconciliationRequired、runtime / configuration guardを証明できないqueued itemはPaused＋ReconciliationRequired、exact private permission payloadを保持できないidentityはFailedまたはscope quarantineへ写し、provider start / resume / interrupt / response / queue drainを自動実行しない。closed / archived / workflow-owned relationを維持し、自動reopenしない。

全batch後にsource / staged scope・logical counts、scope membership、terminal uniqueness、closed normalization rulesを同じlegacy bytesへ適用して得たexpected normalized message / permission / queue public projection、unknown raw bytes digest、ordered quarantined raw bytes digest、全COW root / direct materialization parityを検証する。legacy画面の偶発的な旧表示との文字列一致をparityにしない。確定済みtranscript / terminal / owner relationは維持する一方、証明不能active turn / queue / permissionは上記rulesどおりReconciliationRequired / Paused＋ReconciliationRequired / Failedまたはquarantineへ保守変換したexpected projectionと比較する。一致した`Phase0BootstrapParityManifestV1`とstaged root / ancestorをrequired syncした場合だけ、expected Legacy revisionからexact staged generation / root / parity hashを指すPhase0 authorityへpointerをatomic CASし、pointer file / parentをsync / readbackする。このpointer CASだけがauthority切替のcommit pointである。CAS前crashはlegacy、CAS後response lossはpointer / root / syncの再確認でPhase0へ収束する。切替前queryは固定legacy sourceのread-only projectionだけ、切替後はPhase0 reader / writerだけを使い、per-record legacy fallback、query merge、legacy write-back、dual write、最初のPhase0 live mutation後のlegacy rollbackを禁止する。

Tauri `get_phase0_bootstrap()`とWebSocket `GetPhase0Bootstrap`は同じquery / presenterから`Option<Phase0BootstrapProjection>`を返す。authority pointer未作成またはLegacyでbootstrap未開始なら`InspectingSource`、batch import中は`Importing`、parity検証中は`Verifying`、pointer CAS writer開始からPhase0 pointer / root / parity検証、reachable manifest replay、pending inventory validation、normal read / mutation admission openまで同じbootstrap IDの`Activating`、安全に続行不能なら`Failed`であり、全phaseで`read_only=true`とする。`Failed`だけが`safe_failure=Some`、他phaseはNoneを必須とし、imported / optional total countをfrontendで合成しない。上記post-CAS validationを完了してnormal admissionを開けた通常起動だけがNoneであり、pointer CAS成功直後、storage / decode / pointer OutcomeUnknownをNoneへ変換しない。このqueryはnormal Phase 0 mutation admissionが閉じていても利用でき、raw source path / bytes / quarantine detailを返さずbootstrap stateも変更しない。

bootstrap projectionがSomeの間のapplication quitは無効化せず、quit ingress時刻をT0とするbootstrap-safe bounded exitへ送る。normal shutdown plan / target / obligation、明示agent / workflow shutdown commandを作らず、stop-after-current-bootstrap-stepを設定する。受理closureはbackend発行のopaque `ApplicationQuitOperationId`、caller binding、`ApplicationQuitOperationDirectRecordV1 { locator: ApplicationQuitOperationLocatorV1::BootstrapFlight { bootstrap_id }, .. }`、`BootstrapApplicationQuitFlightRecordV1 { state: BootstrapApplicationQuitFlightStateV1::Settling, .. }`を一つのdecisionとして確定し、`ApplicationQuitResult::Accepted { operation_id, current: ApplicationQuitProjection::Bootstrap(...) }`を返す。special bounded flightも最初に受理したtyped ingressのcanonical `ShutdownExitIntent`を固定する。同じprincipal / request ID / same intentは同resultをreplayし、same request ID / different intentはWebSocketの`AgentSessionWsErrorV1::PayloadConflict { identity: ApplicationQuit { request_id } }`またはTauriの`RequestApplicationQuitApplicationError::PayloadConflict { request_id }`としてeffect 0件で返す。別request IDの後続quitはintentが異なってもcurrent flightへjoinし、first accepted intentを上書きしない。その後続caller keyにも提示されたexact intentと同じbackend operation IDのbinding recordを保存する。pointer writer開始前はcurrent batchのCommittedまたはrollback済みBeforeCommitをcheckpointし、開始後は同じpointer attemptへ`T0 + 13s`までjoinする。OutcomeUnknownをLegacy / Phase0へ推測せず、`T0 + 15s`に`ExitPermitV1 { authority: Bootstrap { bootstrap_id }, exit_intent }`でprocess exitする。次bootはpointerとbootstrap stateを先に解決して同じsource cursorから再開し、旧`coordinator_boot_id`の終了を確認したflightを`Exited`へ確定する。`get_application_quit_operation`はoperation direct locatorを一件引いてbootstrap projectionまたは保存結果未解決を返し、normal planをlookupしない。implicit process-exit effectをterminal、ConfirmedNoEffect、bootstrap / migration successへ写像しない。

Phase 0のphysical transaction / idempotency identityは共通builderだけが作る。closed operation-kind ASCII prefix、u16 big-endian component count、各componentのu32 big-endian byte length＋raw canonical bytesをdomain-separated SHA-256へ入力し、`<known-prefix>/<64 lowercase hex>`へ固定する。componentは1..=16件、各1..=1024 bytes、domain prefix / count / lengthを含むpre-hash canonical inputは最大16 KiB、生成後logical keyは1..=1024 bytesである。unsigned numeric componentはu64 big-endian 8 bytes、enumはclosed ASCII tag、Unicodeは非正規化のraw UTF-8とし、schema v1でsigned numericを禁止する。original componentsとpayload hashをmanifestへ別保持する。empty、17件目、1025-byte component、16 KiB超、unknown prefix、signed numeric、生成後1025-byte keyはBeforeCommitまたはimport scope quarantineであり、truncate、`/`直接連結、decimal ASCII、別hash fallbackを禁止する。bootstrap / normal closure / F3 importは同じknown-answer / boundary vectorsを使う。

D3のclosed prefix setは33件であり、SessionLifecycle acceptance専用`session-lifecycle/v1`、caller binding-only join専用`stop-caller-join/v1` / `session-lifecycle-caller-join/v1` / `application-quit-caller-join/v1`、bootstrap flightの後続遷移専用`bootstrap-application-quit-transition/v1`、shutdown compaction専用`shutdown-migration-retire/v1` / `shutdown-archive-switch/v1` / `shutdown-detail-detach/v1` / `shutdown-finalize-detach/v1`を含む。join closureはnew bindingだけを変更し、existing operation / target / first guardまたはquit locator / first intentをread guardにする。bootstrap transition closureだけがexisting flightを`Settling → Exited | ReconciliationRequired`へexpected revision +1で進め、初回acceptance transactionを再利用しない。bootstrap normalization microcommit専用`phase0-bootstrap-normalize/v1`をsynthetic operation用`phase0-legacy-operation/v1`から分離する。bootstrap prefixのcomponentsは`[bootstrap_id, source_entry_id, source_record_ordinal:u64BE(zero-based), substep_ordinal:u64BE(zero-based)]`、KAT `bootstrap_1 / entry_1 / 0 / 0`はcanonical preimage 81 bytes、`phase0-bootstrap-normalize/v1/0a507a9f122a7948b10195a53a487dafacb80d28fd7b64f553b14f4ec4e0fe40`である。legacy operationの4-component / 68-byte KATはapplication / synthetic operation identityだけに残し、bootstrap substepへ流用しない。initial shutdown rootは専用`shutdown-init/v1`の`[app_data_generation_id, application_quit_operation_id, plan_id, epoch:u64BE(one-based), expected_root_revision:u64BE(0), exit_intent_raw32]`だけが作り、Preparing root revision 1とlatest-attempt pointerを同じclosureへ入れる。resource / target guardをtransaction componentにする場合はguard専用domainのcanonical bytesをSHA-256したraw 32 bytesだけを使い、可変full guard bytes、path、display stringをcomponentへ入れない。

全quit ingressはcoordinator admission前にcanonical `ShutdownExitIntent { mode, code }`へ正規化する。Cmd-Q / application menu / tray / Dock / `NativeExitRequested { code: None }`は`Exit / 0`、`NativeExitRequested { code: Some(code) }`は`Exit / code`、typed Internal ingressはreasonが要求する`Exit | Restart`とそのi32 codeへ写す。最初に受理されたintentだけをsame-boot flightへ固定し、complete plan root、coordinator state、summary、current / historical projection、one-shot `ExitPermitV1`までbyte不変で引き継ぐ。同じprincipal / request ID / same intentは同resultをreplayし、same request ID / different intentはWebSocketの`AgentSessionWsErrorV1::PayloadConflict { identity: ApplicationQuit { request_id } }`またはTauriの`RequestApplicationQuitApplicationError::PayloadConflict { request_id }`としてeffect 0件で返す。別request IDでcurrent flightへ到着した後続quitはmode / codeが異なってもfirst accepted flightへjoinし、intentを上書きせず新planを作らない。process flightが無いときもprior latestがnonterminal、またはunresolved shutdown scope-fence rootがnonemptyならexact `PreviousShutdownReconciliationRequired`をAccepted前に返し、new plan identity / root / page / effectを0件にする。prior latestがterminalでnew-flight admission / store / explicit retry guardを満たしscope fenceが0件の場合だけ、Completedまたはglobal activation前にeffect 0件でabortしてdurable Failed / Cancelled fenceへ閉じたflightの後続quitがnew plan / epochとnew intentを採用できる。compact archive / F3 importもrootとsummaryのintent parityを検証し、不一致ならcode 0やExitへfallbackしない。

Workflow scopeではshutdown対象revisionとcommand identity、shutdown obligationを同じscope manifestへ入れる。Application scopeのeffect targetはquit開始時にopenであるactive / Idle Sessionと進行中Workflowである。関連provider runtime / child processはowner targetから到達するsubordinate effectで、別targetや4096件上限へ重複計上しない。closed / archived Sessionとdurable `OrphanRuntime` recovery obligationは新規targetまたは4096件上限へ含めない。それらのpreexisting pending obligationはpending recovery inventoryの`ClosedSession / ArchivedSession / UnownedRuntime`専用partitionから削除せず、open Session / running Workflow等のpending countやmemory-only orphanをpreexisting recovery summaryへ混入させない。

pending recovery inventoryは同一revisionでatomicに切り替える3-tree root envelopeである。各COW B+tree nodeはdomain-separated child hash、subtree record / byte count、partition summaryを持つcomposable Merkle nodeで、insert / move / delete時は変更pathだけをO(path)で再hashする。root / partition / range hashを全ordered leafのO(N)再列挙で作ってはならず、snapshot rangeは境界pathと完全被覆subtree hashから合成する。canonical primary / count treeだけが各pending obligationをちょうど1件保持し、`OpenSession / WorkflowExecution / ApplicationShutdown / ClosedSession / ArchivedSession / UnownedRuntime`のordered exactly 6 partitionを持つ。primary partitionは次の優先順位で一意に決める。(1) `shutdown_association=Some`は実ownerやSession lifecycleが後で変わってもterminalまで`ApplicationShutdown`へ固定する、(2) `OrphanRuntime` ownerは`UnownedRuntime`、(3) associationなしSessionはpersisted lifecycleに応じ`OpenSession / ClosedSession / ArchivedSession`としclose / archive transitionと同commitでmoveする、(4) associationなし`WorkflowExecution` ownerは`WorkflowExecution`とする。分類不能時は推測moveせずsafe failureとしてmutation admissionを閉じる。primary keyは`partition tag + length-prefixed classification owner identity + obligation ID`である。actual-owner secondary keyは`ObligationOwner + obligation ID`、shutdown-association secondary keyはimmutableな`plan ID + epoch + target key + obligation ID`であり、両valueはprimary key、obligation revision、payload hashだけを持つ。全record count、6 partition count / hash、shutdown snapshot count / hashはprimaryだけから計算し、secondaryを二重計上しない。All / Partition queryはprimary、Owner queryはactual-owner secondary、ShutdownPlan queryはshutdown-association secondaryから各candidateをprimaryへ最大1回direct lookupする。obligation作成はprimaryと必要なsecondaryをinsertし、lifecycle / primary partition / owner transitionはprimaryとsecondary valueを同commitで更新する。association secondaryのkeyは作成後に変更せず、terminal / delete時だけprimaryとsecondaryを同時にremoveする。direct obligation、3 tree ref、一部indexだけのcommitを許さず、root / secondary hash不一致はmutation admissionを閉じて旧rootへ推測fallbackしない。

pending discovery用3-tree envelopeとは別に、Pending / Terminalを含む全obligationのlatest canonical stateをobligation IDで一度だけ引けるimmutable `obligation-state-by-ID` COW rootを持つ。このtreeもcomposable Merkle node / subtree countを使い、transition時のhash計算をO(path)に限定して全state leafのordered再hashを禁止する。obligation作成・pending transition・terminal transitionはdirect obligation record、obligation-state root、pending中だけ存在するcanonical primary / secondary / 3-tree envelopeを同じPhase 0 manifestへ入れる。terminal transitionはstate rootをTerminalへ進めるのと同時にpending primary / secondaryから削除する。manifest participant上限64はlogical mutable participantへ適用し、hash検証済みimmutable COW dependency pagesをparticipantとして数えない。transaction-inventory rootがcommitしてmanifestへ到達しrequired syncが成功する前に、pending rootまたはobligation-state rootのlogical current pointerだけを前進させない。

read側のcommon queryはopaque `Phase0ReadSnapshotRef`でtransaction-inventory、pending 3-tree envelope、obligation-state-by-ID root、latest-activated pointerのrevision / hashだけを一つのleaseへ固定する。shutdown projection / action / compactorはこのbaseにlatest-attempt pointerとunresolved shutdown scope-fence rootのrevision / hashを同じcommit間隙から追加した`Phase0ShutdownReadSnapshotRefV1`を使い、baseだけからcurrent candidateやavailable actionを合成しない。direct / manifest overlayとstate root、pending secondary、scope fence、2本のpointerから別revisionを混ぜない。current logical rootsと未失効read / backup lease、latest / retiring shutdown snapshot、F3 import checkpointから到達するimmutable pagesをGC mark rootとし、全参照消滅後だけ回収する。F3 cutoverは同じbarrierのobligation-state rootからPending / Terminal canonical state、pending 3-treeからdiscovery / owner / association index、shutdown wrapperのscope-fence rootからcross-plan duplicate gateを同canonical keyでimport・照合し、件数 / revision / payload hash不一致ではauthorityを切り替えない。

quitはadmission済みattemptのsettle / materialization後のsnapshot barrierで、canonical primaryの`ClosedSession / ArchivedSession / UnownedRuntime` ordered 3 rangeについてinventory revision、root envelope hash、first / last key、range hash / countをrecord列挙なしのrootまたはtree-height読込で固定し、`preexisting_recovery_count`とtyped `PendingRecoveryInventorySnapshotRef`だけをshutdown rootへ保存する。`PendingRecoveryInventorySnapshotRef.root_page_sha256`は3 tree refとprimaryのexactly 6 partition summaryを含むroot envelope全体のhashであり、各range hash / countはprimaryだけを表す。shutdown開始時に全pending recordを列挙・複製したり、200件pageを新規作成・hashしたりしない。root配下の実recordはstartup recoveryや明示drill-down時に既存のpending-only cursorで最大200 records / decoded 4 MiBずつ読む。quit中はこのsnapshot refから新effectを開始せず、exit / restart表示と後続recovery監督だけに使う。process exitで結果が変わり得るeffectだけが`ExternalEffectIntent.process_exit_coupling=MayChangeOutcome`を持ち、それ以外は`None`とする。

`preexisting_recovery_count == 0`ならsnapshotはNone、1件以上ならdurable `PendingRecoveryInventorySnapshotRef`を必須とし、snapshotの`record_count`とordered 3 rangeのcount合計はroot側countと一致しなければならない。full target pagesとsnapshot refを保持できるのは`LatestShutdownAttemptRefV1`が指す**latest plan**（current Preparing / Preparedを含む）と、`LatestRetiringShutdownPlanRefV1`が指すat most one retiring planだけである。new flightの`shutdown-init/v1` closureはfresh snapshotからprior latest plan state / root / revision、unresolved shutdown scope-fence root、new-flight admission / store / explicit retry guard、retiring pointerをこの順でpreflightする。same-boot current nonterminal flightはこのclosure前にjoinする。process flightが無い状態でsame-bootまたはprevious-bootのprior latestがnonterminal、またはscope fenceが1件以上ならexact `PreviousShutdownReconciliationRequired`でnew plan identity / root / page / effect 0件の`BeforeCommit`拒否にする。same-bootまたはprevious-bootのprior latestがeligible terminalで全new-flight guardを満たす場合だけcompaction guardへ進み、`details=Available`かつretiringがNoneなら旧latestをretiringへ移すのとnew minimal Preparing root / latest-attempt pointer確定を同じcommit pointにし、eligible terminal prior latestがdetails AvailableかつretiringがSomeならexact `PreviousShutdownCompactionPending`でnew plan identity / root / page / effect 0件の`BeforeCommit`拒否にする。eligible terminal prior latestがCompactedなら`ShutdownPlanCompactArchiveV1`、compact root、nested summaryのplan / epoch / exit intent / final state / counts / outcome / safe failure parityをbyte検証し、retiring pointerを変更しない。後続のcomplete Prepared publishは同じnew planだけを進め、prior latestを再探索したりretiring pointerを変更したりしない。previous-boot terminalをnonterminal拒否へ含めない。same-bootまたはprevious-bootのnonterminal planが`get_application_shutdown`のcurrent projectionである間はnew epoch自体を発行せず、同identityのresolutionを要求する。

旧plan compactionをroot-init、Prepared root publish、activation commit pathへ入れない。個別page closureはimmutable page / standalone authority / manifest / transaction inventoryだけをstaging commitし、Preparing root / latest-attemptをread guardのまま不変にする。全page成功時のPrepared closureまたはpartial failure時のterminal closureだけが検証済みpage prefixをrootから到達可能にする。global activation前の`Failed / Cancelled` terminal closureはeffect 0件の`AbortedBeforeActivation(details=Available)` root、original candidate数ではなく到達可能な成功済みpage ref合計と一致するexact target / prepared counts、ordered successfully-prepared page-set hash、bounded safe failure、full page / standalone target authority / snapshot ref、`LatestShutdownAttemptRefV1` CASだけを保存し、archive insert、detail detach、GCを同closureへ入れない。durable Preparing中のprepare failureは同root / latest-attempt pointerをFailed / Cancelledへ閉じ、initial Preparing rootのcommit前だけをprocess-only rejectionとする。到達不能stagingはexclusive writer lockと`BeforeCommit`確定後だけbounded GCする。

guarded background compactorはretiring pointer、source root、pending / obligation-state snapshotを固定してsummary / exact counts / ordered page-set hashをoff-writer foldし、三つのdurable stepを順番に進める。`ArchiveSwitch` closureはcompaction gateを閉じ、plan query / backup / import lease 0、retiring source root / revision、fold revision、latest pointerを再検査し、canonical `ShutdownPlanCompactArchiveV1`、public snapshot refを外す一方でoriginal page suffix・authority ordinal / prefix・pending materializationをinternal cleanup authorityとして持つcompact root revision +1、そのrootを指すretiring pointer CAS、必要ならsame-plan latest-attempt root CASを同じcommit pointへ置く。page、standalone target authority、resource read guardは削除せず、成功直後からold-plan queryはarchive-onlyの`details=Compacted`、entries空、next cursor Noneを返し、residual detailへAvailable fallbackしない。archive queryの`ShutdownPlanPage.plan_revision / root_sha256`は常にsource root revision / hashで、compact shellの存否により変えない。projectionはsummaryが持つplan / epoch / exit intent / target・preexisting count / failureと、`ShutdownPlanCompactProjectionV1`が持つterminal phase・prepared / effect-reserved / terminal count・cutoff / deadlineだけから構成し、snapshot None、actions空にする。summaryからcompact projection fieldを推測しない。

`DetailDetachChunk` closureはstable target-key / authority-key順に最大64 recordかつdecoded 4 MiB / 50 msまでを扱う。Phase 0 file-storeはordered batch overlayとcompact-rootのnext ordinal / detached-prefix / pending transactionを先にdurable commitし、commit後materializerのdirect-file absenceを完了証拠にしてpartial deleteを同じbatchから再開する。future F3 SQLは同stepを`InventoryDetachChunk`と呼び、kind 5 read guardとkind 9 target authorityを`item Live→Cleared → owner delete → link delete`、distinct semantic root各1 CASへ写し、row / item absenceをcursorにする。`FinalizeDetach`はdetail authority / guard / live linkが0、page query lease 0を確認し最大4 pageかつ4 MiB / 50 msまでを扱う。Phase 0はpage batch overlayをcommit後materializeし、全page absenceを確認した0-page dedicated closureだけがstage Complete / retiring Noneへ進める。future F3 SQLはlast atomic page batchまたはpage 0 dedicated transactionでpointerをclearできる。各logical commit / materialization / final clear前後のcrashはarchive、compact-root cursor / residual suffix / pending batch、pointerから同じstageを再開し、queryをAvailableへ戻さない。retiring planがfinalizeするまでは次のAvailable priorを持つquit prepareを`PreviousShutdownCompactionPending`＋external effect 0件で拒否し、latest plan＋retiring planの最大2 detail setを超えない。関連pending obligationは実owner、immutable shutdown association、effect payloadを独立保持するため旧pageを回復authorityにせず、exact private payloadやprovider observation本文をshutdown rootへ複製しない。

`ShutdownPlanRootV1.safe_failure`はpath、raw storage / provider error、secretを除いたbounded `SafeOperationFailure`だけを保持し、`Failed / ReconciliationRequired`ではSome、それ以外ではNoneを必須とする。rootのstate transitionと同じclosure commitで確定し、initial Preparing rootのcommit前に返すpre-acceptance process-only failureだけはdurable plan projectionを作らない。compaction時はfoldで得た`ShutdownSummary.safe_failure`へ移し、restart / compact archive / F3 importでfailure reasonを失わない。

`activation_ancestor_sha256`はlatest-activated pointerとpost-activation descendant rootを結ぶimmutable proofである。Preparedおよびpre-activation Failed / Cancelled / previous-boot Prepared由来ReconciliationRequiredはNone、initial Activated rootも自己hashを避けるためNoneとする。最初のpost-activation transition（Quiescing、Completed、post-activation ReconciliationRequired等）がdescendant rootの`activation_ancestor_sha256=Some(LatestActivatedShutdownPlanRefV1.activated_root_sha256)`を設定し、以後の全descendantと`ShutdownPlanCompactArchiveV1.activation_ancestor_sha256`までbyte不変で継承する。ArchiveSwitchはsource rootの値とarchive fieldのOption tag / bytesをexact照合し、欠落、Noneへのdowngrade、別activated rootへの差替えを拒否する。pointer-pair case (a) は、current root stateがActivatedならlatest-attempt root hashとlatest-activated activated-root hashの一致、post-activation descendantなら`activation_ancestor_sha256=Some(pointer activated-root hash)`の一致で同一activation lineageを証明する。plan / epoch一致だけ、またはcurrent descendant root hashとinitial Activated hashの直接比較で判定しない。

`ShutdownPreparedPageV1`はphysical page refとtarget authorityを検証後に結合したRust query用domain modelであり、そのまま保存しない。physical page bodyはplan ID / epoch、page index / first global ordinalと、各targetのderived `target_key` / global `target_ordinal`、deterministic obligation ID、expected owner revision、target-authority digestだけをimmutableに固定する。stable target identity、Active / Idle activity、exact scope / `PreparedShutdownEffect`は1〜65,536 bytesのtarget authorityだけが持ち、pageやcurrent ownerから補完しない。pageは最大128 target、planは最大32 page / 4096 targetである。complete rootが`Prepared`またはcurrent `Activated / Quiescing`で、rootから到達するpage ref、digest / identity一致のtarget authorityがあり、同obligation IDのdeterministic direct / overlay recordがAbsentならpublic stateはlogical `Prepared`である。`Prepared`表示はeffect reservation eligibilityを意味せず、reservationにはcurrent `Activated` rootとauthorityの再検証が必須である。root未完成の部分pageはentriesとして公開しない。pre-activation `Failed / Cancelled`はdetails Availableの間、保持したpage / authorityから`CancelledBeforeActivation` entryを返す。ArchiveSwitch後だけcompact archiveのcounts / page-set hashからaggregateを返し、detached page entryを再公開しない。pointerが次epochへ進んだ旧Activated planのunreserved entryは`Superseded`として導出し、Preparedへ戻さない。reservation closureはcurrent Activated root、page plan / epoch / hash / target key / global ordinal、authority digest / identity、target expected owner revisionをread guardとして検証し、direct obligationのAbsentをexpected、authorityの実ownerとimmutableな`ApplicationShutdownAssociation`を持つ`Pending(EffectReserved)` obligation、更新済みclaim generationを持つ独立claim record、pending inventory root / page更新を同時commitする。authority欠損または不一致はeffect 0件で拒否し、以後は同じobligationを`ReconciliationRequired`または4値Terminal resultへCASする。late reservation resultは明示shutdown commandを起動せずreadbackへ送る。
cross-plan duplicate gateのauthorityはdirect `UnresolvedShutdownScopeFenceV1`とそのCOW index rootである。canonical keyはdomain-separated length-prefixed `(ObligationOwner, exact OwnedShutdownScopeRef)`で、scope内のruntime / workflow generationを含む。shutdown reservation closureはfence keyのAbsentをguardし、`Pending(EffectReserved)` obligationと同じcommitでfenceをinsertする。Pending `EffectReserved / ReconciliationRequired / Failed`の間だけ存在し、compact Terminal transitionと同じcommitでdeleteする。new shutdown prepareはfence rootをpinして全candidateをpoint lookupし、Presentならpage / root / effect 0件の`PreviousShutdownReconciliationRequired`とRust-owned resolution actionを返す。complete Prepared root commitはlookup時のfence-root revision / hashをread guardにし、lookup後に追加されたfenceを跨いで確定しない。Phase 0は旧obligationをnew planへhandoff / reuse / association付け替えせず、旧identityをterminalに解決してから再試行する。provider / workflow authorityで新しいruntime / executor generationを証明した場合だけ別keyのdistinct effectとして含められ、同generationでの回避を許さない。Phase 0はdirect record＋scope-fence COW index、F3は同canonical keyのUNIQUE authorityへone-shot parity importし、不一致ならcutoverしない。

最初のAccepted quitはplan / epoch / immutable `ShutdownExitIntent`、state Preparing、空page refs / count / snapshotを持つrevision 1のminimal rootと`LatestShutdownAttemptRefV1`を`shutdown-init/v1` closureで同時に確定する。以後、全effect targetのprepared page、ordered page hash / countとpreexisting recovery count / `PendingRecoveryInventorySnapshotRef`を持つPrepared root、global `Prepared -> Activated` CASをauthorityとし、全root state transitionでglobal monotonic pointerを同じclosure commitからCASする。newer epochがpointerを取得した後にold epochのlate transitionでregressできない。init commit前だけはprocess-onlyでAccepted / Preparing projectionを作らない。activation closureはcurrent latest-attempt pointerのplan / epoch / Prepared root hash / revisionをread guardにし、root CAS、latest-attempt pointerのActivated更新、`LatestActivatedShutdownPlanRefV1 { plan_id, epoch, activated_root_sha256, coordinator_boot_id, global_deadline_wall_ms, pointer_revision }`更新を同じcommitで確定する。latest-attempt pointerはcurrent / restart attempt discovery、latest-activated pointerは不可逆activationとExitCoupled evidenceだけのauthorityであり、相互に代用しない。global activation前のpage / rootはinertである。全target prepareまたはactivationの`BeforeCommit`を確定できたfailure時だけprocess exitなし・明示effect 0件でabortしadmissionを再開する。activation writer開始後の`OutcomeUnknown`はabsenceからabortへ格下げせず、admissionを再開しない。current processは明示shutdown commandを開始せず同attemptを`ReconciliationRequired`として15秒以内にexitし、fresh bootが同じactivation identityをresolveする。13秒durability cutoffは`T0`基準のabsolute slotで閉じる。`[0, 0.25s]`はinitial `ShutdownRootInitialized` 1 closure、`[0.25s, 2.0s]`は残りadmission settle 1.75秒、`[2.0s, 3.25s]`はtarget / recovery snapshot enumeration / encode 1.25秒、`[3.25s, 11.75s]`は最大32 page＋Prepared root＋activationの34 closuresを各250 msで8.5秒、`[11.75s, 12.5s]`はOutcomeUnknown lookup / scheduling 0.75秒、`[12.5s, 12.75s]`はactivation未成立時だけdurable Failed / Cancelled finality 1 closure、`[12.75s, 13.0s]`は0.25秒marginである。途中failureは残りpage slotを捨ててfinality slotへ進み、initial initが`BeforeCommit`なら未受理のままfinalityを作らない。残時間から全prepare / activationを開始前に確定不能と判断できるplanはeffect 0件でabortする。Activated後も明示shutdown commandを自動起動せず、activation commitを確認したcurrent coordinatorだけがplan ID / epoch / target revisionを再検査して各targetを`EffectReserved`へCASした後にcommandを開始する。各Session executorのabsolute deadlineは開始時に`min(executor開始 + 10s, durability cutoff)`へ固定し、遅いactivationを理由にcutoffより後へ延長しない。activation後はabort / admission reopenを禁止し、cutoffでterminal / completion / EffectReserved未到達のPrepared targetもstable identity / expected revision / obligation IDを回復根拠として残し、global deadline 15秒以内にrootと同じintentを持つone-shot `ExitPermitV1`でexitする。`EffectReserved`は明示commandの根拠に限定される。Activated後またはactivation outcome未解決のprocess exitによるpipe close / Windows job object / parent-death signalは未予約targetやpreexisting childへ暗黙作用し得るためeffect 0と推測しない。

`LatestActivatedShutdownPlanRefV1.coordinator_boot_id != current_boot_id`、またはsame bootのcoordinatorが`Finalizing / Stopped`であることをcommitted activation後のexit evidenceとする。final summary commitがfailure / OutcomeUnknownでも、restart queryはdeterministic summary transactionのcommitted overlay / direct resultを先に解決し、確定summaryが無ければlatest-activated pointer、Activated root、immutable pages、boot / coordinator evidenceから`ExitedWithRecovery`とunresolved targetを導出する。activation writer開始後のOutcomeUnknownはcutoff時点でsame attemptをresolveし、`BeforeCommit`に加えてrollback ackとlatest-attempt fence成功を確認できた場合だけabortする。Committedならrecovery exitへ進み、`T0 + 15s`でもStillUnknownならactivation-possibleとして保守的にexitする。fresh bootはlatest-attempt pointerとtransaction identityをresolveし、previous-boot PreparedのままでもReconciliationRequiredへ導出する。保存できなかった完了を`Completed`へ推測しない。shutdown target queryは未予約logical Preparedまたは結果未確定EffectReservedを`ExitCoupledOutcomeUnknown { plan_id, epoch }`付き`ReconciliationRequired`へ合成し、terminal direct resultなどpointerよりnewerなobligation stateを優先する。startupはこのviewから明示shutdown commandを自動開始しない。

preexisting pending recovery queryが使うexit-evidence candidateはpointer pairから最大1件だけ選ぶ。(a) latest-attemptとlatest-activatedのplan / epochが一致し、current rootがinitial Activatedならlatest-attempt root hash、post-activation descendantならimmutable `activation_ancestor_sha256`がactivated pointer hashと一致し、かつactivated pointerのboot changeまたはsame-boot Finalizing / Stoppedがあるplan、または(b) latest-activated evidenceがそのattemptのactivation lineageを証明せず、latest-attemptがprevious-boot Prepared rootを指すactivation-possible planである。現在pageの各obligation keyについてcandidateのpinned snapshot B+treeをdirect membership lookupし、snapshot leafのobligation revision / payload hashがcurrent direct obligationと一致し、effectの`process_exit_coupling`が`MayChangeOutcome`である場合だけ同plan / epochの`ExitCoupledOutcomeUnknown`をview overlayする。current obligationのより新しいrevision / terminal / safe observationがあればそれを優先する。1 page最大200 records / decoded 4 MiBに対してこのcandidate 1件だけを参照し、複数planやcompacted archiveを走査しない。new flightのroot-initがlatest pointerをnew Preparing planへ進めた後もexit-evidence candidateの選択規則に従い、retiring / old snapshotの観測を累積しない。旧unreserved targetは`Superseded`へ導出する。これによりpreexisting recordへexit時に個別writeせず、bounded queryとplan compactionを両立する。

```rust
pub enum Phase0ClosureError {
    BeforeCommit(LocalPersistenceFailure),
    OutcomeUnknown {
        transaction_id: String,
        payload_hash: [u8; 32],
    },
    CommittedPendingMaterialization {
        transaction_id: String,
        obligation_id: String,
    },
    MaterializationConflict,
}

pub enum Phase0CommitOutcome {
    Committed {
        transaction_id: String,
        payload_hash: [u8; 32],
    },
    BeforeCommit,
    Conflict,
    StillUnknown {
        transaction_id: String,
        payload_hash: [u8; 32],
    },
}
```

`BeforeCommit`だけがexplicit provider / shutdown command 0件を保証する。Prepare / Commit / Cancelを含め、writer開始後のtimeout / receiver drop / transaction-inventory root CAS・root syncの結果不明は`OutcomeUnknown`とし、operation / transaction identityとpayload hashの`resolve_outcome`で解決するまで次のmutationやeffectへ進まない。resolver / restartはreachable rootを見つけても即Committedにせず、manifest / COW page / root hashを検証し、root file / parent / required ancestor syncを再実行して成功した場合だけ`Committed`を返す。検証または再syncを完了できなければ`StillUnknown`のままstoreを`Stalled`にし、再commit / provider I/Oを0件にする。`BeforeCommit`へ確定できるのは、元writerがroot CAS前終了とrollbackをackした場合、またはrestart後にexclusive writer lockで旧writer不在を証明したfresh lookupでもauthorityが無い場合だけである。root CAS前にno-replace publishしただけのmanifestや、visible / reachableだけのrootをcommitted扱いしない。durability確認後のlegacy materialization failureは未受理へ戻さず`CommittedPendingMaterialization`として同じobligationを`ReconciliationRequired`へ投影する。expected / targetのどちらにも一致しないmaterializationは`MaterializationConflict`としてscopeをquarantineし、推測fallbackしない。

StopAcceptance manifestはStop Accepted前に`TerminalCommit(EffectReserved)`とclaimを同時確定し、後続reserve commitを置かない。通常send / permission / provider establish・resume / recovery / Session close / Workflow shutdownも各external effect前に対応obligationを作る。RecoveryPublicationはBackendRecovery completionでPendingになり、local claim後もEffectReservedを通らずmessage＋marker＋Terminalを同closureで確定する。claimはpending lifecycleと別recordでowner boot id、claim generation、token、leaseを持ち、claim中crash後にreclaimできる。bridge schema、prepared page 128 targets / 1 MiB、root 32 pages / 4096 targets、F3へのone-shot import / authority cutoverは[d3-durable-event-store-design.md](d3-durable-event-store-design.md)を正本とし、cutover後にlegacy / bridgeへdual-writeしない。

### 10. Durable event の進化（bounded context 別）

shutdown projectionのclosed phaseは`Preparing | Prepared | Activated | Quiescing | Completed | Failed | Cancelled | ReconciliationRequired`である。new flightにeligibleなprior latestは`Completed`、またはdurable `Failed | Cancelled` terminal fenceと、同じplan / epoch、target / completed / unresolved / preexisting counts、safe failure、exit intentを持つeffect 0件の`AbortedBeforeActivation` terminal rootが揃う場合だけである。terminal closure直後の`details=Available`はfull refs / snapshotを保持し、new root-initでretiringへreserveする。既に`details=Compacted`ならarchive parityを検証する。全caseでadmission Open、store Healthy、unresolved shutdown scope fence 0を同snapshotで必須とし、process-only Failed、Stalled / unhealthy、admission Closed、scope fence残存をexplicit retryで迂回しない。`RetryQuit`もsame-boot effect 0 Failedとこの全predicateを満たす場合だけ提示する。

shutdown snapshot capacityは`Phase0ShutdownTargetAuthorityV1` 1〜65,536 bytes、`Phase0ShutdownPreparedPageV1` 1 page 128 target refs以下かつpage canonical bytesと対応するstandalone authority canonical bytesの合計1 MiB以下、`Phase0ShutdownPlanRootV1` 32 pages / 4096 targets以下である。65,537 bytes、aggregate 1 MiB＋1、129 refs、33 root pages、4097 targetsはBeforeCommit capacity failureとしてnew plan identity / root / page / target / terminal / provider・workflow・OS effectを0件にし、各上限ちょうどまでは受理する。Phase 0 shutdown page closureはresource inventory / data packを作らずK=0でK1 laneを予約し、transaction commit-index packだけを最大64 pagesに閉じる。

**V-D11**: 進化規約（V-P4 の具体化）:

1. domain-owned `AgentSessionDomainEvent / WorkflowDomainEvent` のsemantic variant追加はadditive-onlyとし、gateway変換のexhaustive matchを同時に更新する。domain eventにserde属性やschema versionを持たせない。
2. gatewayのpersistence command model変更はadditive-onlyとし、既存variantへのfield追加は`#[serde(default)]`を必須にする。旧バージョンのReleashが新variantを読む前方互換は不要だが、新バージョンは全旧eventを読めること（後方互換必須）。
3. `schema_version`はevent logの`PersistenceEventEnvelope`に持たせ、gateway読み込み時にlazy upcastする（旧`completed: bool` → `TodoStatus`等はcommand modelへの読み込み写像で吸収し、書き戻しはしない）。
4. 未知event・未知fieldはgatewayがrawのまま保持し、projectorは無視、書き戻しで保全する。未知payloadをdomain eventのcatch-all variantやJSONへ落とさない。

#### AgentSessionDomainEvent / AgentSession persistence schema

`AgentSessionDomainEvent`は次のvariantだけを持つclosed enumである。payload fieldの完全定義は#1446以降で行うが、membershipは本節が完全かつ規範的であり、§8の既存eventと本節の追加eventを含む。gatewayはこの全membershipをexhaustive matchしてpersistence command modelへ変換し、catch-all variantを設けない。

```rust
pub enum AgentSessionDomainEvent {
    UserInputAccepted,
    AssistantMessageOpened,
    TurnStarted,
    MessagePartRecorded,
    FinalPartsRecorded,
    TurnCompleted,
    TokenUsageUpdated,
    ToolCallStarted,
    ToolCallSucceeded,
    ToolCallFailed,
    ToolResultRecorded,
    ToolCallRetried,
    ToolCallStatusChanged,
    PermissionRequested,
    PermissionResolved,
    PermissionResponseRequested,
    PermissionResponseRejected,
    ProviderPermissionResponseObserved,
    PermissionResponseReconciliationRequired,
    PermissionResponseReconciled,
    TaskStatusChanged,
    TodoListSnapshotRecorded,
    NoticeRecorded,
    ImageRecorded,
    ImageRefRecorded,
    BackendSessionCleared,
    SessionClosed,
    ConfigurationUpdateRequested,
    ConfigurationUpdateRejected,
    SessionConfigurationSelected,
    SessionConfigurationActivated,
    BackendSessionRecoveryStarted,
    SessionConfigurationReactivated,
    SessionGoalReactivated,
    BackendSessionRecoveryCompleted,
    ProviderConfigurationStateObserved,
    ConfigurationObservationAccepted,
    ConfigurationReconciliationRequired,
    ConfigurationReconciled,
    TurnStartRequested,
    TurnStartReconciliationRequired,
    TurnStartReconciled,
    TurnSteerRequested,
    TurnSteerAccepted,
    TurnSteerRejected,
    ProviderTurnSteerStateObserved,
    TurnSteerReconciliationRequired,
    TurnSteerReconciled,
    TurnInterruptRequested,
    QueuePaused,
    QueueResumed,
    QueueItemEnqueued,
    QueueItemCancelled,
    QueueExecutionPrepared,
    QueueExecutionRequested,
    QueueItemStarted,
    QueueItemFailed,
    QueueItemResolutionRequired,
    QueueItemRebased,
    QueueItemRequeued,
    ReconciliationResolutionRequested,
    AgentLaunchDraftPrepared,
    AgentLaunchPreparationExpired,
    AgentLaunchPreparationCancelled,
    AgentLaunchAttemptStarted,
    AgentLaunchStageAdvanced,
    AgentLaunchReconciliationRequired,
    AgentLaunchProtocolIncompatible,
    AgentLaunchReconciled,
    AgentLaunchCompleted,
    AgentLaunchFailed,
    AgentLaunchCancelled,
    SessionCreated,
    LaunchInitialGoalRejected,
    InitialGoalResolutionRequested,
    InitialGoalResolutionCompleted,
    BypassChallengeIssued,
    BypassChallengeConsumed,
    BypassChallengeExpired,
    BypassChallengeCancelled,
    GoalTransitionRequested,
    GoalTransitionRejected,
    ProviderGoalCommandEvidenceObserved,
    GoalPrecommitControlConflictObserved,
    GoalSet,
    GoalTransitioned,
    GoalCleared,
    ProviderGoalStateObserved,
    GoalObservationAccepted,
    GoalReconciliationRequired,
    GoalReconciled,
    BackendProtocolIdentified,
    ProtocolIncompatible,
}
```

このcomplete membershipに対する追加・payload変更とlegacy persistence schemaからの写像は次のとおり:

| 変更 | 内容 | 解消 |
|---|---|---|
| `UserInputAccepted / AssistantMessageOpened` 追加 | operation id、semantic payload hash、input / human / assistant identity、turnまたはqueue dispositionを同じacceptance batchで確定し、response喪失・restart後も同じreceiptとmessage pairへ収束する | #1499 |
| `ToolCallStatusChanged { turn_id, tool_use_id, status, exit_code?, at }` 追加 | ToolCall 状態遷移の記録 | RG-4/RG-8/SD-5 |
| `NoticeRecorded { turn_id?, message_id, notice }` 追加 | `SystemNotificationRecorded` を後継（旧型は読み込み継続） | CX-7/RG-6/CL-5 |
| `TurnCompleted` 系の outcome 拡張 | stop_reason / stats / 構造化 error。active turnのprotocol driftは`TurnCompleted { result: TurnResult::Interrupted { reason: InterruptReason::ProtocolIncompatible, .. } }`としてdurable化し、last TurnResult / Idle projectionへ再投影する | CL-3/4/RG-3/9/RT-5/#1445 |
| `TurnTokenUsage` → V-D8 型 | cache / cost | RG-9 |
| `PermissionResolved` に `resolved_by` / `effective` 追加 | 実効性の記録 | CL-1 |
| `PermissionResponseRequested / Rejected / ProviderPermissionResponseObserved / PermissionResponseReconciliationRequired / Reconciled` 追加 | 公開eventはresponse id、redacted answers、private payload ref、明示reject後のPending復帰、request cancel/tool start、ack不明と解決attemptをwrite-ahead回復する。exact validated payloadはowner-only private blobへ分離しObserved / terminalまで保持するが、event payload / read model / logへplaintextを保存しない | CL-1/CX-1 |
| `TodoListSnapshotRecorded` の item 拡張 | status / priority | RG-5 |
| `ImageRecorded` / `ImageRefRecorded` の配線 | tool 出力 image | CL-6/RG-7 |
| `ConfigurationUpdateRequested / Rejected` 追加 | `update_id`、base / target revision、discriminated patch、activation timing を write-ahead 記録 | #1397/#1445〜#1448 |
| `SessionConfigurationSelected / Activated` 追加 | selected / effective revision と model を含む小さな設定 snapshot を別々に確定。各 event append が canonical commit point | #1397/#1445〜#1448 |
| `BackendSessionRecoveryStarted / SessionConfigurationReactivated / SessionGoalReactivated / BackendSessionRecoveryCompleted` 追加 | resume metadata clearとbarrier開始、observation相関付きconfiguration/Goal復旧、両aggregateの最終atomic完了を同じrecovery idで確定 | #1397/#1407/#1449 |
| `ProviderConfigurationStateObserved / ConfigurationObservationAccepted / ConfigurationReconciliationRequired / Reconciled` 追加 | observation append時にblockし、同じobservation idをoutcomeがconsume。複合provider stateと解決をdurable化 | #1397/#1445〜#1448 |
| `TurnStartRequested / TurnStartReconciliationRequired / Reconciled` 追加 | effective/activation-targetを分けたintent、correlation、early-stream境界、provider観測とqueue terminalまで含むaction別atomic終端を回復 | #1397/#1450 |
| `TurnSteerRequested / Accepted / Rejected / ProviderTurnSteerStateObserved / ReconciliationRequired / Reconciled` 追加 | immutable inputとsteer idをpre-I/O commitし、明示ack/rejectと結果不明を分離。未適用確定時だけqueueへatomic移送しblind retryを禁止 | #1498 |
| `TurnInterruptRequested / QueuePaused / QueueResumed` 追加 | Stop intentとpauseをpre-I/O atomic commitし、CAS付き明示resumeまで自動drainを禁止 | #1404/#1450 |
| `QueueItemEnqueued / QueueItemCancelled / QueueExecutionPrepared / QueueExecutionRequested / QueueItemStarted / QueueItemFailed / QueueItemResolutionRequired / QueueItemRebased / QueueItemRequeued` 追加 | item revision、message marker、immutable semantic snapshot/hash、challenge guard、resolution/rebase/CAS retry、execution/turn相関をappend-onlyに確定 | #1404/#1450 |
| `ReconciliationResolutionRequested` 追加 | resolution attempt/CAS/action/targetをprovider I/O前に記録し、configuration/Goal/launch/turn-start/permission解決を冪等回復 | #1397/#1445〜#1449 |
| `AgentLaunchDraftPrepared / PreparationExpired / PreparationCancelled / AttemptStarted / StageAdvanced / LaunchReconciliationRequired / LaunchProtocolIncompatible / Reconciled / Completed / Failed / Cancelled` 追加 | reservation、create correlation、provider/local ref、initial Goal handoff、観測、部分protocol identity、recoveryと全terminalをattempt streamへ保存 | #1445 |
| `SessionCreated` 追加（Session stream） | session id、originating launch attempt、provider/session ref、protocol identityを持ち、initial configuration seedとのmulti-stream batchをSession公開のcommit pointにする | #1445 |
| `LaunchInitialGoalRejected / InitialGoalResolutionRequested / Completed` 追加 | launch側rejectを再投影し、RetryGoal/ContinueWithoutGoal/CancelSessionをCAS＋write-aheadで排他して各actionを必ず終端 | #1445/#1449 |
| `BypassChallengeIssued / Consumed / Expired / Cancelled` 追加 | execution/reconciliation固有guard、期限、one-time consume、managed-policy再検査とreload可能なchallenge stateを監査 | #1446/#1448 |
| `GoalTransitionRequested / GoalTransitionRejected` 追加 | `transition_id`、goal id / base revision、操作を Goal 専用 write-ahead protocol で記録。成功終端はcanonical Goal eventだけとする | #1449 |
| `ProviderGoalCommandEvidenceObserved / GoalPrecommitControlConflictObserved` 追加 | provider-neutralな`GoalCommandAcceptanceEvidence` / `ProviderEvidenceRef`を保存する。Claude固有command UUID、completed lifecycle、raw control requestはadaptor/gateway evidenceとして保持し、objective一致Goal stateのacceptanceとcommit前control requestのfail-closed/reconciliationを監査 | #1449/#1416 |
| `GoalSet / GoalTransitioned / GoalCleared` 追加 | goal revision、source、reason、evidence ref、`ProviderGoalSnapshot`をcanonicalに記録。Claude set/editではGoal event＋TurnStartedをatomic batch append | #1449 |
| `ProviderGoalStateObserved / GoalObservationAccepted / GoalReconciliationRequired / Reconciled` 追加 | provider ref＋Matched/Unmatched/Ambiguousを保存し、observation append時block、同じidをoutcomeがconsume | #1449 |
| `BackendProtocolIdentified / ProtocolIncompatible` 追加 | 実行 binary と compiled schema / flags / capabilities の一致を監査し、control-plane drift を fail-closed 化 | #1445/#1447〜#1449 |
| `TurnStarted` に resolved effective configuration / `EffectiveModeSnapshot` / Goal ref / protocol identity 追加 | provider/model/mode/effort、当時のpermission/effects/residual protections/context、Goalを不変監査可能にする | #1450 |
| recovery retryの`TurnStarted`に`retry_of_turn_id / recovery_id`相関を追加 | 旧turnを一度だけterminal化し、同じhuman inputを複製せず新turnへ相関する | #1406 |

#### WorkflowDomainEvent / Workflow persistence schema

`WorkflowDomainEvent`は次のvariantだけを持つclosed enumである。payload fieldの完全定義は#1446以降で行うが、membershipは本節が完全かつ規範的であり、gatewayは全variantをexhaustive matchする。`NodeExecution*` eventはこのenumだけに属し、`AgentSessionDomainEvent`へ逆流させない。domain-owned `WorkflowDomainEvent` からgateway command modelへ変換し、既存`WorkflowEvent` persistence schemaをdomain portへ流用しない。

```rust
pub enum WorkflowDomainEvent {
    WorkflowExecutionStarted,
    NodeExecutionStarted,
    NodeExecutionCommandPrepared,
    WorkflowArtifactProduced,
    NodeExecutionCompleted,
    NodeExecutionFailed,
    WorkflowApprovalRequested,
    WorkflowApprovalResolved,
    WorkflowContractViolated,
    NodeExecutionStallObserved,
    NodeExecutionStallCleared,
    WorkflowExecutionCompleted,
    WorkflowExecutionFailed,
    WorkflowExecutionAborted,
    WorkflowExecutionInterrupted,
    WorkflowExecutionResumed,
    NodeExecutionBypassPrepared,
    NodeExecutionLaunchRequested,
    NodeExecutionAgentBound,
    NodeExecutionAgentLaunchFailed,
    NodeExecutionAgentLaunchCancelled,
}
```

このcomplete membershipに対する追加・payload変更とlegacy persistence schemaからの写像は次のとおり:

| 変更 | 内容 | 解消 |
|---|---|---|
| `NodeExecutionBypassPrepared / NodeExecutionLaunchRequested / NodeExecutionAgentBound / NodeExecutionAgentLaunchFailed / NodeExecutionAgentLaunchCancelled` 追加 | `NodeExecution.id / node_name / attempt`でchallenge待機、stable launch attempt、workflow/launch origin、成功/失敗/取消terminalをlaunch terminalとのmulti-stream batchで相関 | #1450 |

#### Transaction metadata（domain event enum 外）

| 変更 | 内容 | 解消 |
|---|---|---|
| `LocalAtomicBatchCommitted` commit receipt 追加 | multi-stream head CAS、per-stream/global seq、event count、idempotencyの確定結果を返す。`AgentSessionDomainEvent` / `WorkflowDomainEvent`のvariantとしてappendせず、event payloadも内包しない | #1445/#1446/#1450 |

`PermissionResolvedBy::Auto` は provider classifier / reviewer の approved / denied を表し、取得できる decision reason / review item ref を同じ permission 履歴へ保存する。inProgress / timedOut / aborted や manual fallback を resolution として合成しない。単に `AgentMode::Auto` だったという理由だけで自動許可を合成しない。

### 11. Read model / GetSessionResponse（完全復元）

client-facing watchのapplication境界はusecase-ownedな`AgentSessionWatchService` / `AgentLaunchWatchService`とする。Session側は`AgentSessionWatchFrame::Snapshot(GetSessionResponse) | Delta(AgentSessionReadModelDelta)`、launch側は`AgentLaunchWatchFrame::Snapshot(AgentLaunchProjection) | Changed(AgentLaunchChanged)`だけを返す。`AgentSessionReadModelDelta`はquery DTOとしてexhaustiveに型付けし、configuration変更は`SessionConfigurationChanged(AgentSessionConfigurationProjection)`というfull read-model deltaにする。各serviceは内部で`LocalWatchRepository`とquery serviceを協調させ、bootstrap leaseまたはlive `LocalWatchUpdateFence.snapshot`だけからframeを構築する。Tauri / WebSocket handlerはserviceを開始してtyped frameをprotocolへ写すだけで、`LocalWatchCommitNotice`、Repository、QueryService、snapshot leaseを直接扱わない。

read model は「そのsurfaceが現在描画するために必要なbounded state」を保持する（lifecycle I 群・presentation P1 の前提）。`get_session` は runtime 可視状態の完全スナップショットとsession `seq`を返す: messages(parts) / turn_phase / pending・Responding・reconciliation中のpermission / `QueueProjection`（item revision、active＋bounded recent terminal＋paused＋seq）/ `TurnStartState` / latest TokenUsage / last TurnResult / notices / query専用`AgentSessionConfigurationReadModel` / `SessionGoalProjection` / `SessionControlOperationLease` / `ProviderPermissionState` / `AgentProtocolState` / capabilities / pending observation・reconciliation・resolution attempt / available actions・mode effectsである。send operation履歴を`get_session`へfull-retainせず、callerが保持するoperation IDを使うdirect queryをauthorityにする。公開`AgentSendOperationView`は`Accepted { immutable receipt, latest mutable status, ordered最大4 obligation observations, actions } | OutcomeUnknown { operation_id }`の2値だけであり、受理前internal state、exact input snapshot、compact cleanup stateを返さない。

operation identityのdirect queryはTauri `get_agent_send_operation(operation_id)` / WebSocket `GetOperation`で同じviewを返し、acceptance不明時のblind再sendではなくlookupに使う。query serviceはcurrent-installation principalとoperation IDでoperationのcommitted manifest overlay＋direct record各1件、ordered最大4件のobligation overlay＋direct record / resultだけをbounded lookupし、event / Session full scanをしない。既知operationは`AgentSendOperationView::Accepted`または保存結果未解決の`OutcomeUnknown`、未知IDは`NotFound`を返す。`RejectedBeforeCommit`はdurable viewを作らず、query failureをAccepted / NotFoundへ推測しない。古いqueue terminal履歴は`get_queue_history(session_id, cursor, limit) -> QueueHistoryPage`でpage取得する。operation feedbackはtranscript/read modelへ混ぜず、Tauri `get_session_operation_feedback / dismiss_session_operation_feedback / retry_session_operation_feedback_resolution`とWebSocket `GetOperationFeedback / DismissOperationFeedback / RetryOperationFeedbackResolution`を共通のexempt control planeとして公開し、getは未解決failureをidentity-keyedに1 page最大32件返す。Bypass waiting stateはfull challenge viewを埋め、独立query `get_bypass_challenge(challenge_id) -> BypassChallengeProjection`もIssued/Consumed/Expired/Cancelledを返す。nonceは認可済みclientへIssued中だけ返し、terminal projectionではredactする。

Application scopeはSession read modelへ押し込まない。normal shutdown current projectionのcommon queryはTauri `get_application_shutdown()` / WebSocket `GetApplicationShutdown`であり、同じquery service / presenterから`CurrentApplicationShutdownResult`を返す。`Current(Some(...))`はcurrent plan / epochとfirst-ingressから不変の`ShutdownExitIntent`、`Preparing / Prepared / Activated / Quiescing / Completed / Failed / Cancelled / ReconciliationRequired`、quit開始時にopenなactive / Idle Sessionと進行中Workflowだけからなるtarget / prepared / EffectReserved / terminal count、closed / archived Sessionおよびdurable `OrphanRuntime`のpreexisting recovery count / `PendingRecoveryInventorySnapshotRef`、13秒cutoff、15秒deadline、safe failure、専用`ApplicationShutdownAction`を持つ。same-bootのAccepted flightはminimal Preparingを含むdurable plan rootと`LatestShutdownAttemptRefV1`をauthorityにし、同flight identityへobligation stateをoverlayする。init writer開始後のOutcomeUnknownはprocess stateからPreparingを合成せず同transactionをresolveする。pre-activation Failed / Cancelledもcurrent terminal flightとしてnew attempt開始またはprocess終了まで`Current(Some(...))`を維持するが、CancelはRetry actionを意味しない。fresh bootでprocess flightが無い場合はlatest-attempt pointerのprevious-boot nonterminalだけをsame identityの`ReconciliationRequired`へ導出して`Current(Some(...))`とし、previous-boot `Failed / Cancelled / Completed`とnormal shutdown attempt未存在は`Current(None)`を返す。bootstrap-safe quitはnormal planへ合成せず、quit operation queryの`ApplicationQuitProjection::Bootstrap`で読む。terminal historyは`get_shutdown_plan`を使い、summary / compact archiveも同じintentを返す。

current queryのfield ownerはhash-validなcanonical plan rootであり、latest-attempt / latest-activated pointerはroot selectorと冗長cross-checkだけを所有する。同じbounded snapshotでhash-validなcomplete rootがexactly one存在し、そのrootのplan ID / epoch / exit intentを一意にanchorできる場合だけprojectionを構築する。そのうえでpointerの冗長plan ID / epoch / intentがrootとsemanticに矛盾する場合はrootのfieldだけから同identityの`Current(Some(ReconciliationRequired))`を作り、safe `ShutdownAuthorityMismatch`を付ける。storage read failure、decode failure、envelope / root self-hashまたはpointer-to-root hash failure、required record欠損、state composite・activation lineage integrity failure、複数のhash-valid rootまたはunanchorable authorityによりidentityを一意にanchorできないcaseは`Internal { correlation_id }`であり、ReconciliationRequired、OutcomeUnknown、Current(None)を合成しない。root / pointer commit transactionの成否だけが未解決で、同じtransactionとplan identityへanchorできる場合に限ってclosed wrapper `OutcomeUnknown { failure }`を返す。queryはshutdown target、obligation、external effect、admissionを変更しない。Preparing / Prepared / Activatedをeffect開始済みへ推測せず、per-target `EffectReserved`は明示shutdown executor commandを開始したdurable根拠としてだけ扱う。

`ApplicationShutdownProjection.available_actions=[ApplicationShutdownAction::RetryQuit]`を返せるのは、current coordinatorと同じbootのpre-activation `Failed`で、external effect 0件、old attemptを閉じるdurable Failed fenceが確定済み、global admissionがOpenへ戻り、shutdown store healthがHealthy、unresolved shutdown scope fence 0であることを同じread snapshotから証明できる場合だけである。`Preparing / Prepared / Activated / Quiescing / Completed / Cancelled / ReconciliationRequired`、activation OutcomeUnknown / StillUnknown、root / fenceを持たないprocess-only failure、transaction Stalled、admission Closed、scope fence残存、fresh bootでは空vectorとし、明示retryだけでguardを迂回しない。Application-level RetryQuitは通常のtyped quit ingressを再要求する専用操作であり、個別obligation / targetの5値`OperationAction`や`resolve_pending_recovery_action / resolve_shutdown_target_action`を流用しない。

historical / restart recoveryを含むcommon queryはTauri `get_shutdown_plan(plan_id, epoch, cursor, limit)` / WebSocket `GetShutdownPlan`とし、`limit <= 128`、encoded page 1 MiB以下を必須とする。first pageはcritical writerを優先するlow-priority snapshot barrierでtransaction inventory、pending 3-tree envelope、obligation-state-by-ID root、latest-activated pointerをbaseへ、unresolved shutdown scope-fence rootとlatest-attempt pointerをwrapperへ同じcommit間隙から固定した`Phase0ShutdownReadSnapshotRefV1` leaseを発行する。最大32 immutable pages / 4096 deterministic obligation overlay＋direct / state lookupをsequential foldし、`prepared_count`、`effect_reserved_count`、4値terminal resultからの`terminal_count`、target単位の4値terminal resultまたはderived `Superseded / CancelledBeforeActivation`を重複なく終了済みとして含む`completed_count`、`unresolved_count = target_count - completed_count`、non-success terminal、preexisting recovery countを同じauthority revisionからexactに算出する。page response用最大128 entryだけをmaterializeし、4096 viewを保持しない。各physical target refはtarget authorityを最大1回point lookupし、digest / identity一致後だけsafe targetへ結合する。authority欠損または不一致はCorruptとし、pageやcurrent ownerから補完しない。foldは最大32 MiB page decode＋16 MiB lookup、2秒で閉じ、barrier待ちまたは上限超過はpartial count / entryを返さず`QueryBusy / DeadlineExceeded`とする。first page cursorはplan / epoch / root hash / overlay revisions / snapshot lease / last target keyへMAC付きで束縛し、後続pageは同じleaseだけを再利用する。lease失効・revision不一致は`CursorExpired`で先頭から再取得させ、別revisionを継ぎ足さない。`ShutdownPlanPage.projection`は8 phaseを常に返し、`summary`はterminalまたはderived exit outcomeがある場合だけSome、進行中はNoneである。detailsがAvailableの場合だけstable target key順の`ShutdownTargetView`を返し、pre-activation Failed / CancelledもArchiveSwitch前は保持したdetailから`CancelledBeforeActivation` entryを返す。ArchiveSwitch成功後のcompacted planだけがarchive summary / counts / ordered page hashと`details=Compacted`を返し、entriesは空、next cursorはNoneである。residual detachが残っていてもAvailableへfallbackせず、最大4096 targetをprojection / summaryへinline化しない。persisted summaryをroot / pointerより強いauthorityにせず、不一致はReconciliationRequiredである。

plan-level全域写像は次で固定する。Completed transactionがOutcomeUnknownなら同transactionを先にresolveし、StillUnknown中はstoreをStalledとしてOutcomeUnknownを返し、Completedを推測しない。

| Durable / process evidence | Public phase | Summary |
|---|---|---|
| current durable Preparing | Preparing | None |
| current bootのactive flightに属するPrepared | Prepared | None |
| activation前のFailed / Cancelled、ArchiveSwitch前 | CancelledまたはFailed | Some(AbortedBeforeActivation、details=Available) |
| previous bootのPrepared、またはactivation writerがStillUnknownのままcurrent bootがFinalizing / Stopped | ReconciliationRequired | Some(ExitedWithRecovery) |
| current bootでcutoff前のActivated | Activated | None |
| current bootで実行中のQuiescing | Quiescing | None |
| Activated / Quiescingでcoordinator bootがcurrentと異なる、またはsame boot Finalizing / StoppedだがCompleted未確認 | ReconciliationRequired | Some(ExitedWithRecovery) |
| Completed、unresolved 0、preexisting recovery 0 | Completed | Some(Completed) |
| Completedだがpreexisting recoveryまたはnon-success terminal targetあり | Completed | Some(ExitedWithRecovery) |
| ReconciliationRequired | ReconciliationRequired | Some(ReconciliationRequired) |
| compact archive overlay | archive summaryのterminal phase | Some(archive summary、details=Compacted) |

target-level全域写像はimmutable page entryと同じobligation IDのcommitted direct / reachable overlayから合成する。

| Plan / target authority | Public target state | terminal result / safe failure | Observation |
|---|---|---|---|
| current active Prepared / Activated、obligation / result Absent、exit evidenceなし | Prepared | None / None | None |
| activation前Failed / Cancelled、details Available | CancelledBeforeActivation | None / None | None |
| activation前Failed / Cancelled、ArchiveSwitch後のcompact archive | entry非公開。aggregateはCancelledBeforeActivation | None / None | None |
| latest-attempt pointerより古いplan、obligation / result Absent | Superseded | None / None | None |
| activation-lineage proofがpointerと一致するlatest post-activation plan、またはlatest-attempt previous-boot Preparedにactivation-possible exit evidenceがあり、obligation / result Absent | ReconciliationRequired | None / 保存済みreasonがあればSome | ExitCoupledOutcomeUnknown(plan, epoch) |
| Pending(EffectReserved)、current coordinator有効 | EffectReserved | None / None | 保存済みsafe observationまたはNone |
| Pending(EffectReserved)だがexit evidenceあり | ReconciliationRequired | None / 保存済みreasonがあればSome | 保存済みnewer observation、無ければExitCoupledOutcomeUnknown |
| Pending(ReconciliationRequired)またはPending(Failed) | ReconciliationRequired | None / canonical pending reason | 保存済みsafe observation |
| Terminal(Succeeded / CancelledBeforeEffect / Superseded) | Completed | Some(exact result) / None | 保存済みsafe observation |
| Terminal(FailedTerminal) | Completed | Some(FailedTerminal) / Some | 保存済みsafe observation |
| Completed planなのにobligation / result Absent、またはhash / association不一致 | ReconciliationRequired | None / Some integrity failure | safe failure。scopeをquarantine |

`ShutdownTargetView`はprepared authorityから復元した`ShutdownTargetSubjectView`、public state、safe observation / failure、terminal result、available actionsを一つのsnapshotで返す。`state=Completed`は`terminal_result=Some(Succeeded | CancelledBeforeEffect | Superseded | FailedTerminal)`を必須とし、FailedTerminalだけ`safe_failure=Some`を許す。Prepared / EffectReserved / CancelledBeforeActivation / Supersededは`terminal_result=None`かつ`safe_failure=None`、ReconciliationRequiredは`terminal_result=None`で保存済みbounded reasonがある場合だけ`safe_failure=Some`とする。derived public `Superseded`とterminal result `ObligationResult::Superseded`を同一fieldへ潰さない。
pre-activation abortはprocess exitを行わないためshutdown commandは0件である。後の無関係なcrashによるchild状態をplan targetのConfirmedNoEffectへ読み替えず、通常startup recoveryへ委ねる。post-activation final summaryを書けなくてもActivated root、latest-activated pointer、coordinator boot evidenceからExitedWithRecoveryを派生し、未保存のsuccessをCompletedへ捏造しない。activation outcome未解決のprevious-boot Preparedはlatest-attempt pointerから同じ保守的なReconciliationRequiredへ導出するが、latest-activated pointerを捏造しない。

pending recoveryのcommon queryはTauri `list_pending_agent_recovery(ListPendingRecoveryRequest)` / WebSocket `ListPendingRecovery`とし、同じquery serviceを使う。filterはAll、exact `ObligationOwner`、publicな`ClosedSession / ArchivedSession / UnownedRuntime` partition、`ShutdownPlan { plan_id, epoch }`のいずれかで、indexed tree / range以外へfallbackしない。All / Partitionはcanonical primary、Ownerはactual-owner secondary、ShutdownPlanはshutdown-association secondaryを使い、secondary candidateごとにprimaryへ最大1回direct lookupする。first pageは3-tree root envelopeのrevisionと各COW rootへ一つのread leaseをpinし、opaque cursorをboot ID / lease ID / filter / request binding / inventory revision / selected tree kind・root hash / last raw keyへMAC付きで束縛する。`1 <= limit <= 200`、encoded response 4 MiB以下に加えて1回のcandidate scanも最大200 records / 4 MiBで閉じる。有効なleaseがpinした旧revisionはcurrent rootが進んでも最後まで読める。別filter / requestへの再利用、binding不一致、MAC不正は`CursorMismatch`、cursorとleaseのrevision / selected tree root不一致、boot / lease失効、pinned root retention外は`CursorExpired`を返し、別revisionの続きへ自動fallbackせず先頭からの再取得を要求する。

public pageは`Pending(Prepared)`を含む全9 kindのpending obligationを列挙し、obligation identity、owner、immutable shutdown association、kind、payloadless public lifecycle / observation / separate safe failure / actions、revisionを返す。Preparedのexact effect blueprintやprivate payload ref、exact permission response、filesystem path、provider raw observationは返さない。`ShutdownPlan` filterはcurrent inventory内でimmutable `shutdown_association`が同じplan / epochであるtarget obligationだけを返し、associationを持たないpreexisting Closed / Archived / UnownedRuntimeを混ぜない。All / owner / partitionで返すcurrent keyが、pointer pairから選んだ最大1件のexit-evidence candidate（same plan / epoch＋root hashまたはactivation ancestor proof、またはlatest-attempt previous-boot Preparedのactivation-possible plan）のpinned snapshotにも存在する場合は、snapshot leaf revision / payload hashがcurrent obligationと一致する`MayChangeOutcome` effectだけにExitCoupled observationをoverlayする。newer current lifecycle / observation / terminalを優先し、複数plan、retiring plan、compacted old snapshotをscanしない。

shutdown pinned snapshotのdrill-downは別のTauri `get_pending_recovery_snapshot(GetPendingRecoverySnapshotRequest)` / WebSocket `GetPendingRecoverySnapshot`で行う。requestのplan ID / epoch / exact `PendingRecoveryInventorySnapshotRef` / partitionをplan rootへ照合し、opaque cursorをboot ID / plan / epoch / snapshot hash / partition / pinned inventory revision / last keyへMAC付きで束縛する。snapshot refは`ClosedSession / ArchivedSession / UnownedRuntime`のexactly 3 rangeをcount 0も含めて常に持つため、このclosed enumの3 partitionは全てvalidであり、empty rangeは空pageを返す。unknown wire tagやlimit範囲外は`InvalidRequest`、plan / epochまたはrequest snapshot refとroot内refのbyte不一致だけを`SnapshotMismatch`、cursorを別plan / snapshot / partitionへ再利用した場合、binding不一致、MAC不正を`CursorMismatch`とする。1 pageは最大200 candidate / entriesかつdecoded 4 MiBの先到達側で閉じ、`Pending(Prepared)`もsafe metadataへredactして返す。各snapshot keyへcurrent obligation / terminal direct resultをbounded lookupし、newer stateがあれば優先する。pointer pairからroot hashまたはactivation ancestor proofで選ぶpost-activation candidate、またはprevious-boot Prepared activation-possible candidateとrequest planが一致し、leaf revision / payload hashがcurrent obligationと一致する`MayChangeOutcome` effectだけにExitCoupled observationをoverlayする。lease / boot / pinned revision / retention失効は`CursorExpired`、old plan compaction後はsummaryを保持した`DetailsCompacted`を返し、current inventoryや別planへfallbackしない。

pending / shutdown viewの`OperationAction`は表示用hintではなく、Rust-owned recovery usecaseへ渡すopaqueなdurable action identityである。clientはprojectionで受け取った`action_id`だけをそのままechoし、format、digest、MAC、provider result、retry key、observation payloadを生成・解析しない。action IDの物理encodingや署名方式はpublic/domain contractではなく、Phase 0とF3が同じcanonical decision identityへ写像できるinternal persistence concernである。

Tauri `resolve_pending_recovery_action` / WebSocket `ResolvePendingRecoveryAction`とTauri `resolve_shutdown_target_action` / WebSocket `ResolveShutdownTargetAction`は同じRust usecaseを使う。usecaseはcurrent resourceより先にaction IDの`RecoveryActionDecision`をdirect lookupする。Completedは保存済みreceipt、classification、canonical safe resultをexact replayし、nonterminal Attemptは同じattemptへjoinし、OutcomeUnknownは同じtransaction identityをresolveする。decision Absentのfresh actionだけがrequestのexpected revision / root / target-state hashとcurrent action availabilityを検証し、不一致は`RevisionConflict`または`ActionUnavailable`としてattempt / claim / external effectを0件にする。未知IDは`NotFound`、malformed IDは`InvalidRequest`であり、`KeepForManualResolution`はresourceを変えず`Unchanged`を返す。

external `ReadAgain | RetrySameEffect`のfresh actionは、同じresourceとeffect identityへ束縛したAttempt、claim / reservation、必要なscope fenceをexternal I/Oより先に一つのclosureでwrite-aheadする。その後も`EffectDispatchGate(effect_id)`がcurrent claim / state / fence / attempt generationをcall直前に再検査する。response loss / crash後はsame action IDでsame attemptへjoinまたは安全にreclaimし、`RetrySameEffect`は保存済み同一idempotency keyだけを使う。local `UseObservedResult | CancelIfSafe | KeepForManualResolution`はkind-specific resource / side stateとCompleted receiptを同じclosureで確定し、部分commitやgeneric result-only terminal化を行わない。

attempt statusは`Prepared | EffectReserved | OutcomeUnknown { transaction_id, payload_sha256 } | ReconciliationRequired { failure } | Completed`のclosed 5値である。Completedだけがcanonical safe resource resultとそのSHA-256を持ち、time passage、restart、plan detail compaction、unrelated resource stateの前進を理由に変更・GCしない。same action IDの再実行は保存済みdecisionへ収束し、current viewから新しいeffectを推測しない。

resource-isolated send input、operation / resource privacy purge、recovery action authority generation rotation、resource inventory、managed backup / restore、app-data resetは#1499のPhase 0 public/runtime contractではない。この節はそれらのtoken、tombstone、purge、backup、reset APIや物理schemaを定義しない。必要なdata lifecycle、physical reclamation、migrationはD3 / F3後続設計で独立した公開要件を確定してから追加する。

action resolverはobligation resultだけを更新するgeneric terminal pathを持たない。actionのauthoritative proofがeffect開始またはackだけを証明し、kindのterminal factを証明しない場合は、同じpending obligation、proof ref / safe observation、対応するowner / operation status、action attemptのCompleted receipt＋safe resource result（`receipt.outcome=RecoveryActionOutcome::Pending`）を一つのclosureで更新してPending outcomeを返す。terminal factを証明する場合だけ、`TurnExecution / QueueExecution / TerminalCommit`はR-020 canonical terminal closure、`ProviderEstablish`は依存元send operation status、`PermissionDelivery`はpermission settlement、`BackendRecovery`は`RecoveryPublication(Pending)` handoff、`RecoveryPublication`はcanonical message publication、`SessionClose`はSession / configuration / archive projection、`WorkflowShutdown`はworkflow stateを所有する既存kind-specific closureへ委譲する。各closureはcompact result、claim / pending index / scope fence削除、proof由来safe observation digest、必要なside state、action attemptのCompleted receipt＋safe resource resultを同じcommitへ入れる。required side participantを保存できなければresultだけを先にcompact化せず、元pending identityと同attemptを保持してOutcomeUnknownまたはReconciliationRequiredへ進める。

`resolve_pending_recovery_action`はobligation-state-by-IDを1 key lookupしPendingだけを対象にする。`ReadAgain`はAuthoritativeReadback capability＋provider-native correlation、`RetrySameEffect`はIdempotentRetry capability＋保存済みexact effect / idempotency key、`UseObservedResult`はcurrent revisionに束縛した保存済みauthoritative proof、`CancelIfSafe`はauthoritative `AuthoritativeNotFound | ConfirmedNoEffect` proofと`EffectDispatchGate(effect_id)`内の未開始再検査を必須とする。ただしQueueExecutionのcancel / rebaseは#1404がauthorityであるため、Phase 0 recovery actionとして`CancelIfSafe`を提示しない。ReadAgainのadapter responseはproof ref、pending revision、safe observationを一つのclosureで確定し、そのproofを同じaction closureで参照する。external I/Oを伴うactionは同じobligationのclaim generation / tokenを先にCASし、別obligation / effect identityを作らず、dispatch直前のeffect gate検査を必須とする。response loss / crash後もsame action / obligationへjoinまたは安全にreclaimし、RetrySameEffectは同じidempotency keyだけを使う。観測結果は上記kind-specific closureへ渡し、generic result-only terminal化を行わない。

`resolve_shutdown_target_action`はcurrent plan / details lookupより先にaction decisionをdirect lookupする。Completedなら保存済みresultをexact replayし、decision Absentまたはnonterminal Attemptの継続時だけdetails Availableのlatest current planをplan ID / epoch / target keyでdirect page lookupしてroot hash / plan revision / page hash / global ordinal / derived target-state hashを検証する。page refからtarget authorityを1 key lookupし、digest、plan / epoch / page index / target key / ordinal / obligation ID / expected owner revisionをbyte検証する。authorityが欠損または不一致ならraw owner / effectを再構築せずactionを出さない。valid authorityかつpublic stateが`Prepared`、またはpublic stateが`ReconciliationRequired`かつobservationが`ExitCoupledOutcomeUnknown`でobligationがAbsentなら、authorityのsame deterministic obligation IDを使う。`ExitCoupledOutcomeUnknown`を`ShutdownTargetPublicState`のvariantとして扱わない。`RetrySameEffect`はidempotent capabilityとcurrent scope fence Absentを確認し、pending EffectReserved、claim、pending index、scope fence、top-level claim generation Someのaction attempt EffectReservedをwrite-ahead target closureで同時確定する。その後`EffectDispatchGate(effect_id)`が同じclaim / state / fence / attempt generation / authority identityをI/O直前に再検査してからだけ同じeffectを呼ぶ。`ReadAgain / UseObservedResult / CancelIfSafe`はdurable authoritative proofを再検証し、Succeeded / ConfirmedNoEffectを証明できる場合だけ、authorityの`SessionClose / WorkflowShutdown`に対応するowner projection、同じdeterministic IDのcompact `Succeeded | CancelledBeforeEffect`、action attemptのCompleted receipt＋safe resource result、必要なclaim / index / fence cleanupを同じtarget closureで確定する。Ambiguousはeffectの結果が不明でもreadback action自体は正常完了しているため、authorityのexact blueprintを持つ`Pending(ReconciliationRequired)`、bounded reconciliation reason、claim state、scope fence、action attempt `Completed { outcome=Pending }`のcanonical safe resultを同じclosureへ入れる。同じaction IDはこの保存済みPending receipt / resource viewをexact replayする。attempt `ReconciliationRequired { failure }`はaction自身のresultまたはcommitを確定できない場合だけに使う。owner / side participantを保存できなければresultまたはattemptだけを先行させず元logical target / pending identityを保持する。pre-activation Cancelled、old-plan Superseded、authority不整合、details Compactedにはactionを出さず、startup / generic recoveryはexplicit commandを代行しない。

target result candidateを含め全targetがcompletedになる場合、Phase 0は最大32 page / 4096 pathをoff-writer foldし、source root、obligation-state / pending / scope-fence roots、latest-attempt pointer、target candidateをguardして、compact target result、plan rootの`Completed` transition、latest-attempt pointer更新、summaryを同じclosureで確定する。全target terminal後のplan stateは常にCompletedであり、preexisting recoveryまたはnon-success resultがあればsummary outcomeをExitedWithRecovery、それ以外をCompletedとする。`ReconciliationRequired` plan stateは未解決targetまたはfold integrity failureが残るnonterminal planだけに使い、全target terminalのcurrent blockerとして残さない。fold中のrevision変化はpartial resultをcommitせず再試行し、last-target resultだけを先にcommitしない。startup finalizerがlatest-attempt pointerから同じbounded foldを再開する対象は、以前のnon-last target closure、generic recovery、F3 / legacy import等により全targetが既にterminalなのにplan rootだけがnonterminalである既存互換状態に限定し、planを永久currentにしない。F3ではlast-target resultとplan counter / stateを同じdatabase transactionで確定する。

Session確立前のNew AgentはSession read modelに押し込まない。S9aは`get_agent_launch_preflight(workspace_id, provider_id, context)`から`Checking | Compatible(AgentBackendCapabilities) | ProtocolIncompatible(partial identity)`を取得する。`prepare_agent_launch`はattempt id/hashをreserveし、Bypassなら`AgentLaunchDraftPrepared + BypassChallengeIssued`を同じlocal batch、non-BypassならPrepared単独でappendする。Queue/Workflow Bypassも各Prepared＋ChallengeIssuedをatomic appendする。確認後の`start_agent_launch`は`StartAgentLaunch.bypass_confirmation`のchallenge id / nonceをIssued challengeへ照合し、draft hash、preflight context、期限、guard、caller scope、policy/gateも再検証できた場合だけ`BypassChallengeConsumed + AgentLaunchAttemptStarted`をlocal atomic batchでappendしてからprovider I/Oする。attempt id / draft hashだけではconsumeできない。draft変更・期限切れはreservation/challengeを失効させ、再prepareを要求する。

reserved attempt idで分離したdurable launch event streamから`get_agent_launch(attempt_id) -> AgentLaunchProjection`を再構築する。projectionは`Prepared / Started / PreparationExpired / PreparationCancelled`を含み、prepare後start前のreloadも復元する。`AgentLaunchChanged`はmutable fieldの取りこぼしを避けるため小さなfull projectionを運ぶ。購読は`watch_agent_launch(attempt_id, after_seq)`で、service内部の`open_watch`がcursorのreplay可否、required sourceのcommon watermarkへ固定したsnapshot lease取得、barrier後subscription/receiver登録を同じstorage transaction / commit lockで行う。`AgentLaunchWatchService`はbootstrap fenceまたは各notice commitへ厳密にpinされたlive fenceの`read_at`でprojectionを構築し、`finish_bootstrap / finish_update`後にtyped frameを返す。lag/lease失効/ProjectionBehind/gap/逆行では`close_watch`して部分結果を捨て、snapshotから再openするためget→subscribe間のraceを作らない。`seq`はattempt単位で単調増加する。Completed後もretention期間内はSession idへの相関を保持し、launch失敗・reconciliation・pre-session ProtocolIncompatibleを復元できる。

Goal履歴はcurrent projectionへfull-retentionしない。`get_goal_history(session_id, cursor, limit) -> GoalHistoryPage`と`get_goal_revision(session_id, goal_id, revision)`をevent logからpage/id lookupし、transition kind/result/time、before/after objective/status、source/evidence、launch相関を返す。`TurnStarted`のgoal id/revisionはこのrevision lookupで後から解決でき、Goal clear/replace後も当時のobjectiveを監査できる。

event log の `SessionConfigurationSelected / Activated` と Goal canonical event を唯一の durable commit point とする。`SessionMeta` の configuration / Goal snapshot は高速 projection/cache であり、event から再構築できる。cache 更新失敗は canonical provider drift ではなく `PersistFailure` と再投影で回復する。Workflow は `WorkflowExecutionStarted` と NodeExecution resolution event を commit point、WorkflowExecution metadata を projection とする。queue item と NodeExecution は effective configuration snapshot と `goal_id + goal_revision` を保持し、turn read model は provider/model/mode/effective effort/Goal/protocol identity を展開表示できる。

一般Sessionの購読は`watch_session(session_id, after_seq)`を唯一の入口とし、`AgentSessionWatchService`内部の`open_watch`がsnapshot/replay決定、required sourceのcommon watermarkへ固定したsnapshot lease取得、barrier後subscription/receiver登録を同じcommit境界で行う。bootstrapはhandleのfence、live更新は`receive`が当該notice commitへ厳密にpinした`LocalWatchUpdateFence`の`read_at`だけを使い、typed snapshot/deltaをmaterializeしてから`finish_bootstrap / finish_update`する。`get_session`後にそのseqでwatchした場合も、cursor以後を必ずreplayして「最後のeventだけ逃し次eventが無い」窓を作らない。cursorがretention外、またはlag/lease失効/ProjectionBehind/gap/逆行なら`close_watch`してfull snapshotから再openする。snapshot/deltaのsession単位seqは単調増加し、disconnect時も`close_watch`でsubscriptionを解放する（FE-3 / presentation P1）。

### 12. Wire 層の型付け（写像の入口）

- **V-D12a（合意済み）**: Codex は `codex-app-server-protocol` / `codex-protocol` 公式クレートをタグ固定 git 依存で導入し、手書き `serde_json::Value` 解釈を全廃する（ST-1）。
- **V-D12b**: Claude は Claude Agent SDK の型定義（`sdk.d.ts` の StdoutMessage union）を正とした typed model（serde struct/enum）を `infrastructure/agent_session/claude/wire.rs` に定義する（ST-2）。SDK バージョンを wire.rs に明記し、更新時に差分レビューする。
- WebSocketの`PayloadConflict`は次のtotal mappingを使い、outer tagをcommand別に分岐させない。

```rust
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

pub enum PayloadConflictIdentityV1 {
    Send { operation_id: String },
    Stop { request_id: String },
    ApplicationQuit { request_id: String },
}

pub enum SessionLifecyclePayloadConflictIdentityV1 {
    SessionLifecycle { request_id: String },
}
```

`SessionOperationFailureKindV1`はcanonical enumのexact 19 tag、`SafeOperationFailureV1`はcanonical failureのexact 5 fieldだけを持つclosed adaptor DTOである。`BoundedNoticeTextV1.original_bytes`はu64 semantic fieldなのでNoneまたは先頭ゼロなしcanonical decimal string、他fieldはcanonical typeとfield-for-fieldである。SafeOperationFailure内のlabel / detailではnested `correlation_id=None`を必須とし、failure identityはtop-level `correlation_id`一件だけにする。domain / usecase typeをwireへ直接serdeせず、unknown tag / field型、noncanonical integer、bounds超過、nested correlation identityの二重化をdecoderで拒否する。

same send operation ID / different exact payloadは`Send { operation_id }`、Stopとapplication quitのsame request ID / different targetまたはintentはそれぞれ`Stop { request_id }`、`ApplicationQuit { request_id }`へ写像する。WebSocket routeを持たないSessionLifecycleはTauri専用`SessionLifecyclePayloadConflictIdentityV1::SessionLifecycle { request_id }`へ写像する。Stop / quitの`request_id`を`operation_id`へ改名しない。Tauriはendpoint固有のgenerated application error enumを直接返し、表外variantを共通公開supersetやallowlist fallbackで受けない。これらはexact 19種の`SessionOperationFailureKind`へ追加せず、`SendAgentMessageResult`、`StopResult`、`ApplicationQuitResult`、`SessionLifecycleResult`のvariantにも追加しない。Accepted後failureとpost-usecase `OutcomeUnknown`は各embedded result / projectionのまま返し、同時にtransport errorを生成しない。
- Tauri / WebSocket共通presenterはV-P8のu64 semantic fieldだけをcanonical decimal stringへfield-for-field写像し、`9223372036854775807`をlosslessにround-tripする。bounded `limit / max_bytes`はJSON nonnegative integer、exit codeはJSON signed integerとして保持する。controllerはsemantic fieldのJSON number / noncanonical decimal、control / exit codeのstring・fraction・型 / route範囲外、one-based semantic fieldの`0`をdomain commandへ渡さず`InvalidRequest`にし、zero-based / Absent fieldの`0`は正規値として保持する。result decoderも同じvalidatorを使い、transport別のnumber coercionを作らない。
- mode / Goal / reasoning effort の capability、更新要求、ack / error、provider permission snapshot も typed request / response として定義し、文字列比較や frontend fallback に戻さない。Claude `/goal` は公開 typed RPC と偽装せず、side effect を宣言した typed `ProviderCliCommand` adapter とする。
- spawn した executable の `BackendProtocolIdentity` を initialize 時に検証する。compiled schema と互換でない binary、experimental flag、initialize capability の組合せでは session を開始しない。
- parse可能かつcontent-planeと分類できた未知message/partだけは、payload長・digest・content分類・固定上限以下のsecret-redacted sampleをV-P1の`Notice(UnsupportedMessage)`＋構造化ログへdurable記録する。既知variantのdecode failure、content/controlを分類できないmalformed frame、size上限超過も、長さ・digest・分類/失敗種別・bounded redacted sampleと取得済みの部分protocol identityだけを`ProtocolIncompatible`へ記録し、full bodyをevent/logへ恒久保存せず新規turnをblockする。完全evidenceが必要な場合だけ暗号化・per-session quota・object size上限・TTL・参照認可付きstoreへ保存して`ProviderEvidenceRef`で参照し、secret plaintextはstoreにも保存しない。control-plane未知値/variantは`ProtocolIncompatible`または対象aggregateのreconciliationへ着地させる。各分類の件数をparityテスト（ST-7）で別々に検証する。

## トレーサビリティ（本書が解消する問題）

| 問題 ID | 設計要素 |
|---|---|
| CL-3, CL-4, RG-3 | V-D7 TurnStopReason / TurnStats |
| CL-5 | V-D4 NoticeKind::McpServerStatus |
| CL-6, RG-7 | V-D2 ToolOutputBlock::Image ＋ §10 ImageRecorded 配線 |
| CL-7 | V-D10 `AgentMode::Plan` への全域写像 |
| CX-1（語彙部分） | V-D6 PermissionQuestion.id / is_secret / is_other_allowed / PermissionAnswers |
| CX-3, RG-1 | §1 Thinking（既存 part）への Codex reasoning delta 写像（wire 対応は V-D12a） |
| CX-5, RG-2, RG-5 | V-D3 TodoListItem 拡張＋Codex plan 写像 |
| CX-7, RG-6 | V-D4 Notice |
| CX-8 | V-D5 Error.retryable / resolved |
| CX-10 | V-D2 ToolKind（Task / Image / Review） |
| CX-11 | V-D2 ToolKind::WebSearch（query / 結果を input / output に保持） |
| SD-5 | V-D2 ToolCallStatus による in-flight 判定 |
| SD-6 | V-D6 ApprovalDisplay |
| SD-7 | V-D4 Notice(Compaction).status=Failed |
| RT-5（語彙部分） | V-D7 TurnError |
| RG-4, RG-8 | V-D2 ToolCallStatus / exit_code |
| RG-9, FE-4（語彙部分） | V-D7 TurnStats / V-D8 TokenUsage |
| ST-1, ST-2 | V-D12 wire 型付け |
| ST-6 | V-D1 MessagePart 単一定義 |
| ST-7（語彙前提） | 全域: parity テストは本語彙上で「同等イベント列」を定義する |
| #1445, #1446 | V-D10 `AgentSessionConfiguration` / 永続化 / migration |
| #1447 | V-D10 5 mode の cross-backend 写像 |
| #1448 | V-D10 `ReasoningEffort` / model capability（V-D8 `TokenUsage` とは別概念） |
| #1449 | V-D10 `AgentGoal` lifecycle / provider capability |
| #1450 | V-D10 workflow template / resolved launch configuration / queue snapshot と §10 durable event |
| #1451 | §11 backend-owned read model / available actions と V-D10 capability-driven UI 契約 |
| #1499 | Automatic Phase 0 bootstrap / authority pointer / bounded physical identity、caller-keyed single send operation / immutable acceptance receipt＋mutable execution status / external-effect obligation / authoritative proof / recovery action replay / feedback identity / typed persistence failure / Phase 0 closure bridge / `UserInputAccepted` / `AssistantMessageOpened` |

**語彙変更が不要な独立修正**（本ドキュメント群の設計を待たずに着手可能）: CX-4（tokenUsage フィールド名）、CX-9（initialize commands の dead code — V-D12a に内包可）、OB-7（画像のみ送信時の空 text block）。CX-1 の wire 形式修正は V-D6 の型を前提に行う。

## 確定事項（2026-07-07、2026-07-15 レビューで確定）

1. **ToolCall 統合（V-D2）**: 単一 `ToolCall` part への統合を**採用**。durable event は既存種を残し projector で合成する移行方式。
2. **Agent mode の表現（V-D10、2026-07-15 改訂）**: 旧 3 値 enum ＋ `plan_mode: bool` の決定を supersede し、`Ask / Edit / Plan / Auto / Bypass` の排他的 5 値 enum を**採用**する。
3. **Notice の持ち方（V-D4）**: transcript の part として記録する方式を**採用**（session-level 別ストリーム案は不採用）。RateLimit 等の最新値は read model 側で導出する。
4. **Goal の所有者（V-D10）**: Session ごとに current Goal 最大 1 件を、configuration から独立した id / revision / pending / sync state を持つ Rust-owned aggregate として永続化する。provider 差分は strategy / scope / effects を含む `Native / Emulated / Unsupported` capability で扱う。
5. **工数の意味（V-D10）**: model の応答・推論強度を表す `ReasoningEffort` を指す。selected と effective / unknown を分け、選択肢は provider API または protocol identity に pin した compatibility table 駆動とする。TokenUsage、cost、時間、turn 数、各種 budget や厳密な上限と混同しない。
6. **設定確定（V-P6 / V-D10）**: frontend は selected / effective / pending projection の mirror とする。Rust は discriminated patch、write-ahead intent、canonical event commit、activation、reconciliation を管理し、外部 provider との atomicity を仮定しない。turn 送信時の上書き・silent fallback を行わない。
7. **Auto / Bypass（V-D10）**: Auto の判定主体は provider classifier / reviewer であり、Releash は境界を広げず結果を監査する。Bypass は Rust usecase の managed-policy 検査と二段階確認を必須とし、workflow checkpoint を越えない。
8. **protocol compatibility（V-D12）**: generated schema と実行 CLI の `BackendProtocolIdentity` を initialize 時に照合する。control-plane drift は `ProtocolIncompatible` として fail-closed にする。
9. **Phase 0 durable closure（#1499）**: existing `send_agent_message`へcaller-keyed operation IDをadditiveに渡すsingle send operation、immutable acceptance receipt＋mutable execution status、terminal identity、全external effectのpre-reserved obligation、kind / payload / effect / owner closed matrix、durable authoritative proof、response-loss replay可能なaction attempt、Session / Workflow / Application scopeのschema-versioned redo manifestを採用する。external I/Oはreservation後も`EffectDispatchGate(effect_id)`がclaim / state / fenceをcall直前に再検査する。Application shutdownは全target prepare→global activation→current coordinatorによるper-target EffectReservedを明示shutdown commandのgateとし、startup / late commitからcommandをblind起動しない。全target terminal時のplanはCompleted、non-success / preexisting recoveryはsummary ExitedWithRecoveryとする。Activated後exitのpipe / job object / parent-death影響は`ExitCoupledOutcomeUnknown`としてreadbackする。初回upgradeはexclusive lock / admission closedのautomatic bootstrapでlegacyをbounded importし、parity後のauthority pointer CASだけでPhase0へ切り替える。bootstrap中のquitだけはnormal planを作らない15秒bounded exitを使う。切替後はlegacy fallback / dual writeをせず、F3もSQLite transactionへone-shot importする。
