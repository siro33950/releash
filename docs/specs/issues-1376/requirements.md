# Requirements

## Type

バグ修正（human checkpoint の第一級性回復）。

agent session で permission request（Claude の `AskUserQuestion` / Codex の `requestApproval` / `item/tool/requestUserInput`）が発生し backend が回答待ち（`WaitingPermission`）に入っているのに、UI に `PermissionDialog` が表示されず、ユーザーから見て不可視のまま停止する事象を修正する。修正の source of truth は Rust 側 runtime / read model に置き、frontend は backend-owned な pending permission を描画するだけに留める。

関連: #1376（本 ISSUE） / 親方針: `.claude/rules/rust-first-logic.md`、CLAUDE.md「human checkpoint を第一級に扱う」

## 背景と目的

Releash の中心は workflow state であり、human checkpoint（承認・却下・回答待ち）を第一級に扱うことを方針としている。permission request はその human checkpoint の典型例である。

Issue #1376 の調査により、permission request の**変換**（provider → `AgentRuntimeEvent::PermissionRequested`）は Claude / Codex 双方で正しく行われており、runtime も正しく `WaitingPermission` に入り `pending_permission_request` を保持していることが確認された。問題は共通の**表示・復元経路**にある。

現状、UI の `PermissionDialog` は chat message の `MessagePart::Permission`（`ChatSessionView.tsx:528` の `case "permission"`）だけを描画元にしている。この permission part は runtime 中の transient event `agent-streaming-delta` を主経路として UI に届く。一方で backend の回答待ち状態を表す `pending_permission_request` は `agent-session-state-changed` で流れて reducer に保存されるものの（`useAgentSdkListeners.ts:319` / `agentChatReducer.ts:474`）、production では dialog の描画元・復元元に使われていない。

このため次の状況で「backend は回答待ちなのに UI に dialog が出ない」不可視停止が成立する。

- `agent-turn-prepared` 未受信・対象 message が state に無い・message body evict・別 view 等により `agent-streaming-delta` が適用されず捨てられる（`agentChatReducer.ts:525` / `:541`）。
- backend 側の `stream_emit_suppressed`（`runtime/usecase.rs:2441`）により delta 自体が抑止される。
- reload / tab 移動で transient event を失った後、`get_session` で復元しようとしても `GetSessionResponse` に `pending_permission_request` が無い（`session/mod.rs:929` で確認済み、フィールド不在）ため dialog を再構成できない。

本変更の目的は、backend が正しく停止している human checkpoint を UI が確実に復元・表示できるようにし、agent / workflow が人間の判断待ちで不可視停止する状態を解消することである。

### 現状のコード調査（事実 / Issue 記載を実コードで確認済み）

- **変換は正常**: Codex（`infrastructure/agent_session/codex/convert.rs:537`）と Claude（`infrastructure/agent_session/claude/convert.rs:186`、`claude/permission.rs:17`）はいずれも permission request を `PermissionRequest` / `PermissionRequestBody::Question` に変換し、共通 runtime へ渡している。この時点で provider 差は吸収済み。
- **runtime は回答待ちに入る**: `runtime/usecase.rs:2022` の `PermissionRequested` 処理で `DomainMessagePart::Permission` を `apply_parts(Immediate)` で流し、`phase = WaitingPermission` / `pending_permission_request = Some(...)` を設定し、`agent-session-state-changed` を pending 付きで emit する。
- **UI dialog は message part 依存**: `ChatSessionView.tsx:528` の `case "permission"` のみが `PermissionDialog` を描画する（実コードで確認済み）。`pending_permission_request` は reducer に保存されるが通常描画には使われていない。
- **`get_session` は pending を返さない**: `GetSessionResponse`（`session/mod.rs:929`）に `pending_permission_request` フィールドが存在しないことを確認した。`turn_phase` / pending queue / token usage は合成するが pending permission は返さない。frontend DTO（`useSessionStore.ts:112`）にも無い。
- **durable event は即 read model 投影でない**: permission part の durable event は `append_durable_part_events` → `append_session_event_without_projection`（`runtime/usecase.rs:3406` / `session/store.rs:487`）で記録され、その瞬間に message read model へ必ず投影されるとは限らない。UI の主経路は transient delta。
- **finalize は未解決を cancelled で閉じる**: `event_log/finalization.rs:28` で turn finalize 時に未解決 permission を `PermissionDecision::Cancelled` として閉じる。ログ上の `turn_interrupted` + `cancelled` は本症状（回答待ちになったが中断で閉じた）と整合する。

## スコープ

本 Spec は Issue #1376 の「必須修正」3 点と「推奨修正」4 点の双方に対応する。

必須修正:

- **①（backend read model）** `GetSessionResponse` に `pending_permission_request` を追加し、runtime state の `state.pending_permission_request` を初期ロード API（`get_session`）で返す。frontend DTO（`useSessionStore.ts`）と `convertRawGetSessionResponse` にも追加し、`dispatchSessionMeta` で `SET_PENDING_PERMISSION` を hydrate する。
- **②（UI fallback 描画）** message part に対象 request の `permission` part が存在しない場合に限り、backend-owned な pending permission を fallback として `PermissionDialog` で表示する。message part と pending state の双方が存在する場合は二重表示しない。
- **③（read model への即復元）** `PermissionRequested` に由来する pending permission を、session reload / 後からの session open でも message read model 上へ復元可能にする。実現手段（`PermissionRequested` だけ即 projection / persist するか、`get_session` が event log projection から未解決 permission を latest agent message へ合成するか）は design.md で決定する。

推奨修正:

- **④（delta 破棄の可観測化）** `agent-streaming-delta` が対象 message 未存在で捨てられた場合（`agentChatReducer.ts:525` / `:541`）に warn を出し、取りこぼしを診断可能にする。
- **⑤（回答待ち不可視の診断）** `WaitingPermission` が一定時間続き visible dialog が無い状況を検知できる診断イベントを出す。判定に必要な状態の所有は Rust 側に置く。
- **⑥（emit 抑止中の回答経路）** `stream_emit_suppressed`（`runtime/usecase.rs:2441`）中に `PermissionRequested` が来た場合、state-change fallback（②の pending permission 経路）だけで回答可能にする。
- **⑦（workflow step session の checkpoint 導線）** workflow step session でも pending human checkpoint を一覧・detail のどちらからでも開けるようにする。

## 非スコープ

- provider → `PermissionRequested` の**変換ロジック**の変更（Claude / Codex とも正常であることが確認済みのため対象外）。
- permission の auto-allow / interactive tool 判定（`claude/permission.rs`）そのものの変更。
- finalize 時の未解決 permission を `cancelled` で閉じる既存挙動の変更（従来どおり維持する。要求事項で回帰しないことを担保する）。
- `PermissionDialog` の UI レイアウト・回答フロー・`respond_permission` の意味論の変更（fallback 描画元を追加する配線を除く）。
- permission 以外の human checkpoint（承認/却下 review 等）の復元経路の変更。

## 要求事項

- **R1**: `get_session`（`GetSessionResponse`）が、runtime が `WaitingPermission` 中に保持する `pending_permission_request` を返すこと。backend runtime state が source of truth であり、Tauri 以外の将来 client surface からも同じ shape を読めること（①）。
- **R2**: frontend が `get_session` 応答の `pendingPermissionRequest` を DTO 変換・reducer hydrate（`SET_PENDING_PERMISSION`）で取り込むこと。domain decision を frontend に増やさず、backend が返した値を保持するだけであること（①）。
- **R3**: 対象 request の `permission` part が message parts に存在しない場合、UI が backend-owned な pending permission を fallback として `PermissionDialog` で表示し、ユーザーが回答できること（②）。
- **R4**: message parts の `permission` part と pending permission state が同一 request id で両方存在する場合、`PermissionDialog` が二重表示されないこと（②）。
- **R5**: `PermissionRequested` 後に UI が `agent-streaming-delta` を一切受け取れなくても、session reload / 後からの session open で pending permission を復元し dialog を表示・回答できること（③）。
- **R6**: 未解決 permission の finalize が従来どおり `cancelled` になる既存挙動を回帰させないこと。
- **R7**: pending permission の source of truth が Rust 側 runtime / read model にあり、frontend は表示のみに留まること。full-retention / frontend 再計算経路を新設しないこと。
- **R8**: `agent-streaming-delta` が対象 message 未存在で捨てられた場合に warn が出力され、取りこぼしが診断可能であること（④）。
- **R9**: `WaitingPermission` が一定時間継続し visible dialog が無い状況を検知できる診断イベントが出力されること。判定に必要な状態は Rust 側が所有すること（⑤）。
- **R10**: `stream_emit_suppressed` 中に `PermissionRequested` が発生しても、②の state-change fallback 経路だけでユーザーが回答でき、不可視停止しないこと（⑥）。
- **R11**: workflow step session でも pending human checkpoint を一覧・detail のどちらの導線からも開いて回答できること（⑦）。

## 受け入れ基準の概要

- **Rust test**
  - `get_session` が `WaitingPermission` 中の `pending_permission_request` を返す（R1）。
  - `PermissionRequested` 後、streaming delta を受けなくても session reload で pending permission を復元できる（R5）。
  - 未解決 permission の finalize が従来どおり `cancelled` になる（R6）。
- **frontend test**
  - `getSession` 応答の `pendingPermissionRequest` が reducer に hydrate される（R2）。
  - message part に permission が無く pending state だけある場合に `PermissionDialog` を表示する（R3）。
  - message part と pending state が両方ある場合に二重表示しない（R4）。
  - `agent-streaming-delta` の対象 message が state に無い場合の挙動が明示的にテストされる。
- **integration**
  - Claude `AskUserQuestion` で、session reload 後も dialog が表示され回答できる。
  - Codex `item/tool/requestUserInput` または `requestApproval` で、tab 移動後も dialog が表示され回答できる。
  - workflow step session で hidden / reopened のケースで pending human checkpoint を開いて回答できる（R11）。
- **推奨修正の検証**
  - `agent-streaming-delta` の対象 message が state に無い場合に warn が出る（R8）。
  - `WaitingPermission` 長期化かつ visible dialog 不在で診断イベントが出る（R9）。
  - `stream_emit_suppressed` 中の `PermissionRequested` を state-change fallback だけで回答できる（R10）。
- **品質ゲート**: `pnpm lint` / `pnpm test` / `pnpm build` / `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る。

## 仮定

- `state.pending_permission_request` が保持する情報（request id / body / tool 名など）は、`PermissionDialog` の描画と `respond_permission` 呼び出しに必要な `request` shape を過不足なく供給できる前提とする。転送 shape の具体は design.md で決める。
- ②の fallback 判定は「対象 request id と同じ `permission` part が message parts に無いときだけ pending を描画する」という規則で二重表示を避けられる前提とする。表示位置（`ChatSessionView` かその親か）は design.md で決める。
- ③の実現手段（即 projection / persist か、`get_session` の projection 合成か）は要求ではなく設計判断であり、R5 の性質（reload で復元できる）を満たせばどちらでもよい。design.md で決定する。
- 本症状の再現は「backend は `WaitingPermission`・回答待ちだが UI に dialog が無い」不可視停止であり、変換不正ではないという Issue 調査結論を前提とする（実コードで裏付け済み）。

## Open Questions

なし。
