# Design

`requirements.md` の R1〜R11 と `behavior.md` を、実コード調査に基づく具体設計へ落とし込む。source of truth は Rust 側 runtime / read model に置き、frontend は backend-owned な pending permission を描画するだけに留める。

## 概要

backend runtime は permission request を受けると `RuntimeSessionState.pending_permission_request: Option<PermissionRequestMsg>` を保持し `phase = WaitingPermission` へ遷移する（`runtime/usecase.rs:2022` の `PermissionRequested` 処理で確認）。しかし UI の `PermissionDialog` は message part の `case "permission"`（`ChatSessionView.tsx:528` 系、実際は `AgentMessageParts` 内 `msg.parts.map`）だけを描画元にしており、message part に permission part が届かない／transient delta を失った場合に「backend は回答待ちだが UI に dialog が無い」不可視停止が成立する。

本設計は次の 2 系統で不可視停止を解消する。

1. **初期ロード経路の穴を塞ぐ（①③）**: `get_session`（`GetSessionResponse`）に backend-owned な `pending_permission_request` を載せ、backend runtime が継続している reload / tab 移動 / 後から open でも復元できるようにする。`get_session` は runtime state の `pending_permission_request` だけを読む。runtime state に pending が無い場合は、古い durable event log から回答不能または解決済みの checkpoint を再導入しないため pending を返さない。
2. **描画経路の穴を塞ぐ（②⑥）**: message parts に対象 request の permission part が無い場合に限り、reducer が保持済みの `pendingPermissions[sessionId]` を fallback として `PermissionDialog` で描画する。二重表示は request id 一致で抑止する。

加えて可観測性（④⑤）と workflow step session の導線（⑦）を扱う。

## 変更対象

### Backend（Rust）

- `src-tauri/src/usecase/agent_session/session/mod.rs`
  - `GetSessionResponse`（`:929`）に `pending_permission_request: Option<PermissionRequestMsg>` を追加（①）。
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs`
  - `get_session`（`:773` 付近）で `pending_permission_request` を合成して `GetSessionResponse` に載せる（①③）。
  - `WaitingPermission` 長期化 × visible dialog 不在の診断イベント（⑤）。※判定に必要な状態は Rust 側が所有。
  - `stream_emit_suppressed` 中に `PermissionRequested` が来ても state-change fallback が確実に emit されることの担保（⑥）。
- `src-tauri/src/usecase/agent_session/event_log/finalization.rs`
  - 挙動変更なし（R6: 未解決 permission を `Cancelled` で閉じる既存挙動を維持）。回帰防止テストのみ追加。

### Frontend（TypeScript / React）

- `src/hooks/useSessionStore.ts`
  - `RawGetSessionResponse`（`:188`）/ `GetSessionResponse`（`:112`）に `pendingPermissionRequest` を追加。`convertRawGetSessionResponse`（`:217`）で透過。（①R2）
- `src/hooks/useAgentChat.ts` の `dispatchSessionMeta`（session hydrate 経路）
  - `getSession` 応答の `pendingPermissionRequest` から `SET_PENDING_PERMISSION` を dispatch。（①R2）
- `src/hooks/agentChatReducer.ts`
  - `APPLY_STREAMING_DELTA`（`:541` 系）の対象 message / session 未存在で捨てる結果を副作用層で warn。（④R8）
  - `pendingPermissions` state は既存（`:44`）を再利用。
- `src/components/panels/AgentChatPanel/ChatSessionView.tsx`
  - message parts に permission part が無く `pendingPermissions[sessionId]` が在る場合の fallback `PermissionDialog` 描画（②R3/R4）。
- `src/components/panels/WorkflowView/*` および `BoundSessionChat`
  - workflow step session で fallback dialog が一覧・detail の両導線から到達可能であることの確認と、不足があれば配線（⑦R11）。fallback は `ChatSessionView` 内に置くため、既存の step session 描画経路をそのまま流用できる想定。

## アーキテクチャと責務分割

- **source of truth**: `RuntimeSessionState.pending_permission_request`（in-memory）と、それを durable に裏付ける event log の `PermissionRequested` / `PermissionResolved`。`get_session` は session open/reload の read path なので、event log 全体の読み直しではなく runtime state の軽量 read model を返す。
- **read model の合成は Rust**: `get_session` が pending permission を返す。frontend は返り値を `pendingPermissions[sessionId]` に hydrate するだけで、domain decision（二重表示判定を除く純表示分岐）を増やさない。
- **frontend の責務**: reducer が保持する `PermissionRequest` を、message part に無いときだけ fallback として描画する（表示のみ）。full-retention / 再計算経路は新設しない（R7）。

### ② fallback 描画の判定位置

`ChatSessionView` 内に置く。理由: `PermissionDialog` の既存描画（`AgentMessageParts` 内）と同じ component tree で、message parts と `pendingPermissions[sessionId]` の双方を参照できるため。表示規則:

- `pendingPermissions[sessionId]`（= `PermissionRequest`）が存在し、かつ
- その `request.id` と一致する `permission` part がどの message parts にも存在しない

ときだけ fallback dialog を 1 つ描画する。message part 側に同 id の permission part がある場合は fallback を描画しない（R4 二重表示防止）。fallback dialog は既存 message list の末尾（最新 agent message の直後相当の位置）に描画する。

## データモデルまたは型

### Backend

`PermissionRequestMsg`（`session/mod.rs:127`）を転送 shape に採用する。frontend `PermissionRequest`（`src/types/session.ts:101`）と field 対応が取れており、`PermissionDialog` の `request` prop・`respond_agent_permission` 呼び出しに必要な情報（id / tool_name / kind / input / plan / allowed_prompts / questions / title / display_name / description / decision_reason）を過不足なく供給できる（仮定の裏付け）。

```rust
// session/mod.rs GetSessionResponse に追加
#[serde(skip_serializing_if = "Option::is_none", default)]
pub pending_permission_request: Option<PermissionRequestMsg>,
```

`#[serde(rename_all = "camelCase")]` により JSON は `pendingPermissionRequest` になる。`skip_serializing_if` で回答待ちでない通常 session の payload を増やさない（full-retention 回避）。

### Frontend

```typescript
// RawGetSessionResponse / GetSessionResponse に追加
pendingPermissionRequest?: PermissionRequest | null;
```

`SET_PENDING_PERMISSION`（`agentChatReducer.ts:474`）と `pendingPermissions: Record<string, PermissionRequest>`（`:44`）は既存。event 経路（`useAgentSdkListeners.ts:319`）と hydrate 経路が同じ action を共有する。

## 処理フロー

### ①③ get_session が pending permission を返す

`get_session`（`runtime/usecase.rs:773`）で `GetSessionResponse` 生成時に `pending_permission_request` を次の規則で解決する。

1. `ctx.sessions` lock 下で対象 session の `state.runtime.is_some()`、`state.phase == WaitingPermission`、`state.pending_permission_request == Some(...)` がすべて成立するなら、それを clone して返す。frontend reload / tab 移動（backend プロセス継続）はこの経路で復元される。
2. runtime state に pending が無い場合は `None` を返す。ここで persisted event log を再走査しないことで、`respond_permission` 中に runtime state は clear 済みだが durable `PermissionResolved` 追記前、または古い `get_session` hydrate が遅延した場合に、解決済み permission を UI へ復活させない。

この構成により、backend runtime が継続している範囲で R5（streaming delta を一切受け取れなくても reload / 後から open で復元）を満たす。read model は backend 内で完結し、frontend は結果を hydrate するだけ（R7）。`find_permission_request` / `respond_permission` の補助的な検証経路では durable event log を参照できるが、session open/reload の read path である `get_session` は full-retention な event log parse に依存しない。

> 仮定: message read model への「即 projection / persist」ではなく、runtime state の `pending_permission_request` を `get_session` に載せる方式を採用する。理由は (a) 既に `apply_parts(Immediate)` が streaming message read model へ permission part を追記しており二重投影を避けたい、(b) `PermissionRequested` 処理時点で runtime state が回答待ちの source of truth を保持している、(c) 通常の session open/reload read path で event log 全体を読み直さない、(d) runtime が無い session や clear 済み session に回答不能な pending checkpoint を提示しない。R5 の性質（backend runtime 継続中の reload / 再表示で復元できる）を満たせばよいという要求の許容範囲内（requirements.md 仮定・③）。

### ①R2 frontend hydrate

`getSession` → `convertRawGetSessionResponse` が `pendingPermissionRequest` を透過 → session hydrate（`dispatchSessionMeta`）で `SET_PENDING_PERMISSION`（request or null）を dispatch。既存 event 経路と同じ action を使うため、event と reload の双方で `pendingPermissions[sessionId]` が同一 shape に収束する。

### ②R3/R4/⑥ fallback 描画

`ChatSessionView` で message parts を走査し「permission part の request.id 集合」を作る。`pendingPermissions[sessionId]` が在り、その id が集合に無ければ fallback `PermissionDialog` を描画する。`onAllow` / `onDeny` / `onAnswer` は既存 `respondPermission`（`useAgentChat.ts:1244`、`invoke("respond_agent_permission", ...)`）を流用する。

⑥（`stream_emit_suppressed` 中）: delta が抑止されても `emit_session_state_change`（`PermissionRequested` 処理内）は独立に emit されるため、reducer は `pendingPermissions` を保持でき、fallback だけで回答できる。設計上の追加作業は「state-change emit が delta suppression 経路と独立していること」の担保と回帰テストのみ。

### 回答による解消

fallback から回答しても経路は既存と同一（`respond_agent_permission`）。runtime `respond_permission`（`usecase.rs:412`）が `pending_permission_request = None` / `phase = Streaming` に遷移し、`agent-session-state-changed`（pending=None）を emit → reducer が `SET_PENDING_PERMISSION(null)` で `pendingPermissions[sessionId]` を削除 → fallback dialog が閉じる（behavior: 「復元した permission に回答すると回答待ちが解消する」）。

### ④R8 delta 破棄の warn

`agentChatReducer.ts` の `APPLY_STREAMING_DELTA` は reducer の純粋性を保ち、session 未存在 / message 未存在（`applyMessageUpdate` が `null`）では drop reason を返す state 遷移だけに留める。`useAgentSdkListeners.ts` などの副作用層が reducer の結果を見て `console.warn`（sessionId / messageId / 破棄理由）を出す。warn は診断用途に限定し、reducer 内で副作用・集計・保持はしない。

### ⑤R9 回答待ち停滞の診断

判定状態は Rust 側が所有する。`RuntimeSessionState.permission_wait_started_at`（既存）に加え、`PermissionDialog` 表示中に frontend から `report_agent_permission_request_observed` で visibility observation（session id / request id / visible）を送る。backend は `RuntimeSessionState.permission_request_visibility` に request id と最終確認時刻を保持し、TTL 内の observation が現在の `pending_permission_request.id` と一致する場合だけ permission request が観測済みと判定する。

既存 tick / watchdog 経路は `WaitingPermission` が閾値時間継続し、かつ Rust-owned visibility state 上で visible dialog が無い場合に診断イベント（log/telemetry）を emit する。dialog unmount / 回答 / turn 終端 / 新規 permission request では visibility state を clear し、frontend が落ちたまま可視扱いで固定されないよう heartbeat は TTL で失効させる。

### ⑦R11 workflow step session の導線

workflow step session も `BoundSessionChat` → `ChatSessionView` を通るため、② の fallback は step session でもそのまま効く。一覧・detail の両導線が `ChatSessionView`（同じ session id）へ到達することを確認し、到達しない導線があれば pending checkpoint の可視化（バッジ等）と open 配線を追加する。source of truth は同じ backend read model。

## エラー処理

- `get_session` は runtime state に pending permission が無い場合、event log fallback を行わず `pending_permission_request = None` として返す。これにより session open/reload の read path で event log 全体の clone/parse を避け、回答後の古い pending 復活も防ぐ。
- `respond_agent_permission` の失敗は既存どおり `SET_ERROR` で表面化（`useAgentChat.ts` 既存）。
- id mismatch（pending と response の request_id 不一致）は既存 runtime 実装（`usecase.rs:412` 系）がエラーを返す。fallback 描画は常に `pendingPermissions[sessionId]` の id を使うため mismatch は起きない。
- `report_agent_permission_request_observed` の失敗は permission 回答フロー自体を止めず、frontend 側の warn に留める。診断用 read model の更新に失敗した場合は observation が無い状態として扱われ、R9 の診断で検知可能にする。

## テスト方針

### Rust（`#[cfg(test)]`）

- `get_session` が `WaitingPermission` 中の in-memory `pending_permission_request` を返す（R1）。
- live runtime はあるが runtime state に pending permission が無い状態で、event log に未解決 `PermissionRequested` が残っていても `get_session` は pending permission を返さない（stale 復活防止）。
- live runtime が無い persisted-only session では、event log に未解決 `PermissionRequested` があっても `get_session` が `turn_phase=Idle` / `pending_permission_request=None` を返す（回答不能 checkpoint の再提示防止）。
- 未解決 permission の finalize が従来どおり `Cancelled` になる（R6 回帰防止、`finalization.rs`）。
- `stream_emit_suppressed = true` でも `PermissionRequested` の state-change emit（pending 付き）が行われる（R10/⑥）。
- `WaitingPermission` 長期化時、matching request id の fresh visible heartbeat がある場合は診断せず、heartbeat が無い／失効／mismatch の場合だけ診断イベントを出す（R9）。

### Frontend（`*.test.ts(x)`）

- `convertRawGetSessionResponse` が `pendingPermissionRequest` を透過し、hydrate で `SET_PENDING_PERMISSION` が dispatch される（R2）。
- `agentChatReducer` の `SET_PENDING_PERMISSION` set/clear（既存 + null 経路）。
- `ChatSessionView`: message part に permission が無く `pendingPermissions` だけある場合 fallback dialog を描画（R3）／両方ある場合は 1 つだけ（R4）。
- `APPLY_STREAMING_DELTA` の対象 message / session 未存在で warn が出る（R8）。
- `PermissionDialog` は pending 表示中だけ visible heartbeat を送り、unmount 時に visible=false を送る（R9）。

### integration

- Claude `AskUserQuestion`: reload 後も dialog 表示・回答（R5）。
- Codex `item/tool/requestUserInput` / `requestApproval`: tab 移動後も dialog 表示・回答（R5）。
- workflow step session の hidden / reopened で一覧・detail から開いて回答（R11）。

### 品質ゲート

`pnpm lint` / `pnpm test` / `pnpm build` / `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`。

## リスクと代替案

- **③ の実現手段（採用: get_session 合成／代替: 即 projection・persist）**: 代替は permission part を `PermissionRequested` 時点で read model へ即 projection する案。UI 主経路（transient delta）と durable 経路の両方に permission part が乗り、reload でも message part 経由で復元できる。ただし二重投影・書き込み経路増加のリスクがあるため、read 側合成を採用。R5 を満たす限りどちらでもよい（requirements 仮定）。
- **⑤ の "visible dialog 不在" 判定**: frontend 表示状態そのものは backend が直接観測できないため、frontend からの heartbeat を Rust-owned read model に取り込む。heartbeat が送れない／失効した場合は不可視として診断するため、過検知はあり得るが、dialog が見えている通常の回答待ちは fresh heartbeat により除外できる。
- **fallback 描画位置**: `ChatSessionView` 内に固定。親（`BoundSessionChat`）に上げる案もあるが、message parts 参照が必要なため tree 内が自然。
- **転送 shape**: `PermissionRequestMsg` を採用。frontend `PermissionRequest` と field 差異があれば `PermissionDialog` が壊れるため、変換整合を test で固定する。

## 仮定

- `PermissionRequestMsg` は `PermissionDialog` 描画と `respond_agent_permission` に必要な情報を過不足なく供給できる（field 対応を実コードで確認済み）。
- backend プロセスは frontend reload / tab 移動で再起動しないため、復元は in-memory runtime state で足りる。state-change / transient delta を UI が取りこぼしても `RuntimeSessionState.pending_permission_request` が残るため、`get_session` は event log を読み直さず pending を返せる。backend 再起動後や session 未ロード時の persisted-only permission は UI に提示しない。
- ② の二重表示防止は request id 一致判定で十分（同一 request が message part と pending の双方に在るのは一時的で、id が一致する）。
- workflow step session は `ChatSessionView` を共有するため fallback がそのまま効く（不足時のみ導線配線を追加）。

## Open Questions

なし。
