# Agent チャット正規化語彙・データ構造の理想形

作成日: 2026-07-07
更新日: 2026-07-15（Agent 実行設定を追加）

milestone 84「Agentチャット安定化」のドキュメント群:

- [agent-chat-instability-audit.md](agent-chat-instability-audit.md) — 問題点インベントリ（全 66 件、要求リスト）
- **agent-chat-ideal-vocabulary.md（本書）** — 正規化語彙・データ構造の理想形
- [agent-chat-ideal-lifecycle.md](agent-chat-ideal-lifecycle.md) — ライフサイクルの理想形（不変条件）
- [agent-chat-ideal-presentation.md](agent-chat-ideal-presentation.md) — UI 表示の理想形

本書は「Claude / Codex から届く事象を、何という語彙に正規化するか」の正本を定義する。監査で確定した dropped / divergent 問題群の解消先であり、ライフサイクル・表示の 2 文書はこの語彙を前提とする。問題 ID（CL-x 等）は監査ドキュメントを参照。

## 設計原則

- **V-P1 (no-silent-drop / fail-closed control plane)**: wire 層は届いたメッセージを無言破棄してはならない。content-plane の変換先が無い既知・未知メッセージは `Notice(kind=UnsupportedMessage)` と構造化ログへ着地させ、session を継続できる。permission / configuration ack、Goal、provider mode / reviewer、turn completion、応答必須 server request など control-plane の未知・decode 失敗は raw ref と protocol identity を `ProtocolIncompatible` に記録し、新規 turn を fail-closed で block する。active turn中ならpending permissionをcancelしprovider interrupt後に`Interrupted(ProtocolIncompatible)`で必ずfinalizeして、spinner/dialogを残さない。「捨てる」は明示的な設計判断としてのみ許され、本書に記録する。
- **V-P2 (parity)**: 同一概念は backend に依らず同一の語彙要素へ写像する。backend 固有の概念（Codex の item 種別等）は新しい part 種を増やすのではなく、既存語彙の kind / フィールドへ写像する。
- **V-P3 (durable 表現可能性)**: UI に表示されるべき全情報は、この語彙（part / turn outcome / usage / notice）で表現でき、durable event として記録できなければならない。transient にしか存在しない表示情報を作らない。
- **V-P4 (additive 進化)**: 永続化される語彙（durable event / read model）の変更は additive-only とし、既存セッションの読み込み互換を壊さない。
- **V-P5 (full-retention 回避)**: 語彙拡張はサマリ・参照（`ToolOutputRef`）・スナップショットで表現し、wire の生ペイロード全量を恒久保存しない。
- **V-P6 (Rust-owned configuration)**: Agent の実行設定は Releash の Rust backend が正本を所有する。frontend は capability と確定済み設定の mirror に留め、adapter の受理前に確定表示しない。turn 送信時の frontend 値で設定を上書きしない。

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
| `AgentSessionEvent`（durable） | `usecase/agent_session/event_log/events.rs:104` | 上記の拡張を受ける器が無い。`TurnTokenUsage` が input/output のみ |
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
    pub level: NoticeLevel,          // Info / Warning / Error
    pub kind: NoticeKind,
    pub label: String,               // 一覧表示用の短文
    pub detail: Option<String>,      // 展開表示用
    pub status: Option<NoticeStatus>,   // InProgress / Completed / Failed（compaction 等の進行型）
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

// provider送信直前だけephemeral memoryに置く。event/read modelへ保存しない。
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
    pub has_ephemeral_secret_answers: bool,
    pub edited_payload: Option<JsonPayload>,
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
- user/rule/autoの回答はprovider送信前に`PermissionResponseRequested(PermissionResponseIntent)`をappendして`Responding`へ移す。ack後だけ`PermissionResolved`をappendする。providerが要求はまだPendingのまま回答だけを明示rejectした場合は、理由付き`PermissionResponseRejected(PermissionResponseRejection)`をappendして同じrequestを`Pending`へ戻し、旧response idを終端する。timeout/restartはresponse idとrequest cancel/tool start observationを相関してeffectiveを判定し、確定不能なら`ReconciliationRequired`として再回答を禁止する。同じresponse idのidempotent recoveryだけを許す。
- `is_secret` questionのplaintextはprovider送信用ephemeral memoryだけに置き、event log、message/session store、read model、log、backupへ保存しない。durable intent/resultは`Redacted { answered }`だけを持ちfingerprintも作らない。secretを含むresponseがrejectされた時点で旧ephemeral値を破棄する。crash後もplaintext再送できないため、providerがまだpendingと確認できた場合だけ新しいresponse idで再入力を要求し、旧responseを自動retryしない。
- `decision_reason` / `description` は現行フィールドを維持し、表示まで配線する（FE-7 は presentation 側）。
- **id の合成**: Claude の AskUserQuestion は wire 上 question id を持たないため、変換層で安定 id（出現順の `q0`, `q1`…）を合成し、ephemeral `PermissionAnswerInput`を backend ごとの期待形式（Codex: id キーの `{answers: {<id>: {answers: [..]}}}`、Claude: 質問順ベース）へ逆写像する。写像は各 backend の permission モジュールが所有する。
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
        reason: InterruptReason,     // UserAbort / Timeout / Crash / SessionClosed を追加
        error: Option<String>,
        stats: TurnStats,            // 中断でも usage を失わない
    },
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
- `SessionConfigurationChanged(AgentSessionConfigurationProjection)` — selected / effective / pending / reconciliation の authoritative projection を通知。旧 `PermissionModeChanged` を置換する
- `PermissionRequested` / `SlashCommandsUpdated` / `KeepAlive` / `Fatal` — 現行維持

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
    pub raw_status: String,
    pub objective: Option<String>,
    pub token_budget: Option<u64>,           // read-only。ReasoningEffort とは無関係
    pub tokens_used: Option<u64>,
    pub time_used_seconds: Option<u64>,
    pub evaluated_turns: Option<u64>,
    pub latest_evaluator_reason: Option<String>,
    pub created_at: Option<String>,
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

pub struct GoalPrecommitControlConflictObservation {
    pub observation_id: String,
    pub transition_id: String,
    pub command_uuid: Option<String>,
    pub control_kind: String,
    pub raw_control_ref: String,
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
    pub raw_status: Option<String>,
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
    SetMode(AgentMode),
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
    pub raw_mode: Option<String>,
    pub permission_snapshot: Option<ProviderPermissionSnapshot>,
    pub reasoning_effort: Option<String>,
    pub raw_control_ref: Option<String>,
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

pub struct AgentSessionConfigurationProjection {
    pub selected: AgentSessionConfiguration, // adapter が受理したユーザー選択
    pub effective: AgentEffectiveConfiguration, // provider が現在使用中の値
    pub pending_update: Option<PendingConfigurationUpdate>,
    pub sync_state: ConfigurationSyncState,
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

pub enum TurnConfigurationSource {
    SessionEffective { revision: u64 },
    QueueItem {
        item_id: String,
        item_revision: u64,
        execution_id: String,
        snapshot_hash: String,
    },
    WorkflowNode {
        run_id: String,
        node_id: String,
        execution_attempt_id: String,
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
    Ready(AgentSessionConfigurationProjection),
    NeedsConfigurationResolution(ConfigurationResolutionProblem),
}

pub enum ConfigurationResolutionScope {
    Session,
    QueueItem { item_id: String },
    WorkflowRun { run_id: String },
    WorkflowNode { run_id: String, node_id: String },
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
    WorkflowNode {
        run_id: String,
        node_id: String,
        execution_attempt_id: String,
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
    pub bypass_challenge_id: Option<String>,
}

pub enum AgentLaunchOrigin {
    Manual,
    WorkflowNode {
        run_id: String,
        node_id: String,
        execution_attempt_id: String,
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
    RunDefault { revision: u64 },
    NodeOverride { revision: u64 },
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
    pub run_default_revision: u64,
    pub node_override_revision: u64,
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
    pub run_id: String,
    pub node_id: String,
    pub execution_attempt_id: String,
    pub resolution_id: String,
    pub resolved_configuration_hash: String,
    pub challenge: BypassConfirmationChallenge,
}

pub struct WorkflowNodeBypassPrepared {
    pub waiting: WorkflowWaitingBypassConfirmation,
    pub expected_workflow_seq: u64,
    pub prepared_at: String,
}

pub enum WorkflowNodeExecutionGateState {
    Ready,
    WaitingConfiguration(WorkflowWaitingConfiguration),
    WaitingBypassConfirmation(WorkflowWaitingBypassConfirmation),
    BypassConfirmationExpired(WorkflowWaitingBypassConfirmation),
    Starting { execution_attempt_id: String },
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
}

pub enum AutoOperationalState {
    NotApplicable,
    InProgress,
    Active,
    ManualFallback { reason: String },
    TimedOut,
    Aborted,
}

pub enum ProviderPermissionSnapshot {
    Claude {
        permission_mode: String,
        auto_state: AutoOperationalState,
        allow_dangerously_skip_permissions: bool,
    },
    Codex {
        sandbox_policy: String,
        approval_policy: String,
        approvals_reviewer: String,
        collaboration_mode: Option<String>,
        permission_profile_id: Option<String>,
        auto_state: AutoOperationalState,
    },
}

pub struct ProviderPermissionState {
    pub snapshot: ProviderPermissionSnapshot,
    pub normalized_mode: Option<AgentMode>,
    pub residual_protections: Vec<ResidualProtection>,
}
```

`AgentBackendCapabilities` は pin した schema version と application scope を含む。Goalとeffortのavailabilityはschema上の存在だけでなく、workspace trust、session、managed policy、deployment/organization上限、capability overrideを含む実行contextで再評価し、source/context hash/checked atを返す。`SessionGoalProjection.available_actions` は raw capability と現在 status、pending transition、managed policy を Rust query が評価した結果であり、frontend は遷移表を再実装しない。

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
- Claude の `dontAsk` は `Auto` と同義ではない。Claude / Codex の複合的な permission state は `ProviderPermissionSnapshot` に損失なく保持し、`normalized_mode=None` の session は `ReconciliationRequired` として turn を開始しない。
- `Bypass` は通常のprovider操作承認を最大限迂回するintentであり、provider固有のresidual protections、Releash managed policy、workflow human checkpoint、承認 node、停止条件を迂回しない。`BypassConfirmationChallenge` は target mode、期限、nonce に加え、Sessionなら`session_id + selected_revision`、New Agentなら`attempt_id + canonical_draft_hash`、Queueなら`session_id + item_id + execution_id + snapshot_hash`、Workflowなら`run_id + node_id + execution_attempt_id + resolution_id + resolved_configuration_hash`、reconciliationならscope＋resolution attempt＋expected observation/seq＋action/target hashへ束縛する。Launch/Queue/Workflowのprepare eventと`BypassChallengeIssued`は同じlocal atomic batchでappendする。provider I/O前にRust usecaseがmanaged policyとguardを再検査し、`BypassChallengeConsumed`を各durable intentと同じlocal atomic batchでappendする。provider I/O中にlockは保持しない。provider失敗後もconsumedのままとし、同一intent id・同一guardのidempotent retryだけが再利用できる。waiting projectionはchallenge全体を保持し、reload後もguard/期限/residual protectionsを復元する。template に `Bypass` を保存しても権限付与にはならない。

#### AgentGoal

- Goal 本体と lifecycle の正本は常に Releash に置き、configuration aggregate から完全に分離する。`SessionGoalProjection.current_goal` は同時に最大 1 件で、Active / Paused / Blocked のときだけ active と呼ぶ。Completed / Failed も clear または次の set までは current として履歴表示し、Goal の set / edit / transition / clear で configuration revision を進めない。
- `goal_id` と provider の opaque ref で通知を相関し、置換前 Goal の遅延 completion が新 Goal を完了させないようにする。raw observationの`ProviderGoalSnapshot`自身へprovider refと`Matched { goal_id, revision } / Unmatched / Ambiguous`をdurableに保存し、crash/replay後も相関判定を再現する。Unmatched/Ambiguousをcurrent Goalへ適用しない。transition は `source`（User / Provider / Evaluator / System）、理由、時刻、任意の `evidence_ref` を記録する。
- Goal capability は `set / edit / clear / pause / resume / readback / completion_event / auto_continuation / max_objective_length` を項目別に `Native / Emulated / Unsupported(reason)` で返す。各actionはmode同様に`schema_supported / runtime_available / availability_source / availability_context_hash / unavailable_reason / checked_at`を持ち、workspace/session/managed-policy context変更時に再評価する。adapter の適用戦略は `ProviderNativeRpc`、`ProviderCliCommand`、明示した `ReleashManagedEvaluator`、`Unsupported` のいずれかとし、暗黙の prompt 接頭辞で対応済みに見せない。
- Codex `thread/goal/set|get|clear`・goal notification は pin した typed RPC adapter（`ProviderNativeRpc`）で扱う。status は `active → Active`、`paused → Paused`、`complete → Completed`、`blocked / usageLimited / budgetLimited → Blocked` と全域写像し、raw status と read-only accounting を `ProviderGoalSnapshot` に保持する。unknown status は raw snapshot を失わず Goal reconciliation に入り、`Failed` は Releash / System 固有で Codex native status とは扱わない。objective変更はset RPCによる`Edit` emulationとして`ReplacesProviderGoalIdentity / ResetsProviderProgress`を宣言する。
- Claude で公開確認できる surface は typed Goal RPC ではなく `/goal` CLI command（`ProviderCliCommand`）であり、setとactive Goalのobjective変更はGoal保存/置換と同時にturnを開始する。`Set`は`StartsTurn`、`Edit`は`StartsTurn / ReplacesProviderGoalIdentity / ResetsProviderProgress`を宣言する。pinしたCLI fixtureで`system/command_lifecycle(completed, command_uuid)`とtyped Goal state (`goal_set`/`goal_status`またはactive Goal snapshot)の両方を観測し、要求objective hash一致を確認した`ClaudeGoalCommandEvidence`だけをacceptance evidenceにする。content-plane deltaだけをbufferし、evidence後に`ProviderGoalCommandEvidenceObserved + GoalSet/GoalTransitioned + TurnStarted`をatomic appendして公開する。commit前の`can_use_tool`/`request_user_dialog`等の応答必須control-planeはbufferせずfail-closed応答→interruptし、`GoalPrecommitControlConflictObserved`を保存してGoal/turn reconciliationへ送る。shape/order/相関をfixtureで証明できないCLI versionではStartsTurn actionを`Unsupported`にする。
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

1. user 起点の execution-affecting 更新は初期実装では `Idle` のみ受け付ける。Session共通`SessionControlOperationLease`を取得し、`base_selected_revision` を CAS 検証してcapabilityとmanaged policyを確認する。`sync_state != Synced`、Goal sync stateが非Synced、Goal transition pending、別control lease中は次の更新を拒否する。`Bypass` は一回限りの confirmation challenge も検証する。
2. provider I/O の前に `ConfigurationUpdateRequested { update_id, base_selected_revision, target_revision, patch, applies_from }` を event log へ append する。append 成功が durable intent の commit point であり、失敗したら provider へ送らない。
3. `Live` は adapter が provider-native 更新または明示された Releash-managed strategy を直ちに適用する。`NextTurn / SessionRestart` で独立した provider 設定 API が無い場合、adapter は typed capability に基づき staging を受理するだけで、provider 適用済みとは報告しない。複数 provider field が必要なら adapter 内で順序・補償を所有し、部分成功を success として返さない。
4. live の provider ack、または next-turn / restart staging の adapter acceptance 後に `SessionConfigurationSelected` event を appendし、selected configuration の唯一の durable commit point とする。live は `SessionConfigurationActivated` も appendして effective revision を進める。next-turn / restart は pending request を保持したまま `AwaitingNextTurn / AwaitingRestart` とし、実際の provider activation event append まで effective を進めない。
5. providerが独立configuration APIを持つ`AwaitingNextTurn`は、次turnのprovider startより前にselected patchを適用し、activation ackと`SessionConfigurationActivated` appendが完了してからeffective snapshotでturnを開始する。`AwaitingRestart`もrestart/readback後に同じ順序でactivateする。
6. model/mode/effortを`turn/start` payloadでしか適用できないproviderでは、activation前`TurnStarted`を要求しない。`TurnStartRequested.execution_configuration`は、既に確定したSession/queue値なら`ExistingEffective(ResolvedTurnConfiguration)`、pending selectedなら`ActivateSelected { selected, originating_update_id, canonical_target_hash, prevalidated_context_hash }`とし、provider ack前のtargetをeffective型へ詰めない。queue起点はitemのcanonical semantic snapshotとitem revision/execution id/hashを固定し、current selectedから再構成しない。ack/readback後に初めてactual effectiveを`TurnStarted`へ保存する。`SessionConfigurationActivated`は`ActivateSelected`をsession-scopeで実際にactivateした場合だけappendし、queue per-turn overrideは`QueueItemStarted + TurnStarted`だけをatomic commitしてSession effectiveを変更しない。timeout/ack不明は`TurnStartReconciliation`へ移す。Reuse/Accept成功とCancel/CleanUp成功はqueue terminal/message markerまで同じbatchで閉じ、未完了intentは同じinput/correlationで回復する。
7. `SessionConfigurationSelected`前のprovider rejectだけは`ConfigurationUpdateRejected`を記録してpendingを消し、旧selected/effectiveを維持する。selected commit後のNextTurn/Restart activation reject・timeoutはselectedを巻き戻さず、new selected / old effectiveのまま`ConfigurationReconciliationRequired`としてturn / queue drain / workflow resumeをblockする。明示rollbackを選んだ場合は旧effective相当を新しいselected revisionとしてcanonical eventへappendし、revisionを逆行させない。ack後のcanonical event append失敗、部分成功、provider-originated競合も同じreconciliationへ送る。`ProviderConfigurationStateObserved`はmodel/effortに加え、Claudeのraw modeまたはCodexのsandbox/approval/reviewer/collaboration preset等を`ProviderPermissionSnapshot`として複合状態のまま保持し、raw control refも残す。readbackで全fieldを確認できない場合は安全なrollback/acceptをallowed actionsへ出さない。readback / idempotent reapply / rollback /明示acceptを`ConfigurationReconciled`で確定する。reconciliation自身には`reconciliation_id`を発行し、local request由来のときだけ`originating_update_id`、provider観測があるときだけ`observation_id`を関連付ける。provider-originated driftのために架空のupdateを合成しない。結果がBypassになる解決は汎用`BypassChallengeGuard::Reconciliation`でscope、resolution attempt、observation/seq、action/target hashへ束縛し、fresh challengeとpolicy/gate再検査を必須にする。

idempotence は revision だけでなく `update_id` で判定する。`ProviderConfigurationStateObserved`のappend自体で`ObservationPending { observation_id }`へ遷移し、canonical activation/no-change acceptance/reconciliation eventが同じobservation idをconsumeするまで新規turnをblockする。restart時は未consumed observationを再評価する。provider-originated change は pending update と同値なら ack として扱い、異なる場合は上書きせず reconciliation に入る。`SessionMeta` は event log から再構築できる projection/cache であり、cache 更新失敗は `PersistFailure` と再投影で回復する。canonical event append 失敗と同一視しない。

transientな`BackendSessionCleared`を受けたらresume metadata clearと`BackendSessionRecoveryStarted`を同じlocal atomic commitにし、configuration/Goalを`RecoveringBackendSession { recovery_id }`へ移してturn/queue/workflow resumeをblockする。新provider sessionでselected configurationをreapply/readbackし、readback observation idをconsumeする`SessionConfigurationReactivated`をappendする。GoalのNone/terminal/no-change/restoreを網羅した`SessionGoalReactivated + BackendSessionRecoveryCompleted`を同じ最終batchでappendして初めてSynced/公開し、Goal restoreがStartsTurnなら`TurnStarted`もそのbatchへ含めearly streamをbufferする。Goal readback pathもconsumed observation idを保存する。結果不明はGoal/turnまたはconfiguration reconciliationへ送る。

全ての`TurnStarted`、queue drain、backend resume、workflow resume直前にprovider/model/mode/reasoning effortとGoal continuation capabilityを、deployment/org override、workspace trust、managed policy、provider launch gate/residual protectionsを含む最新availability context hashで再評価する。authoritative effort validation不能やeffective Bypassのpolicy/gate失効は送信せずUnknown/`NeedsConfigurationResolution`/reconciliationへ移す。成功時のprovider permission/effects/residual protections/context hashは`EffectiveModeSnapshot`へ固定し、`TurnStarted`のimmutable execution snapshotとして監査する。

queueではenqueue snapshotのcanonical semantic hashを再評価結果と比較する。provider/model/mode/effort/effects/residual protections/policyの意味が変わればsilent差替えせず`QueueItemResolutionRequired`へ送り、`QueueItemRebased`だけがsnapshotを更新する。`checked_at/evaluated_at`等の観測時刻はsemantic hashから除外し、意味が同じ場合はsource snapshot/hashを維持したまま最新revalidation evidenceをTurnStartedへ保存する。Bypass prepareのchallengeにも最新effects/residual protectionsを含める。

configuration / Goal / launch / turn-start / permission reconciliationの解決操作も共通write-ahead sagaを通す。`reconciliation_id`、expected observation id/projection seq、action、target hashをCAS検証してresolution attemptをreserveする。結果がBypassならscope＋attempt＋observation/seq＋action/target hashへ束縛したchallengeを発行し、provider I/O前に`ReconciliationResolutionRequested { resolution_attempt_id, ... }`＋consumeを同じbatchでappendして`ResolvingReconciliation`へ排他遷移する。provider ack/readback後だけ対象`*Reconciled`をappendする。permissionのReenterSecretはPending確認後に旧responseを終端して新response idを発行しplaintextを保存しない。未完了resolution intentはrestart時に同じattempt id/correlationでreadback/recoveryし、actionやresponseを二重実行しない。

初回launchの部分成功や成否不明は`LaunchReconciliation`へ移す。`attempt_id`から安定生成したprovider create correlation keyと、provider対応時のidempotency keyをrequest前にdurable化する。各`StageAdvanced`はprovider ref、local session id等のstage payloadも保存する。provider readbackはcreate lookupの`Found { provider_ref, matched_by } / NotFound { consistency, stable_since } / Ambiguous { candidate_refs } / Unsupported`とconfiguration observationを`LaunchProviderObservation`へ保存する。Reuseは一意なFoundだけとする。Recreateはauthoritative NotFound、または同じidempotency keyでcreate自体が安全な場合だけallowedにする。eventual NotFoundはstability windowを記録してもidempotency key無しRecreateの根拠にせず、Ambiguous/Unsupported/存在不明でも出さない。reconciliationは`reconciliation_id`、最後に完了した`LaunchStage`、opaqueなprovisional provider ref、local session id、provider観測、観測できた範囲のprotocol identity、provider create recovery capability、`CleanUp / ReadBack / Reuse / Recreate / Cancel`のallowed actionsを保持する。initialize完了前のdecode failureでも部分的な`ObservedProtocolIdentity`とexpected hash/raw control refを`AgentLaunchAttemptStatus::ProtocolIncompatible`へ保存する。`AgentLaunchAttempt`とBypass challengeは同じ`canonical_draft_hash`を共有し、draft変更後のchallenge再利用を拒否する。

provider resource作成とinitial configurationのapply/readbackが完了したらsession idを確保し、`SessionCreated + SessionConfigurationSelected(revision=1) + SessionConfigurationActivated(revision=1) + LaunchStageAdvanced(LocalSessionCommitted)`をlaunch/session stream横断のlocal atomic batchでappendする。initial Goalが無ければ`AgentLaunchCompleted`も同じbatchへ含める。これが新Session configurationのcanonical seedで、SessionMetaは後から再投影する。batch失敗時はlocal Sessionを公開せずlaunch reconciliationへ入り、provider effectiveなのにlocal selected/effectiveが無い窓を旧値fallbackで隠さない。

launch draftの`initial_goal`はconfigurationに混ぜてprovider createへ送らず、`LocalSessionCommitted { session_id }`後に独立Goal sagaへhandoffする。`LaunchStageAdvanced(InitialGoalTransitionRequested)`と`GoalTransitionRequested { originating_launch_attempt_id }`を同じlocal event transactionでcommitしてからprovider I/Oを行う。initial Goalのcanonical Goal event、Claudeならevidence付き`TurnStarted`、`LaunchStageAdvanced(InitialGoalCommitted)`、`AgentLaunchCompleted`も同じtransactionで確定する。それまでは`WaitingForInitialGoal`とする。Goal結果不明は同じtransition idでreconcileする。provider明示rejectはGoal streamの`GoalTransitionRejected`とlaunch streamの`LaunchInitialGoalRejected`を同じtransactionでcommitし、reload後も`WaitingForInitialGoalResolution`へ投影する。

RetryGoal / ContinueWithoutGoal / CancelSessionはexpected transition/attempt seqをCASし、`InitialGoalResolutionRequested { resolution_attempt_id, action }`をappendして`ResolvingInitialGoalFailure`へ移す。RetryGoalは`InitialGoalResolutionCompleted { action: RetryGoal, next_transition_id }`＋新transition idのlaunch/Goal intentを同じtransactionでcommitして`WaitingForInitialGoal`へ移す。ContinueWithoutGoalはresolution completed＋launch completed、CancelSessionはcleanup後にresolution completed＋launch cancelled＋session closedを同じtransactionで確定する。結果不明は同resolution attemptのLaunchReconciliationへ移し、暗黙retryしない。

Workflow originでは全modeで`execution_attempt_id + resolution_id`からstableなattempt idを導出し、`WorkflowNodeExecutionRequested + AgentLaunchAttemptStarted`をworkflow/launch stream横断の同じtransactionで開始する。Bypassだけは先に`WorkflowNodeBypassPrepared + BypassChallengeIssued`をcommitして待機し、確認後の共通開始transactionへ`BypassChallengeConsumed`を追加する。完了時は`AgentLaunchCompleted + WorkflowNodeAgentBound`、失敗/取消時は`AgentLaunchFailed/Cancelled + WorkflowNodeAgentLaunchFailed/Cancelled`を同じtransactionで確定し、retryは新execution attempt idを使う。

#### Provider 仕様の根拠と pinning

- Claude: [permission modes](https://code.claude.com/docs/en/permission-modes)、[Goal / requirements](https://code.claude.com/docs/en/goal#requirements)、[Effort](https://platform.claude.com/docs/en/build-with-claude/effort)、[organization effort limits](https://code.claude.com/docs/en/model-config#organization-effort-limits) を規範入力とする。
- Codex: [App Server](https://learn.chatgpt.com/docs/app-server)、[Auto-review](https://learn.chatgpt.com/docs/sandboxing/auto-review)、[long-running Goal](https://learn.chatgpt.com/docs/long-running-work) と、[openai/codex app-server README / generated schema](https://github.com/openai/codex/tree/main/codex-rs/app-server) を規範入力とする。
- living docs は調査・意味の根拠、実装 wire の規範は dependency に pin した CLI / SDK tag が生成する schema と fixture とする。ただし schema だけを pin して PATH 上の別 version を起動してはならない。
- initialize 前後に `BackendProtocolIdentity { executable_version, schema_tag, commit_sha, schema_hash, experimental_flags, initialize_capabilities_hash }` を取得し、compiled adapter の compatibility manifest と照合する。Codex schema の experimental flag、Claude/Codex launch gate、runtime capability も identity に含める。
- 不一致や control-plane decode failure は低強調 `UnsupportedMessage` で続行せず、Session確立後はsession-level、確立前はdurable launch attemptの`ProtocolIncompatible`としてfail-closedにする。initialize途中で全identity fieldを取得できない場合も`ObservedProtocolIdentity`の取得済みfieldとraw control refを失わない。version 更新時は mode availability、Goal status / RPC、reasoning effort option、approval / sandbox / reviewer field の差分を D1 と parity fixture で review する。

#### 旧データの移行

| 旧設定 | 新 `AgentMode` |
|---|---|
| `plan_mode = true` | `Plan` |
| `permission_mode = Ask` / legacy `readonly` | `Ask` |
| `permission_mode = Edit` | `Edit` |
| `permission_mode = Full` | `NeedsConfigurationResolution(LegacyBypassConfirmationRequired)` |

`plan_mode = true` を permission mode より優先する。既知の model 値も `ProviderModelRef` へ移し、selected effort は `ProviderDefault`、effective は pinned table / readback で判定できる場合だけ concrete value、できなければ理由付き `Unknown` とする。Full以外の既知legacy値は`selected.revision = effective.revision = 1`へidempotentにlazy migrationし、自動write-backしない。legacy Full Sessionはsendをblockし、fresh challenge、managed policy、runtime availability、provider gateを確認してから新しいrevisionのBypassとしてcommitする。workflow templateのFullはBypass intentへ移せても権限付与ではなく、既存Run/queueを含む各executionで新challengeを必須とする。次の成功した設定writeでcurrent schemaとmigration audit eventを保存する。

mode / model の欠損・未知値を `Edit` 等へ既定化せず、scope、field、raw payload、resolution id、allowed actions を持つ `NeedsConfigurationResolution` として turn / queue drain / workflow resume を block する。migration 対象は SessionMeta だけでなく、既存 queue item、workflow definition、`RunStarted` snapshot、Tauri / WebSocket DTO を含む。旧 Workflow Run に復元可能な snapshot が無い場合も `Edit` へ戻さず、`WorkflowWaitingConfiguration` に置く。

### 9.5 Local atomic event transaction

launch / Session / Goal / workflow / queueを跨ぐ「同じlocal atomic batch」は説明上の比喩ではなく、Rust-owned `LocalEventTransactionStore`の1 transactionを意味する。新しいexecution-affecting eventを独立JSON logへ順番にappendしてatomic扱いしてはならない。

```rust
pub struct EventStreamKey {
    pub kind: String,
    pub id: String,
}

pub struct TypedEventEnvelope {
    pub event_type: String,
    pub schema_version: u32,
    pub payload: JsonPayload,
}

pub struct AtomicStreamAppend {
    pub stream: EventStreamKey,
    pub expected_head_seq: u64,
    pub events: Vec<TypedEventEnvelope>,
}

pub struct LocalAtomicBatchCommitted {
    pub batch_id: String,
    pub idempotency_key: String,
    pub global_commit_seq: u64,
    pub participants: Vec<AtomicStreamAppend>,
    pub committed_at: String,
}
```

- `commit_batch`は全participantのheadをCASし、per-stream seqとglobal commit seqを割当て、typed event payload、batch id、idempotency key、head更新を単一のdurable transactionでcommitする。SQLite WAL等の実transactionを使い、participant logへの逐次append＋補償で代用しない。
- commit前のbatchはどのquery/projector/watchにも見せない。crashがcommit前なら0件、commit後なら全participantが見える。`batch_id/idempotency_key`の再実行は同じ結果を返し、異なるpayloadならconflictにする。別のPrepared/Committed二相状態を外へ露出しない。
- per-stream event log/read model/cacheはcommitted transactionから再構築するprojection/indexである。legacy JSON eventはF7 migrationで順序を保ってstoreへidempotent importし、移行後に新eventを旧logへdual-writeしない。
- provider I/O前のintent batchはcommit成功後だけ送信可能。provider ack後のcanonical batchがcommitできなければ、旧stateへ戻ったふりをせず外部observation付きreconciliationへ進む。
- `get_session`、`get_agent_launch`、workflow queryと各watchはglobal commit barrierでsnapshot/replay cursorを取得し、同じbatchの一部だけを描画しない。surface固有seqはcommitted batchのper-stream seqから導出する。

### 10. Durable event（AgentSessionEvent）の進化

**V-D11**: 進化規約（V-P4 の具体化）:

1. 変更は additive-only。既存 variant のフィールド追加は `#[serde(default)]` 必須。
2. 新規 variant 追加時、旧バージョンの Releash が読む可能性は考慮しない（前方互換は不要）が、新バージョンは全ての旧イベントを読めること（後方互換必須）。
3. event log ファイルに `schema_version` を持たせ、読み込み時に lazy migration（旧 `completed: bool` → `TodoStatus` 等は読み込み写像で吸収し、書き戻しはしない）。
4. 未知イベント・未知フィールドは読み飛ばさず raw のまま保持し、projector は無視、書き戻しで保全する。

追加・変更する durable event:

| 変更 | 内容 | 解消 |
|---|---|---|
| `ToolCallStatusChanged { turn_id, tool_use_id, status, exit_code?, at }` 追加 | ToolCall 状態遷移の記録 | RG-4/RG-8/SD-5 |
| `NoticeRecorded { turn_id?, message_id, notice }` 追加 | `SystemNotificationRecorded` を後継（旧型は読み込み継続） | CX-7/RG-6/CL-5 |
| `TurnCompleted` 系の outcome 拡張 | stop_reason / stats / 構造化 error | CL-3/4/RG-3/9/RT-5 |
| `TurnTokenUsage` → V-D8 型 | cache / cost | RG-9 |
| `PermissionResolved` に `resolved_by` / `effective` 追加 | 実効性の記録 | CL-1 |
| `PermissionResponseRequested / Rejected / ProviderPermissionResponseObserved / PermissionResponseReconciliationRequired / Reconciled` 追加 | response id、redacted answers、明示reject後のPending復帰、request cancel/tool start、ack不明と解決attemptをwrite-ahead回復。secret plaintextは保存しない | CL-1/CX-1 |
| `TodoListSnapshotRecorded` の item 拡張 | status / priority | RG-5 |
| `ImageRecorded` / `ImageRefRecorded` の配線 | tool 出力 image | CL-6/RG-7 |
| `ConfigurationUpdateRequested / Rejected` 追加 | `update_id`、base / target revision、discriminated patch、activation timing を write-ahead 記録 | #1397/#1445〜#1448 |
| `LocalAtomicBatchCommitted` transaction envelope 追加 | multi-stream head CAS、per-stream/global seq、typed participants、idempotencyを単一transactionで確定しhalf-commitを禁止 | #1445/#1446/#1450 |
| `SessionConfigurationSelected / Activated` 追加 | selected / effective revision と model を含む小さな設定 snapshot を別々に確定。各 event append が canonical commit point | #1397/#1445〜#1448 |
| `BackendSessionRecoveryStarted / SessionConfigurationReactivated / SessionGoalReactivated / BackendSessionRecoveryCompleted` 追加 | resume metadata clearとbarrier開始、observation相関付きconfiguration/Goal復旧、両aggregateの最終atomic完了を同じrecovery idで確定 | #1397/#1407/#1449 |
| `ProviderConfigurationStateObserved / ConfigurationObservationAccepted / ConfigurationReconciliationRequired / Reconciled` 追加 | observation append時にblockし、同じobservation idをoutcomeがconsume。複合provider stateと解決をdurable化 | #1397/#1445〜#1448 |
| `TurnStartRequested / TurnStartReconciliationRequired / Reconciled` 追加 | effective/activation-targetを分けたintent、correlation、early-stream境界、provider観測とqueue terminalまで含むaction別atomic終端を回復 | #1397/#1450 |
| `TurnInterruptRequested / QueuePaused / QueueResumed` 追加 | Stop intentとpauseをpre-I/O atomic commitし、CAS付き明示resumeまで自動drainを禁止 | #1404/#1450 |
| `QueueItemEnqueued / QueueItemCancelled / QueueExecutionPrepared / QueueExecutionRequested / QueueItemStarted / QueueItemFailed / QueueItemResolutionRequired / QueueItemRebased / QueueItemRequeued` 追加 | item revision、message marker、immutable semantic snapshot/hash、challenge guard、resolution/rebase/CAS retry、execution/turn相関をappend-onlyに確定 | #1404/#1450 |
| `ReconciliationResolutionRequested` 追加 | resolution attempt/CAS/action/targetをprovider I/O前に記録し、configuration/Goal/launch/turn-start/permission解決を冪等回復 | #1397/#1445〜#1449 |
| `AgentLaunchDraftPrepared / PreparationExpired / PreparationCancelled / AttemptStarted / StageAdvanced / LaunchReconciliationRequired / LaunchProtocolIncompatible / Reconciled / Completed / Failed / Cancelled` 追加 | reservation、create correlation、provider/local ref、initial Goal handoff、観測、部分protocol identity、recoveryと全terminalをattempt streamへ保存 | #1445 |
| `SessionCreated` 追加（Session stream） | session id、originating launch attempt、provider/session ref、protocol identityを持ち、initial configuration seedとのmulti-stream batchをSession公開のcommit pointにする | #1445 |
| `LaunchInitialGoalRejected / InitialGoalResolutionRequested / Completed` 追加 | launch側rejectを再投影し、RetryGoal/ContinueWithoutGoal/CancelSessionをCAS＋write-aheadで排他して各actionを必ず終端 | #1445/#1449 |
| `WorkflowNodeBypassPrepared / WorkflowNodeExecutionRequested / WorkflowNodeAgentBound / WorkflowNodeAgentLaunchFailed / WorkflowNodeAgentLaunchCancelled` 追加 | challenge待機、stable attempt、workflow/launch origin、成功/失敗/取消terminalをlaunch terminalとのmulti-stream batchで相関 | #1450 |
| `BypassChallengeIssued / Consumed / Expired / Cancelled` 追加 | execution/reconciliation固有guard、期限、one-time consume、managed-policy再検査とreload可能なchallenge stateを監査 | #1446/#1448 |
| `GoalTransitionRequested / GoalTransitionRejected` 追加 | `transition_id`、goal id / base revision、操作を Goal 専用 write-ahead protocol で記録。成功終端はcanonical Goal eventだけとする | #1449 |
| `ProviderGoalCommandEvidenceObserved / GoalPrecommitControlConflictObserved` 追加 | Claude StartsTurnのcommand UUID＋completed lifecycle＋objective一致Goal stateをacceptance evidenceにし、commit前control requestのfail-closed/reconciliationを監査 | #1449/#1416 |
| `GoalSet / GoalTransitioned / GoalCleared` 追加 | goal revision、source、reason、evidence ref、`ProviderGoalSnapshot`をcanonicalに記録。Claude set/editではGoal event＋TurnStartedをatomic batch append | #1449 |
| `ProviderGoalStateObserved / GoalObservationAccepted / GoalReconciliationRequired / Reconciled` 追加 | provider ref＋Matched/Unmatched/Ambiguousを保存し、observation append時block、同じidをoutcomeがconsume | #1449 |
| `BackendProtocolIdentified / ProtocolIncompatible` 追加 | 実行 binary と compiled schema / flags / capabilities の一致を監査し、control-plane drift を fail-closed 化 | #1445/#1447〜#1449 |
| `TurnStarted` に resolved effective configuration / `EffectiveModeSnapshot` / Goal ref / protocol identity 追加 | provider/model/mode/effort、当時のpermission/effects/residual protections/context、Goalを不変監査可能にする | #1450 |

`PermissionResolvedBy::Auto` は provider classifier / reviewer の approved / denied を表し、取得できる decision reason / review item ref を同じ permission 履歴へ保存する。inProgress / timedOut / aborted や manual fallback を resolution として合成しない。単に `AgentMode::Auto` だったという理由だけで自動許可を合成しない。

### 11. Read model / GetSessionResponse（完全復元）

read model は「UI が描画する全て」を保持する（lifecycle I 群・presentation P1 の前提）。`get_session` は runtime 可視状態の完全スナップショットとsession `seq`を返す: messages(parts) / turn_phase / pending・Responding・reconciliation中のpermission / `QueueProjection`（item revision、active＋bounded recent terminal＋paused＋seq）/ `TurnStartState` / latest TokenUsage / last TurnResult / notices / `AgentSessionConfigurationState` / `SessionGoalProjection` / `SessionControlOperationLease` / `ProviderPermissionState` / `AgentProtocolState` / capabilities / pending observation・reconciliation・resolution attempt / available actions・mode effects。古いqueue terminal履歴は`get_queue_history(session_id, cursor, limit) -> QueueHistoryPage`でpage取得する。Bypass waiting stateはfull challenge viewを埋め、独立query `get_bypass_challenge(challenge_id) -> BypassChallengeProjection`もIssued/Consumed/Expired/Cancelledを返す。nonceは認可済みclientへIssued中だけ返し、terminal projectionではredactする。

Session確立前のNew AgentはSession read modelに押し込まない。S9aは`get_agent_launch_preflight(workspace_id, provider_id, context)`から`Checking | Compatible(AgentBackendCapabilities) | ProtocolIncompatible(partial identity)`を取得する。`prepare_agent_launch`はattempt id/hashをreserveし、Bypassなら`AgentLaunchDraftPrepared + BypassChallengeIssued`を同じlocal batch、non-BypassならPrepared単独でappendする。Queue/Workflow Bypassも各Prepared＋ChallengeIssuedをatomic appendする。確認後の`start_agent_launch`がdraft hash、preflight context、policy/gateを再検証し、`BypassChallengeConsumed + AgentLaunchAttemptStarted`をlocal atomic batchでappendしてからprovider I/Oする。draft変更・期限切れはreservation/challengeを失効させ、再prepareを要求する。

reserved attempt idで分離したdurable launch event streamから`get_agent_launch(attempt_id) -> AgentLaunchProjection`を再構築する。projectionは`Prepared / Started / PreparationExpired / PreparationCancelled`を含み、prepare後start前のreloadも復元する。`AgentLaunchChanged`はmutable fieldの取りこぼしを避けるため小さなfull projectionを運ぶ。購読は`watch_agent_launch(attempt_id, after_seq)`で、serverがsnapshot/replayとsubscription登録を同じbarrier内で行う。retention内なら`after_seq`より後をreplayし、古すぎるcursorは最新snapshotを返すため、get→subscribe間のraceを作らない。`seq`はattempt単位で単調増加し、reload/reconnectまたはgap/逆行検出時はsnapshotを再取得する。Completed後もretention期間内はSession idへの相関を保持し、launch失敗・reconciliation・pre-session ProtocolIncompatibleを復元できる。

Goal履歴はcurrent projectionへfull-retentionしない。`get_goal_history(session_id, cursor, limit) -> GoalHistoryPage`と`get_goal_revision(session_id, goal_id, revision)`をevent logからpage/id lookupし、transition kind/result/time、before/after objective/status、source/evidence、launch相関を返す。`TurnStarted`のgoal id/revisionはこのrevision lookupで後から解決でき、Goal clear/replace後も当時のobjectiveを監査できる。

event log の `SessionConfigurationSelected / Activated` と Goal canonical event を唯一の durable commit point とする。`SessionMeta` の configuration / Goal snapshot は高速 projection/cache であり、event から再構築できる。cache 更新失敗は canonical provider drift ではなく `PersistFailure` と再投影で回復する。Workflow は `RunStarted` と step resolution event を commit point、run metadata を projection とする。queue item と Workflow step execution は effective configuration snapshot と `goal_id + goal_revision` を保持し、turn read model は provider/model/mode/effective effort/Goal/protocol identity を展開表示できる。

一般Sessionの購読は`watch_session(session_id, after_seq)`を唯一の入口とし、serverがsnapshot/replay決定とsubscription登録を同じbarrier内で行う。`get_session`後にそのseqでwatchした場合も、cursor以後を必ずreplayして「最後のeventだけ逃し次eventが無い」窓を作らない。cursorがretention外ならfull snapshotを返す。snapshot/deltaのsession単位seqは単調増加し、frontendは欠落・逆行時にsnapshotを再取得する（FE-3 / presentation P1）。

### 12. Wire 層の型付け（写像の入口）

- **V-D12a（合意済み）**: Codex は `codex-app-server-protocol` / `codex-protocol` 公式クレートをタグ固定 git 依存で導入し、手書き `serde_json::Value` 解釈を全廃する（ST-1）。
- **V-D12b**: Claude は Claude Agent SDK の型定義（`sdk.d.ts` の StdoutMessage union）を正とした typed model（serde struct/enum）を `infrastructure/agent_session/claude/wire.rs` に定義する（ST-2）。SDK バージョンを wire.rs に明記し、更新時に差分レビューする。
- mode / Goal / reasoning effort の capability、更新要求、ack / error、provider permission snapshot も typed request / response として定義し、文字列比較や frontend fallback に戻さない。Claude `/goal` は公開 typed RPC と偽装せず、side effect を宣言した typed `ProviderCliCommand` adapter とする。
- spawn した executable の `BackendProtocolIdentity` を initialize 時に検証する。compiled schema と互換でない binary、experimental flag、initialize capability の組合せでは session を開始しない。
- content-plane の typed decode 失敗・未対応 variant は V-P1 に従い `Notice(UnsupportedMessage)` ＋構造化ログへ着地させる。control-plane は `ProtocolIncompatible` または対象 aggregate の reconciliation へ着地させ、新規 turn を block する。両者の件数を parity テスト（ST-7）で別々に検証する。

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
