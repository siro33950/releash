# Design — issues-1247

対象 Issue: #1247
要求の正本: `requirements.md`（本ディレクトリ）
振る舞いの正本: `behavior.md`（本ディレクトリ）

本書は、Agent session の保存正典を **durable event 列（`AgentSessionEvent`）** へ収束させ、message page・session status・workflow turn-complete input を **projection（read model 反映）** で導出する構造の実装設計を定義する。`behavior.md` が `design.md` に委ねた durable event の variant 名・projector の内部構造・モジュール配置・session status の網羅的対応表・finalization の終端 event をここで確定する。

本変更は内部アーキテクチャの再定義であり、外部から観測可能な振る舞いは不変（R6）である。

---

## 概要

現状の Agent session の状態は複数の場所に分散している（`requirements.md` Background、本リポジトリ調査による事実）:

- **永続層**: `usecase/agent_session/session/` の `ChatSession` / `ChatMessage` / `MessagePart` を、`adaptor/gateway/agent_session/session_storage/` が `meta.json` + `messages/{seq}.json` + `index.json`（#1213 導入）で保存する。
- **runtime memory**: `infrastructure/agent_session/runtime/bridge_common.rs` の `AgentProcess` が `streaming_parts: Vec<MessagePart>` / `state: BridgeState` / `turn_phase: TurnPhase` を turn 中に保持する。
- **streaming**: cumulative snapshot 方式。`accumulate_sdk_message()` が SDK メッセージを `MessagePart` vector に push し、`run_turn_complete_transition_locked()` で consolidate して永続化する。
- **workflow handoff**: turn 完了判定を `run_turn_complete_transition_locked()` が返す `was_streaming`（= `proc.state == BridgeState::Streaming`）で gating し、`spawn_workflow_turn_complete_notification()` 経由で `WorkflowRuntimeUsecase::complete_turn()` を呼ぶ。

本設計では「session の事実」を **append-only な `AgentSessionEvent` 列**として表現し、read model（message page・session status・workflow turn-complete input）を event 列の **projection** として導出する。

- **durable event**: replay 可能で session の事実を構成する最小語彙。block 確定・tool 呼び出しの開始/成功/失敗・retry・permission 解決・turn 終端を表す。
- **live-only delta**: 表示中のみ意味を持ち replay されない逐次差分（text / tool-input / reasoning の delta）。event 列には含めない。

初期実装は **互換 projector**（A2）であり、event 列を独立した永続ファイルとして持たず、projection 結果を既存の `ChatSession` JSON 構造（`meta.json` / `messages/{seq}.json` / `index.json`）へ書き込む。event 列の独立永続化は将来拡張余地として扱う（後述「リスクと代替案」）。

cumulative snapshot 前提（#1214 未完了）・`bridge_common.rs` 未分割（#1217 未完了）の現状コード上で成立させる。

---

## 変更対象

### Rust（バックエンド）

- `src-tauri/src/usecase/agent_session/event_log/`（**新規モジュール**）
  - `AgentSessionEvent`（durable event 語彙）・`projector`（event 列 → read model）・`finalization`（異常終端ルール）を定義する。read model 型（`ChatSession` / `ChatMessage` / `MessagePart` / `SessionState`）と同じ usecase 層に置き、projector がそれらを直接構築できるようにする。live-only delta は現段階では `bridge_common.rs` の legacy accumulator 内の扱いに閉じ、`AgentSessionEvent` へは append しない。
- `src-tauri/src/usecase/agent_session/session/mod.rs`
  - projector の出力先である `ChatMessage` / `MessagePart` / `SessionState` は変更しない（serde 表現を維持、R6）。projector が参照するのみ。
- `src-tauri/src/usecase/agent_session/status.rs`
  - session status の read model（`TurnPhase` / `SessionStatus`）を projector の出力として接続する。型定義自体は維持。
- `src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs`
  - `AgentProcess` に per-turn の `TurnEventLog`（runtime 保持の event buffer）を追加する。
  - `accumulate_sdk_message()` / `run_turn_complete_transition_locked()` / `run_bridge_error_transition_locked()` を、runtime memory を直接更新する経路から **durable event を append → projection で read model に反映する経路**へ置き換える（R7）。
  - timeout（`finalize_turn_as_timeout_locked()`）/ interrupt（`interrupt_active_agent_turn()` 応答）も finalization event を append する経路に接続する。
  - **`bridge_common.rs` のモジュール分割は行わない（#1217 が担当 / Non-goal）。** event 列駆動への差し替えは同ファイル内に閉じた `event_log` 利用として実装する。

### フロントエンド

- 変更なし。本変更は内部の保存正典の再定義に限定し、新規 Tauri コマンド・UI 要素は追加しない（A4）。projection 結果の `ChatSession` / `SessionStatus` の serde 表現が不変であるため、frontend からは何も変わらない（R6）。

---

## アーキテクチャと責務分割

`.claude/rules/rust-first-logic.md` および `docs/architecture/` の依存方向（`infrastructure → adaptor/gateway → domain ← usecase ← adaptor/controller`）に従う。event 語彙・projection・finalization の判断はすべて Rust に置く。

```text
                event append              projection
SDK / runtime ───────────────▶ AgentSessionEvent 列 ───────────────▶ read model
 (bridge_common)               (TurnEventLog: runtime 保持)          ├─ message page (ChatMessage/MessagePart)
                                                                     ├─ session status (SessionState + TurnPhase)
                                                                     └─ workflow turn-complete input
                                                                            │
                                                            互換 projector で永続化
                                                                            ▼
                                          meta.json / messages/{seq}.json / index.json (#1213)
```

| レイヤー | 責務 |
| --- | --- |
| `usecase/agent_session/event_log` | `AgentSessionEvent` 語彙、durable / live-only の境界、projector（event 列 → read model）、finalization ルール。純粋ロジックでテスト可能 |
| `usecase/agent_session/session` | projector の出力型（`ChatSession` / `ChatMessage` / `MessagePart` / `SessionState`）。serde 表現を維持 |
| `usecase/agent_session/status` | projector が導出する session status（`SessionState` + `TurnPhase`）の read model 型 |
| `adaptor/gateway/agent_session/session_storage` | 互換 projector の出力を既存 split layout（#1213）へ書き込む。本変更で構造は変えない |
| `infrastructure/.../bridge_common` | `AgentProcess` の `TurnEventLog` 保持、3 経路での event append、projection 結果の read model 反映と永続化呼び出し |

### 正典の再定義

#1213 は「メタ = `meta.json`、message body = `messages/{seq}.json`、runtime 状態（`streaming_parts` / `turn_phase`）は非正典」と定義した。本変更はこれを次のように再定義する:

- **session の事実の正典 = `AgentSessionEvent` 列**。message page・session status・workflow turn-complete input はすべてこの event 列の projection。
- **永続層（`meta.json` / `messages/{seq}.json` / `index.json`）= event 列の projection 結果の保存先**（互換 projector の出力）。#1213 の構造をそのまま流用し、フォーマット破壊的変更はしない（R6 / 制約）。
- **`streaming_parts` 等の runtime memory = projection の途中状態の保持器**であり、唯一の正典としては扱わない。session status（streaming 中・permission 待ち）は event 列から projection で導出する（R3）。

> **接続点（R5）**: 本変更は #1213 が確立した split layout を projection の出力先として利用する。streaming は cumulative snapshot 前提のまま（#1214 が seq delta protocol を担当）であり、本変更が導入する durable / live-only の境界は #1214 の概念的下地となる。`bridge_common.rs` の event 駆動化は同ファイル内に閉じ、`event_log` モジュールという明確な seam を残すことで、#1217 の runtime / stream / persist / recovery 分割が後から接続しやすい形にする。この接続点はコード上のコメントとして `event_log/mod.rs` 冒頭にも記す。

---

## データモデルまたは型

### durable event 語彙（`event_log/mod.rs`）

OpenCode の durable event を参考にしつつ、Releash の既存 `MessagePart`（`Thinking` / `Text` / `ToolUse` / `ToolResult` / `Error` / `Permission` / `TaskStatus` / `TodoListSnapshot` / `SystemNotification` / `Image` / `ImageRef`）と整合する Releash 固有の語彙として定義する（A3）。

```rust
/// session の事実を構成する最小の durable event。replay 可能。
/// turn_id は 1 回の user prompt 投入で開始する turn を識別する。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentSessionEvent {
    /// user prompt 投入で turn を開始する。
    TurnStarted {
        turn_id: TurnId,
        message_id: String,
        prompt: PromptInput,      // text + mentions + 添付参照 + human message parts
        at: f64,
    },
    /// 確定した assistant text block（live-only の text_delta を集約した最終形）。
    TextRecorded {
        turn_id: TurnId,
        message_id: String,
        content: String,
        parent_tool_use_id: Option<String>,
    },
    /// 確定した reasoning block（live-only の reasoning_delta を集約した最終形）。
    ReasoningRecorded {
        turn_id: TurnId,
        message_id: String,
        content: String,
        parent_tool_use_id: Option<String>,
    },
    /// tool 呼び出しの開始（input 確定後）。
    ToolCallStarted {
        turn_id: TurnId,
        tool_use_id: String,
        tool: String,
        input: serde_json::Value,
        parent_tool_use_id: Option<String>,
    },
    /// tool 呼び出しの成功。
    ToolCallSucceeded {
        turn_id: TurnId,
        tool_use_id: String,
        content: String,
    },
    /// tool 呼び出しの失敗。
    ToolCallFailed {
        turn_id: TurnId,
        tool_use_id: String,
        content: String,
    },
    /// tool 呼び出しの retry（再試行の記録）。
    ToolCallRetried {
        turn_id: TurnId,
        tool_use_id: String,
        attempt: u32,
    },
    /// permission 要求。
    PermissionRequested {
        turn_id: TurnId,
        tool_use_id: Option<String>,
        request: serde_json::Value,
    },
    /// permission 解決（許可 / 拒否 / 取消）。
    PermissionResolved {
        turn_id: TurnId,
        tool_use_id: Option<String>,
        decision: PermissionDecision, // Allowed | Denied | Cancelled
        answers: Option<serde_json::Value>,
    },
    /// background task の状態変化（TaskStatus part 由来）。
    TaskStatusChanged {
        turn_id: TurnId,
        message_id: String,
        task_tool_use_id: String,
        status: String,
        description: Option<String>,
        summary: Option<String>,
    },
    /// todo list snapshot。
    TodoListSnapshotRecorded {
        turn_id: TurnId,
        message_id: String,
        items: Vec<TodoListItem>,
    },
    /// compaction 等の system notification。
    SystemNotificationRecorded {
        turn_id: TurnId,
        message_id: String,
        notification_type: SystemNotificationType,
        status: String,
        label: String,
        detail: Option<String>,
        hook_id: Option<String>,
    },
    /// turn の正常終端（exit_code == 0）。
    TurnCompleted {
        turn_id: TurnId,
        exit_code: i64,
        token_usage: Option<TurnTokenUsage>,
    },
    /// turn の異常終端（finalization）。abort / timeout / bridge crash を一意に表す。
    TurnInterrupted {
        turn_id: TurnId,
        reason: InterruptReason,  // Abort | Timeout | BridgeCrash
        error: Option<String>,
    },
    /// session のクローズ。
    SessionClosed { at: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptReason { Abort, Timeout, BridgeCrash }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision { Allowed, Denied, Cancelled }
```

`TurnId` は turn の単調増加識別子（既存 `AgentProcess::turn_seq` に対応づけ可能）。`PromptInput` / `TurnTokenUsage` / `TodoListItem` / `SystemNotificationType` は既存型を再利用または薄くラップする。

### live-only delta（legacy accumulator 内の扱い）

replay 不要で表示中のみ意味を持つ逐次差分は、durable event とは分離する。現段階では `LiveDelta` の公開 API は持たず、`text_delta` / `thinking_delta` / tool input partial は `bridge_common.rs` の既存 `accumulate_sdk_message()` 系が legacy live buffer（`streaming_parts`）へ反映する。**`AgentSessionEvent` 列には決して含めない**。

terminal flush で確定した text / reasoning / error block だけを `PartEventMode::FinalLiveBlocks` として `TextRecorded` / `ReasoningRecorded` / `ErrorRecorded` へ変換する。将来 #1214 の seq-delta protocol で live delta API が必要になった場合は、この境界の外側に追加する。

### durable / live-only の対応表（R2 の明文化）

| SDK / 内部イベント | 分類 | event / delta |
| --- | --- | --- |
| `stream_event` の `text_delta` | live-only | legacy live buffer（`AgentSessionEvent` へ append しない） |
| `stream_event` の `thinking_delta` | live-only | legacy live buffer（`AgentSessionEvent` へ append しない） |
| tool input の partial（逐次） | live-only | legacy live buffer（`AgentSessionEvent` へ append しない） |
| text block の確定（block 終端 / flush 境界） | durable | `TextRecorded` |
| thinking block の確定 | durable | `ReasoningRecorded` |
| `assistant` の tool_use ブロック（input 確定） | durable | `ToolCallStarted` |
| 同一 turn / tool_use_id の再 tool_use ブロック | durable | `ToolCallRetried` + `ToolCallStarted` |
| `user` の tool_result（is_error=false） | durable | `ToolCallSucceeded` |
| `user` の tool_result（is_error=true） | durable | `ToolCallFailed` |
| permission 要求 | durable | `PermissionRequested` |
| permission 応答 | durable | `PermissionResolved` |
| `turn_complete`（exit_code==0） | durable | `TurnCompleted` |
| `turn_complete`（interrupted）/ `error` / timeout | durable | `TurnInterrupted` |

live-only delta は「同じ block の最終形が durable event として必ず append される」ことを不変条件とする。よって event 列のみから read model は完全再構築でき、delta を適用しなくても欠落しない（behavior「live-only delta は read model の正典に含まれない」）。

### read model（projector の出力）

```rust
/// event 列から projection した read model。
pub struct SessionReadModel {
    pub messages: Vec<ChatMessage>,                 // message page の元（既存型）
    pub status: ProjectedStatus,                    // session status
    pub workflow_turn_complete: Option<WorkflowTurnCompleteInput>,
    pub tool_retries: Vec<ToolRetryProjection>,      // retry 履歴（UI 表示は変えない内部 read model）
}

pub struct ProjectedStatus {
    pub session_state: SessionState,                // Active/Idle/Done/Error/Closed/Archived
    pub turn_phase: TurnPhase,                      // Idle/Streaming/WaitingPermission（runtime 遷移状態）
}

pub struct ToolRetryProjection {
    pub turn_id: TurnId,
    pub tool_use_id: String,
    pub attempt: u32,
}

/// workflow への turn 完了通知の入力。runtime state ではなく event 列から導出する（B5）。
pub struct WorkflowTurnCompleteInput {
    pub turn_id: TurnId,
    pub exit_code: i64,
    pub final_text_parts: Vec<String>,
    pub token_usage: Option<TurnTokenUsage>,
    pub interrupted: bool,
}
```

---

## 処理フロー

### projector: event 列 → read model

`project(events: &[AgentSessionEvent]) -> SessionReadModel` は純粋関数として実装する。

1. **message page の構築**: event を順に fold する。
   - `TurnStarted` → user role の `ChatMessage` を生成。
   - `TextRecorded` / `ReasoningRecorded` → assistant message の `MessagePart::Text` / `Thinking` を追加。
   - `ToolCallStarted` → `MessagePart::ToolUse`。`ToolCallSucceeded` / `ToolCallFailed` → 対応する `tool_use_id` の `MessagePart::ToolResult`（`is_error` を設定）。
   - `ToolCallRetried` → UI の既存 `MessagePart` 形式は変えず、内部 read model の retry 履歴（`tool_retries`）へ反映する。
   - `PermissionRequested` / `PermissionResolved` → `MessagePart::Permission`（`status` を解決状態に更新）。
   - `TaskStatusChanged` / `TodoListSnapshotRecorded` / `SystemNotificationRecorded` → event の `turn_id` / `message_id` で明示された対象 message に対応 `MessagePart` を反映する。
   - 同一 `tool_use_id` / `message_id` への後続 event は **in-place 更新**（重複 part を作らない）。これにより「同一 event 列の再 projection で message が重複しない」（behavior reconnect Rule）を保証する。
2. **session status の導出**（後述の対応表）。
3. **workflow turn-complete input の導出**: `TurnStarted` を持つ turn に `TurnCompleted` または `TurnInterrupted` が現れた時点で `Some` を返す。`final_text_parts` はその turn の `TextRecorded` から、`interrupted` は `TurnInterrupted` の有無から導出する（runtime `BridgeState::Streaming` の直接参照を正典としない / B5）。

projection は event 列にのみ依存する純粋関数のため、**同一 event 列からは常に同一の read model が導出される**（behavior 決定性 Rule）。

### session status の projection（R3 / B3 の網羅的対応表）

session status = 永続側 `SessionState` ＋ runtime 由来の遷移状態 `TurnPhase` の二軸。event 列から両軸を導出する。

| 直近の event 状態 | `SessionState` | `TurnPhase`（遷移状態） |
| --- | --- | --- |
| `TurnStarted` 後、終端 event なし | Active | Streaming |
| `PermissionRequested` 後、対応 `PermissionResolved` なし | Active | WaitingPermission |
| `TurnCompleted`（exit_code==0） | Idle | Idle |
| `TurnInterrupted { reason: Abort }` | Idle | Idle |
| `TurnInterrupted { reason: Timeout \| BridgeCrash }` | Error | Idle |
| `SessionClosed` | Closed | Idle |
| event なし（新規） | Idle | Idle |

`Done` / `Archived` は session ライフサイクル操作（明示クローズ・アーカイブ）由来であり turn の event からは導出しない。これらは既存の `SessionState` 遷移経路を維持し、event 列の projection 結果に上書き合成する（projector は turn 由来の `SessionState` を提示し、ライフサイクル由来の `Done` / `Archived` は上位で優先）。

これにより「runtime memory を参照せず event 列のみから streaming 中・permission 待ちが判定できる」「runtime memory が唯一の保持者である状態は存在しない」（behavior runtime 遷移状態 Rule / R3）を満たす。

### finalization（R4 / abort・timeout・bridge crash）

異常終端時に未完了の turn / tool call / permission を partial で残さないため、終端ルーチン `finalize_turn(log, reason)` が **明示的な終端 event を append** する（projector に推論を委ねず、event 列を自己完結させる）:

1. 当該 turn の **未完了 tool call**（`ToolCallStarted` はあるが `ToolCallSucceeded` / `ToolCallFailed` がない `tool_use_id`）ごとに `ToolCallFailed { content: "<reason> により中断" }` を append。
2. 当該 turn の **未解決 permission**（`PermissionRequested` はあるが `PermissionResolved` がない）ごとに `PermissionResolved { decision: Cancelled }` を append。
3. 最後に `TurnInterrupted { reason }` を append。

この順序により、projection 後の read model に partial な turn / tool call / permission は残らず、turn は「未完了」ではなく「終端済み」として一意に判定できる（behavior finalization Rule）。完了判定が runtime state と session JSON のどちらを見るかで揺れない（failure mode 1・3 の解消）。

trigger との対応:

| trigger | append 経路 |
| --- | --- |
| abort（interrupt） | `interrupt_active_agent_turn()` 応答の `turn_complete(interrupted=true)` → `finalize_turn(log, Abort)` |
| timeout | `finalize_turn_as_timeout_locked()` → `finalize_turn(log, Timeout)` |
| bridge crash | `run_bridge_error_transition_locked()` → `finalize_turn(log, BridgeCrash)` |

### 3 経路の event 駆動への置き換え（R7）

`AgentProcess` に per-turn の `TurnEventLog`（runtime 保持の `Vec<AgentSessionEvent>`）を追加し、既存 3 経路を次のように差し替える。**`streaming_parts` は廃止せず、`project(turn_event_log)` の結果（＋未確定の text delta tail）を保持する live 表示バッファとして再定義する**。cumulative snapshot の emit はこのバッファから従来どおり行うため、streaming の見え方は不変（B2 / R6）。

- **`accumulate_sdk_message()`**: SDK メッセージ解析時、
  - `text_delta` / `thinking_delta` / tool input partial → legacy live バッファを更新（event は append しない）。
  - block 確定 / tool_use / tool_result / permission → 対応する `AgentSessionEvent` を `TurnEventLog` に append。
  - emit する cumulative snapshot は `project(turn_event_log)`（＋ live tail）から生成。従来の `append_to_parts` / in-place 更新は projector 側の fold ロジックへ移す。
- **`run_turn_complete_transition_locked()`**: flush 後に interrupt 指示なしなら exit_code に関わらず `TurnCompleted { exit_code }` を append し、abort / timeout / bridge crash の明示経路では finalization を呼ぶ。`final_parts` を `consolidate_parts_from_slice` ではなく `project(turn_event_log).messages` の最終 message から取得する。`was_streaming` 相当の判定は「当該 turn に `TurnStarted` があるか」で代替し、workflow 通知入力（`WorkflowTurnCompleteInput`）を projection から得る。
- **`run_bridge_error_transition_locked()`**: `sdk_error_part_from_message()` で `MessagePart::Error` を直接 push する代わりに、`finalize_turn(log, BridgeCrash)` を呼ぶ（未完了 tool/permission も同時に終端）。その後 projection 結果を永続化する。

永続化は従来どおり post-lock で `persist_streaming_parts()` 相当を呼ぶが、保存対象は `project(turn_event_log)` の message である。出力先は #1213 の split layout（`messages/{seq}.json` / `index.json` / `meta.json`）で不変（R6）。

> observable behavior の不変は、terminal projection 時点で `project(turn_event_log)` が現行 `consolidate_parts_from_slice(&streaming_parts)` と同一の `Vec<MessagePart>` を生成することで担保する（テスト方針の golden 比較で検証）。mid-turn の durable-only projection では未確定の streamed text / thinking は event に含めないため、`project(turn_event_log)` と `consolidate_parts_from_slice(&streaming_parts)` は一致しない。

### 復旧 / reconnect

- **reconnect**: read model を `project(events)` で再構築する。projector は in-place 更新で重複を作らないため、累積 parts の二重適用は発生しない（failure mode 2 / behavior reconnect Rule）。
- **crash / abort 後**: 終端 event が append 済みのため、再 projection で partial が残らない。互換 projector は projection 結果を `messages/{seq}.json` へ書くため、再起動後は永続化された projection（= 終端済み read model）を読む。決定的再構築は event 列に対する unit test で示す（A2 により event 列の独立永続化は将来拡張のため、crash 後の event 列そのものの復元は本変更の対象外。永続化されるのは projection 結果）。
- **terminal 前 hard-crash**: streamed text / thinking は terminal の final live block で durable event 化する設計のため、terminal event 前に process / app が hard-crash した場合、mid-turn store に未確定 tail は含まれず、その text / thinking は復旧後の read model から失われ得る。これは mid-turn persist を durable event のみに限定して hot path と永続形式を保つための許容トレードオフであり、terminal 済み read model の reconnect 決定性とは矛盾しない。

---

## エラー処理

- **projection の堅牢性**: 不整合な event 列（`ToolCallSucceeded` に対応する `ToolCallStarted` がない等）でも projector は panic せず、orphan event をスキップして warn ログに残す（#1213 の session 単位隔離方針を踏襲）。
- **finalization の冪等性**: `finalize_turn()` は既に終端済みの tool call / permission を二重終端しない（未完了分のみ append）。`TurnInterrupted` が既にある turn への再 finalize は no-op。
- **append 失敗の不在**: `TurnEventLog` は runtime memory（`Vec`）への push であり I/O を伴わないため append 自体は失敗しない。永続化（projection 結果書き込み）の失敗は従来どおり warn ログで継続し描画を妨げない（#1213 streaming persist 方針）。
- **既存エラー文言の汎化**: フルパス・serde 生メッセージを API へ漏らさない既存方針を維持。

---

## テスト方針

Rust は各モジュール `#[cfg(test)]`。CLAUDE.md / `docs/architecture/TEST.md` に従う。frontend 変更はないため frontend テストは追加しない。

### projector / event_log（usecase, 純粋ロジック）

- **append → projection の境界テスト**（受け入れ基準 2 / R3）: `TurnStarted` → `ToolCallStarted` → `ToolCallSucceeded` → `TurnCompleted` の event 列を project し、message page に prompt と tool 結果が含まれること。event 列に無い message が現れないこと。
- **決定性**: 同一 event 列を 2 回 project して結果が一致すること。
- **live-only 非依存**: event 列のみから read model が完全再構築でき、legacy live バッファを適用しなくても欠落しないこと。live-only delta が event 列に混入しないことを append API とテストで保証。
- **session status の対応表**: 上記網羅表の各行（streaming 中 / permission 待ち / Idle / Error / Closed）を event 列から projection して期待 status になること（受け入れ基準 8 / R3）。
- **runtime memory 非参照**: runtime を参照せず event 列のみで streaming 中・permission 待ちが判定できること。
- **finalization**（受け入れ基準 5 / R4）: 進行中 turn ＋ 未完了 tool call ＋ 未解決 permission を持つ event 列に対し、abort / timeout / bridge crash それぞれで終端 event が付与され、projection 後に partial が残らないこと。終端後の完了判定が一意であること。
- **reconnect 二重適用なし**: 同一 event 列の再 projection で message が重複しないこと。
- **互換性（golden）**（受け入れ基準 6 / R6）: 代表的な SDK メッセージ列に対し、`project(turn_event_log)` の `Vec<MessagePart>` が現行 `consolidate_parts_from_slice` の出力と一致すること。

### bridge_common（infrastructure, 経路置き換え）

- **3 経路の event append**（受け入れ基準 7 / R7）: `accumulate_sdk_message` が durable event を append し live delta を分離すること。`run_turn_complete_transition_locked` が `TurnCompleted` を、`run_bridge_error_transition_locked` が finalization event を append すること。
- **workflow 通知の event 由来**（B5）: `WorkflowTurnCompleteInput` が runtime `BridgeState` ではなく projection から導出されること。`TurnStarted` を持つ turn の終端でのみ通知されること。
- **既存テスト維持**: turn-complete / bridge-error / timeout / interrupt の既存テスト群を新経路で通す。

---

## リスクと代替案

### リスク

- **`bridge_common.rs`（約 19,350 行）への侵襲**: 3 経路の差し替えは大規模ファイルの hot path に触れる。緩和: ロジックを `event_log` モジュール（純粋関数）へ寄せ、`bridge_common` 側は event append と projection 呼び出しに限定する。`event_log` を seam として残し #1217 の分割に備える。
- **`project()` の呼び出し頻度**: streaming 中の毎 delta で全 event を再 project すると O(N²) になりうる。緩和: live バッファは incremental に更新し（直近確定 event のみ反映）、full projection は flush / turn 完了 / reconnect 等の境界に限定する。determinism テストは full projection で担保。
- **golden 不一致の検出漏れ**: `project` と `consolidate_parts_from_slice` の出力差は observable behavior の破壊に直結する。緩和: 代表 SDK メッセージ列の golden 比較を必須テストとし、差分が出たら projector を修正する（実装を仕様に合わせる、CLAUDE.md 方針）。
- **TurnPhase の二重定義**: infra `bridge_common::TurnPhase`（`Serialize` のみ）と usecase `status::TurnPhase`（`Serialize + Deserialize`）が併存する。projector は usecase 側を出力とし、infra 側は live 表示のために維持する（本変更で統合はしない / スコープ外）。

### 代替案（不採用理由つき）

- **event 列を独立ファイルへ即時永続化**: crash 後に event 列そのものから決定的再構築できる利点があるが、A2（初期は互換 projector）と R6（既存永続構造と矛盾しない）に照らし、新フォーマット導入は scope 過大。projection 結果を既存 split layout へ書く互換 projector を採用し、独立永続化は将来拡張余地とする。
- **projector に finalization を推論させる（終端 event を append しない）**: event 列を小さく保てるが、「event 列が session の事実を自己完結して表す」という正典の不変条件が崩れ、projector の暗黙ルールに依存する。明示的終端 event の append を採用し、event 列だけで partial 不在を保証する。
- **`streaming_parts` を完全廃止し projection 結果のみ保持**: 概念は綺麗だが cumulative snapshot の emit 経路を大幅に書き換え、#1214 のスコープに踏み込む。`streaming_parts` を「projection の保持器」として再定義する最小変更に留める。

---

## 仮定（Assumptions）

`requirements.md` A1〜A5・`behavior.md` B1〜B5 を前提とする。本設計固有の仮定:

- **D1.** event 列は初期実装では **runtime 保持（`AgentProcess::TurnEventLog`）** とし、独立永続化はしない。永続化されるのは projection 結果（既存 split layout）。crash 後の決定的再構築は「永続化済み projection を読む」＋「event 列に対する unit test で決定性を示す」で満たす（A2）。
- **D2.** `AgentSessionEvent` の語彙は OpenCode をそのまま写すのではなく、既存 `MessagePart` / `BridgeState` / `TurnPhase` と整合する Releash 固有語彙とする（A3）。`MessagePart` の各 variant に projection で到達できることを最小要件とする。
- **D3.** `project()` が現行 `consolidate_parts_from_slice` と同一の `Vec<MessagePart>` を生成することを observable behavior 不変の判定基準（golden）とする。
- **D4.** `bridge_common.rs` は分割しない（#1217 / Non-goal）。event 駆動化は `event_log` モジュール利用として同ファイル内に閉じる。
- **D5.** session status の `Done` / `Archived` は turn event からは導出せず、既存ライフサイクル遷移を維持して projection 結果へ上書き合成する。turn 由来は `Active` / `Idle` / `Error` / `Closed` と `TurnPhase` の二軸に限定する。
- **D6.** frontend 変更・新規 Tauri コマンドは伴わない（A4）。projection 結果の serde 表現が不変のため frontend からは透過。

---

## Open Questions

なし。`requirements.md` / `behavior.md` の Open Questions はすべて解消済みであり、本設計で新たに人間の判断を要する未確定点は生じていない。保存形式（互換 projector vs 独立永続化）は A2 により互換 projector と確定済み、語彙・projector 構造・モジュール配置・session status 対応表・finalization 終端 event は本書で確定した。
