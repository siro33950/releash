# design.md — S10a: エラーの live 着地（crash / Fatal の即時可視化）

- Issue: #1398 / Milestone 84 Phase 0
- 解消する監査項目: FE-2（high）, RT-6（low）
- 入力: `requirements.md`, `behavior.md`
- 正本参照: `specs/milestone-84-agent-chat-stabilization/agent-chat-instability-audit.md`（FE-2 / RT-6）、`agent-chat-ideal-lifecycle.md` I12

## 概要

Agent チャットの 2 経路のエラー — turn 実行中の CLI プロセス死（crash / FE-2）と、Idle 中の backend プロセス死（Fatal / RT-6）— を、**発生時点で live UI（chat panel）に Error block として即時着地させ、durable にも記録して reload 後の表示と一致させる**。

現状の 2 つのギャップを埋める。

1. **crash（FE-2）**: `complete_turn` は durable 側で Error part を合成・永続化するが（`finalize_turn` → `TurnInterrupted{error}` → projector `push_unique_error` → `persist_message_parts`）、その Error part を **live（`agent-streaming-delta`）へは emit しない**。`complete_turn` 冒頭の `flush_streaming_update` は finalize より前に走るため、合成された Error part は live stream に乗らない。live 経路は `agent-session-state-changed` のみで、frontend の同 handler は transcript に Error を追加しない（badge state のみ更新）。結果、生成中の streaming message から spinner が消えるだけで、Error block は reload 後にしか現れない。

2. **Idle-Fatal（RT-6）**: `apply_runtime_event` の Fatal 分岐（`should_complete_crash == false`、phase == Idle）は `set_session_state(Error)` と理由なしの transient state change のみ。durable event も Error part も残さず、`project_status` は active turn の terminal が無い限り `Error` を返さないため、次の event append 時の再投影（`append_session_event_and_project_state`）で meta の `state` が上書きされ、Error の痕跡が消える。理由も payload に無く live/durable どちらにも残らない。

方針は **現行の `MessagePart::Error` / `SessionState::Error` の表示語彙に閉じる**こと。両経路の live 着地は既存の `agent-streaming-delta`（snapshot）を再利用し、durable 記録は projector が Error part と Error state を再投影で復元できる形にする。badge の理由 tooltip は backend read model（`SessionSummary` / `get_session`）が source of truth。

## 変更対象

### backend（Rust）

| ファイル | 変更 |
|---|---|
| `usecase/agent_session/runtime/usecase.rs` | `complete_turn`: finalize 後に projected parts（Error part 含む）を `agent-streaming-delta` snapshot として emit（FE-2）。Fatal 分岐（Idle 時）: durable Fatal 記録 → projection → Error message parts 永続化 → live snapshot emit → 理由付き state change（RT-6）。 |
| `usecase/agent_session/event_log/events.rs` | `AgentSessionEvent` に message identity・発生時刻・理由を持つ `SessionErrored` variant を追加（Idle-Fatal の durable 表現）。 |
| `usecase/agent_session/event_log/projector.rs` | `SessionErrored` を処理: standalone な agent Error message（`MessagePart::Error`）を発生順に履歴へ合成し、履歴とは別に現在の Error 状態と最新理由を投影する。 |
| `usecase/agent_session/session/mod.rs` | `SessionSummary` / `ChatSession` に `error_reason: Option<String>` を追加。 |
| `usecase/agent_session/session/store.rs` / `read_model.rs` | `SessionSummary` / `ChatSession` 構築時に projection 由来の `error_reason` を充填。 |
| `usecase/agent_session/runtime/ports.rs` / `adaptor/presenter/agent_session.rs` | 既存 `agent-streaming-delta` snapshot に、unknown message を生成する場合だけ backend 生成済み `ChatMessage` を任意フィールドとして添付する。新しい event 種別は追加しない。 |

### frontend（React/TS）

| ファイル | 変更 |
|---|---|
| `types/session.ts` | `SessionSummary` / `ChatSession` 型に `errorReason?: string \| null` を追加（表示用）。`MessagePart` の `error` variant は既存。 |
| session badge component（一覧・タブの状態表示） | Error 状態のとき `errorReason` を tooltip（`title`）に表示。 |
| listener / reducer | listener は snapshot に完全な `message` が添付された場合だけ既存 `ADD_MESSAGE` で mirror してから `SET_STREAMING_MESSAGE` を適用する。reducer は Error variant・role・timestamp を判断／合成しない。Error block は既存 `ChatSessionView` の `case "error"` で描画される。 |

## アーキテクチャと責務分割

- **判断ロジックは Rust に閉じる**（rust-first-logic）。何を Error として live/durable に出すか、理由の合成、state 復元は usecase / projector が所有する。
- **live 着地の transport は既存経路を再利用**する。新規 Tauri event / DTO 種別は追加しない（`requirements A2`）。
  - Error part の live 送出 = `agent-streaming-delta`（`snapshot: true`）。
  - session state の live 送出 = `agent-session-state-changed`。
- **durable の source of truth は event log**。`SessionSummary.state` / `ChatSession.state` / `error_reason` はいずれも event log の projection から導出し、meta への永続化は projection 結果のミラーに留める。これにより再投影で復元可能になり、上書き消失（RT-6 / R4）を構造的に防ぐ。
- **frontend は表示に徹する**。badge tooltip は `errorReason` の描画のみ。Error block の描画は既存 part renderer を使う。

## データモデルまたは型

### 新規 durable event

```rust
// event_log/events.rs — AgentSessionEvent に追加
SessionErrored {
    message_id: String,
    reason: String,
    at: f64,
},
```

- Idle-Fatal（active turn が無い）を durable に表現する専用イベント。crash 側は既存 `TurnInterrupted { error }` をそのまま使う。
- `message_id` は Fatal episode ごとに backend が生成する固有 ID、`at` は発生時刻、`reason` は `AgentRuntimeEvent::Fatal { message }` の message をそのまま格納する。これにより event log のみから page read model の identity・timestamp・順序を再現できる。

> 仮定 D1: `AgentSessionEvent` への variant 追加は「durable event log の内部表現」であり、`requirements R6` / 非スコープが禁じる**表示語彙**（`Notice` 等の新 UI 語彙）には当たらない。表示は既存 `MessagePart::Error` / `SessionState::Error` に閉じる。代替（synthetic turn）は「リスクと代替案」を参照。

### projector 拡張

- `project()` は全 `SessionErrored` から **agent-only の合成 ChatMessage**（human prompt を伴わない）を発生順に履歴へ追加し、各 message に event の `message_id` / `at` / `reason` をそのまま使う。
- 現在の `session_errored_reason: Option<String>` は履歴とは別に集約する。`SessionErrored` で最新理由を設定し、後続 `TurnStarted` で現在状態だけを解除する。過去の Error message は transcript から削除しない。
- `project_status`: 現在の `session_errored_reason.is_some()` の場合、`ProjectedStatus { session_state: Error, turn_phase: Idle }` を返す（active turn の terminal と同格に扱う）。
- `SessionReadModel` に `error_reason: Option<String>` を追加（`session_errored_reason`、または crash 経路の場合は最新 turn の Error part content から導出）。

### read model への理由露出

```rust
// SessionSummary / ChatSession に追加
pub error_reason: Option<String>,  // serde: skip_serializing_if = "Option::is_none"
```

- `state == Error` のときのみ意味を持つ。source of truth は projection。

### frontend 型

```ts
// types/session.ts
interface SessionSummary { /* ... */ errorReason?: string | null }
interface ChatSession    { /* ... */ errorReason?: string | null }
// MessagePart の { type: "error"; content } は既存を利用
```

## 処理フロー

### A. crash（turn 中 CLI 死 / FE-2）

trigger: `AgentRuntimeEvent::Fatal`（phase != Idle → `should_complete_crash == true`）または `TurnCompleted(Interrupted{Crash})` → `complete_turn`。

1. `flush_streaming_update`（既存, finalize 前）。
2. `finalize_turn` が組み立てた `ToolCallFailed` + `TurnInterrupted{reason: Crash, error}` を、SessionStore から storage-owned projected read model transaction に渡す。
3. storage transaction は一つの file lock 内で最新の event log / meta を読み、再投影後に event append → Error part を含む対象 message 更新 → index/meta projection を完了する。checkpoint は変更対象ファイルだけに限定し、途中失敗時は lock を保持したまま変更前へ戻す。通常 turn で transcript 全 message を hydrate しない。
4. transaction が成功して返した persisted parts だけを `emit_streaming_delta_or_retry` で `snapshot: true` として emit（message_id = 当該 agent message）。→ frontend `SET_STREAMING_MESSAGE` が streaming message の parts を Error part 込みに置換。
5. `emit_session_state_change`（既存, `session_state: Error`, `turn_phase: Idle`）。→ frontend が streaming message を finalize（`MARK_AGENT_TURN_COMPLETED`）し badge state を更新。
6. 結果: 直前までの出力 + Error block が live に残る（behavior「crash 前の出力は併存」）。reload では手順3の永続化 parts が同一表示を再現。

> emit 順序は「snapshot（手順4）→ state change（手順5）」。Tauri event は順序保存されるため、frontend は Error part を transcript に反映してから idle 化する。

### B. Idle-Fatal（Idle 中 backend 死 / RT-6）

trigger: `AgentRuntimeEvent::Fatal`（phase == Idle → `should_complete_crash == false`）。

1. runtime を close、phase = Idle（既存）。
2. **【新規】** Fatal episode ごとの `message_id` と `at` を生成し、`SessionErrored { message_id, reason, at }` を SessionStore から storage-owned projected read model transaction に渡す。
3. **【新規】** transaction は一つの file lock 内で最新 event/meta を入力に一度だけ再投影し、event append → 合成 Error message append → index/meta projection を完了する。checkpoint は event log・追加 message・index/meta に限定し、途中失敗時は lock 内で復元する。成功時は `state = Error` が meta に残り、後続 projection でも復元される。
4. **【新規】** transaction が成功して返した完全な Error message と同じ parts だけを `snapshot: true` の `agent-streaming-delta` として emit。→ frontend は backend の role / timestamp / identity をそのまま mirror して Error block を描画（**live 着地**）。
5. `emit_session_state_change`（`session_state: Error`, `turn_phase: Idle`）で streaming message を finalize、badge state を Error に。
6. 結果: 理由付き Error block が live/durable/reload で一致。`SessionSummary.error_reason` / `ChatSession.error_reason` から badge tooltip の理由が読める。

### C. badge tooltip（R5）

- `list_sessions` → `SessionSummary { state: Error, error_reason }`、`get_session` → `ChatSession { state: Error, error_reason }`。
- frontend の badge component は `state === "error"` のとき `errorReason` を `title` に設定。reload 後も projection 由来のため理由が残る。

## エラー処理

- Error episode の durable 書き込み（event append / page message append または `persist_message_parts` / meta projection）は storage の一つの atomic operation として扱う。途中失敗時は transaction lock 内で変更対象ファイルだけを復元し、durable 着地を確認できない Error snapshot と Error state は live publish しない。並行する meta/event 更新は transaction の後に直列化され、rollback で失われない。
- live emit 自体の失敗は既存の retry / suppression（`STREAM_EMIT_FAILURE_*`）に従い、完全な message metadata も parts と一緒に再送する。ただし turn 終端の authoritative snapshot は suppression を解除して必ず新しい送信試行を開始し、失敗した場合も retry に保持する。
- Idle-Fatal の `SessionErrored` は episode ごとに固有 message ID を持つ。連続 Fatal はそれぞれ履歴に残し、`project_status` / `error_reason` は最後の episode を現在状態として採用する。
- crash と Idle-Fatal が近接した場合、crash（should_complete_crash==true）が優先され turn の `TurnInterrupted` 経路に流れる。`SessionErrored` は active turn が無いときのみ生成する。

## テスト方針

### Rust（usecase / projector）

- `complete_turn`（crash）: finalize 後に `snapshot: true` の streaming delta が **Error part を含んで** emit されること、かつ `persist_message_parts` の parts と live snapshot の parts が一致すること（AC1 / AC3 / AC5）。notifier をテストダブルにして emit 内容を検証。
- Fatal（Idle）: `SessionErrored` が append され、再投影で `state == Error` と Error message が復元されること。live snapshot に理由付き Error part が乗ること（AC2 / AC5）。
- projector: `SessionErrored` 後に無関係な event を append・再投影しても `state == Error` と `error_reason` が維持されること。後続 turn 開始時は現在 Error だけを解除し、履歴 message は保持すること。複数 Fatal の ID・timestamp・順序が再現されること（AC4 / R4）。
- read model: `SessionSummary` / `ChatSession` の `error_reason` が Error 時に理由を返し、非 Error では `None`（R5）。
- reload 一致: crash / Idle-Fatal それぞれで、live emit した parts と `get_session` の messages が同一であること（AC3、behavior「live と reload の表示等価」の Scenario Outline を Rust で固定）。
- durable failure: append-event / meta projection / append-message / `persist_message_parts` の失敗注入で event log・meta・page が変更前へ戻り、未確定 Error snapshot / Error state が live publish されないこと。
- storage transaction: event 書き込み後の失敗中に並行 meta/event 更新を開始しても、rollback 後にその更新が直列化され失われないこと。長い transcript でも terminal materialization が読む message chunk 数は対象 message の一定数に留まること。

### frontend

- `agent-streaming-delta`（snapshot, Error part 含む）受信で transcript に Error block が現れること。unknown message の snapshot は添付された完全な backend message を `ADD_MESSAGE` してから `SET_STREAMING_MESSAGE` を適用し、reducer が Error variant やローカル時刻から message を合成しないことを検証。
- badge component: `state === "error"` かつ `errorReason` 指定時に tooltip（`title`）へ理由が出ること、reload 相当の再描画でも残ること（R5）。

## リスクと代替案

- **リスク: `SessionErrored` variant 追加が「語彙拡張」と解釈される**。非スコープの「語彙拡張をしない」は `Notice(kind)` 等の**表示語彙**を指すと解釈（D1）。表示は `MessagePart::Error` / `SessionState::Error` に閉じるため要求は満たすが、レビューで論点化し得る。
  - **代替 A（synthetic turn）**: 新 event を足さず、Idle-Fatal を `TurnStarted` + `FinalPartsRecorded(Error part)` + `TurnInterrupted{Crash}` の合成 turn として記録し、既存語彙のみで再投影 Error を得る。欠点は `TurnProjection::to_messages` が human prompt message を必ず生成するため、空の人間メッセージが transcript に混入すること。抑止には projection への special-case（synthetic prompt 判定）が別途必要で、footprint は同等以上。
  - **代替 B（projector 特殊化のみ）**: event を足さず、Idle-Fatal 時に meta の `state` を Error にした上で projector が「meta が Error かつ terminal 無し」を見て Error message を合成。欠点は meta が projection の入力に混ざり source-of-truth が二重化、再投影での上書き（R4）を構造的に防げない。RT-6 の根本原因（meta と projection の乖離）を再生産するため不採用。
  - 採用理由: 本 design（専用 durable event + projection 単一 source）は R4 を構造的に満たし、phantom human message を生まないため代替 A/B より副作用が小さい。

- **リスク: live snapshot と state change の順序逆転**で一瞬 Error block 無しの idle が見える。→ 同一 `apply_runtime_event` / `complete_turn` 内で snapshot を先に emit し、Tauri の順序保存に依拠して回避。

- **リスク: badge component の特定**。session 状態 badge の描画箇所は実装時に確定する（`error_reason` を渡す props 経路のみ）。backend の read model 変更は badge 実装に依存しないため、backend を先行実装し frontend は tooltip 追加のみで閉じる。

## 仮定

- D1: `AgentSessionEvent::SessionErrored` の追加は durable event log の内部表現であり、`requirements R6` / 非スコープが禁じる表示語彙拡張には当たらない。
- A2（requirements 継承）: crash の live 착지は既存 transient 経路（`agent-streaming-delta` / `agent-session-state-changed`）の再利用で行い、新規 event / DTO 種別は追加しない。durable 側 Error part 合成は既存挙動を維持する。
- A3（requirements 継承）: Error 理由の source of truth は event log projection に置き、meta / `SessionSummary` / `ChatSession` はそのミラー。再投影で復元可能にすることで RT-6 の上書き消失を防ぐ。
- A4（requirements 継承）: badge 理由 tooltip は frontend 表示のみを担い、理由は backend read model 由来。
- A6（behavior 継承）: active turn / message が無い Idle-Fatal の Error part は、**human prompt を伴わない合成 agent message**（episode ごとの固有 message id）へ紐付ける。
- crash の live snapshot は当該 agent message id を再利用し、Idle-Fatal は Fatal episode ごとに新しい message id を用いる。

## Open Questions

なし。
