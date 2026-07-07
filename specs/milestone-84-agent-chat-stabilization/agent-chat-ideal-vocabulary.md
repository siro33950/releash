# Agent チャット正規化語彙・データ構造の理想形

作成日: 2026-07-07

milestone 84「Agentチャット安定化」のドキュメント群:

- [agent-chat-instability-audit.md](agent-chat-instability-audit.md) — 問題点インベントリ（全 66 件、要求リスト）
- **agent-chat-ideal-vocabulary.md（本書）** — 正規化語彙・データ構造の理想形
- [agent-chat-ideal-lifecycle.md](agent-chat-ideal-lifecycle.md) — ライフサイクルの理想形（不変条件）
- [agent-chat-ideal-presentation.md](agent-chat-ideal-presentation.md) — UI 表示の理想形

本書は「Claude / Codex から届く事象を、何という語彙に正規化するか」の正本を定義する。監査で確定した dropped / divergent 問題群の解消先であり、ライフサイクル・表示の 2 文書はこの語彙を前提とする。問題 ID（CL-x 等）は監査ドキュメントを参照。

## 設計原則

- **V-P1 (no-silent-drop)**: wire 層は届いたメッセージを無言破棄してはならない。変換先の無い既知メッセージ・未知メッセージは `Notice(kind=UnsupportedMessage)` と構造化ログに必ず着地させる。「捨てる」は明示的な設計判断としてのみ許され、本書に記録する。
- **V-P2 (parity)**: 同一概念は backend に依らず同一の語彙要素へ写像する。backend 固有の概念（Codex の item 種別等）は新しい part 種を増やすのではなく、既存語彙の kind / フィールドへ写像する。
- **V-P3 (durable 表現可能性)**: UI に表示されるべき全情報は、この語彙（part / turn outcome / usage / notice）で表現でき、durable event として記録できなければならない。transient にしか存在しない表示情報を作らない。
- **V-P4 (additive 進化)**: 永続化される語彙（durable event / read model）の変更は additive-only とし、既存セッションの読み込み互換を壊さない。
- **V-P5 (full-retention 回避)**: 語彙拡張はサマリ・参照（`ToolOutputRef`）・スナップショットで表現し、wire の生ペイロード全量を恒久保存しない。

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
    OversizeDropped,         // 既存の 8MB 超過破棄の可視化を統合
    PersistFailure,          // lifecycle I8: 永続化失敗の可視化
    Diagnostic,              // stall 診断等
}
```

- Notice は transcript 上の part として durable 化する（表示先の振り分け — inline / banner / badge — は presentation 文書で定義）。
- **判断**: session-scoped な別ストリームではなく part として持つ。理由: read model 一本で live / reload 等価（P 原則）を保て、発生時点の文脈（どの turn で何の直後か）が残る。rate limit のような「最新値だけ意味がある」ものは read model 側で latest を導出する。

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

// 回答は question.id をキーに、複数回答を配列で保持する
// （Codex wire の { answers: { <id>: { answers: [String] } } } と可逆）
pub struct PermissionAnswers(pub BTreeMap<String, Vec<String>>);

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
    Resolved {
        decision: PermissionDecision,        // Allowed / Denied / Cancelled（配線する）
        answers: Option<PermissionAnswers>,
        resolved_by: PermissionResolvedBy,   // User / Rule / Auto / Backend / System
        effective: bool,                     // CL-1: backend に実際に効いた決定か
    },
}
```

- `effective: false` は「ユーザーは押したが backend はもう待っていなかった」を表し、履歴上の誤記録（CL-1）を防ぐ。失効（CLI 取り下げ・interrupt）の遷移規則は lifecycle 文書 I7。
- `decision_reason` / `description` は現行フィールドを維持し、表示まで配線する（FE-7 は presentation 側）。
- **id の合成**: Claude の AskUserQuestion は wire 上 question id を持たないため、変換層で安定 id（出現順の `q0`, `q1`…）を合成し、応答時に backend ごとの期待形式（Codex: id キーの `{answers: {<id>: {answers: [..]}}}`、Claude: 質問順ベース）へ逆写像する。写像は各 backend の permission モジュールが所有する。
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

### 8. AgentRuntimeEvent（維持・最小拡張）

**V-D9**: enum の種類は現行を維持する（合意済み: ACP へ載せ替えない）。Notice / Todo / ToolCall 更新はすべて `PartsMerged` 経由で流し、イベント種を増やさない。変更点のみ:

- `TurnCompleted(TurnResult)` — V-D7 の拡張型に
- `TokenUsageUpdated(TokenUsage)` — V-D8 の拡張型に
- `BackendSessionCleared` — dead code を解消し配線（lifecycle I9 / SD-1）
- `PermissionRequested` / `PermissionModeChanged` / `SlashCommandsUpdated` / `KeepAlive` / `Fatal` — 現行維持

### 9. PermissionMode / plan mode

**V-D10**: `PermissionMode { Ask, Edit, Full }` ＋ `plan_mode: bool` の現行構造を維持し、**wire との写像を全域定義**する。Claude wire の `"plan"` は `(Ask, plan_mode=true)` へ写像し、CLI 主導の plan 遷移を UI に同期する（CL-7）。

- **代替案**: `Plan` を含む 4 値 enum。UI トグル（mode 選択と plan の直交）と workflow の permission 3 値既定との整合を壊すため不採用。

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
| `TodoListSnapshotRecorded` の item 拡張 | status / priority | RG-5 |
| `ImageRecorded` / `ImageRefRecorded` の配線 | tool 出力 image | CL-6/RG-7 |

### 11. Read model / GetSessionResponse（完全復元）

read model は「UI が描画する全て」を保持する（lifecycle I 群・presentation P1 の前提）。`get_session` は runtime 可視状態の完全スナップショットを返す: messages(parts) / turn_phase / pending_permission_request（#1379 で対応済み）/ pending queue / latest TokenUsage / last TurnResult / session-level notices（latest rate limit 等の導出値）。

また、transient delta と snapshot にはセッション単位の単調増加 `seq` を含める。frontend は seq の欠落・逆行を検出したら snapshot を再取得して自己修復する（FE-3 / presentation P1 の契約。順序・合成の正しさは backend が保証し、frontend は検出と再取得のみを行う）。

### 12. Wire 層の型付け（写像の入口）

- **V-D12a（合意済み）**: Codex は `codex-app-server-protocol` / `codex-protocol` 公式クレートをタグ固定 git 依存で導入し、手書き `serde_json::Value` 解釈を全廃する（ST-1）。
- **V-D12b**: Claude は Claude Agent SDK の型定義（`sdk.d.ts` の StdoutMessage union）を正とした typed model（serde struct/enum）を `infrastructure/agent_session/claude/wire.rs` に定義する（ST-2）。SDK バージョンを wire.rs に明記し、更新時に差分レビューする。
- 両者とも、typed decode に失敗した行・未対応 variant は V-P1 に従い `Notice(UnsupportedMessage)` ＋構造化ログへ着地させる。この着地数が 0 であることを parity テスト（ST-7）の前提チェックにする。

## トレーサビリティ（本書が解消する問題）

| 問題 ID | 設計要素 |
|---|---|
| CL-3, CL-4, RG-3 | V-D7 TurnStopReason / TurnStats |
| CL-5 | V-D4 NoticeKind::McpServerStatus |
| CL-6, RG-7 | V-D2 ToolOutputBlock::Image ＋ §10 ImageRecorded 配線 |
| CL-7 | V-D10 wire 写像の全域化 |
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

**語彙変更が不要な独立修正**（本ドキュメント群の設計を待たずに着手可能）: CX-4（tokenUsage フィールド名）、CX-9（initialize commands の dead code — V-D12a に内包可）、OB-7（画像のみ送信時の空 text block）。CX-1 の wire 形式修正は V-D6 の型を前提に行う。

## 確定事項（2026-07-07 レビューで確定）

1. **ToolCall 統合（V-D2）**: 単一 `ToolCall` part への統合を**採用**。durable event は既存種を残し projector で合成する移行方式。
2. **PermissionMode の表現（V-D10）**: 現行 3 値 enum ＋ plan_mode: bool の維持＋wire 写像の全域化を**採用**（4 値化は不採用）。
3. **Notice の持ち方（V-D4）**: transcript の part として記録する方式を**採用**（session-level 別ストリーム案は不採用）。RateLimit 等の最新値は read model 側で導出する。
