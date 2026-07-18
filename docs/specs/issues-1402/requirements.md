# Requirements

対象 Issue: #1402「[Agentチャット安定化] L1: 停止（interrupt）の信頼性保証」

マイルストーン: [84] Agentチャット安定化（Phase 0 / 依存なし・即着手可）

解消する監査問題: **OB-1**, **SD-2**, **OB-5**

正本ドキュメント:

- 問題インベントリ: `specs/milestone-84-agent-chat-stabilization/agent-chat-instability-audit.md`（OB-1 / SD-2 / OB-5）
- ライフサイクル: `specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-lifecycle.md`（不変条件 I5、状態機械の StartingTurn / Interrupting 行、判断 L-D2 / L-D5）
- UI 表示: `specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-presentation.md`（queue 行 = Paused）

## Type

不具合修正（backend 挙動の信頼性保証）。turn の停止（interrupt）を backend に依らず確実に終端させ、停止後に queue が自動再開しないようにする。挙動変更を伴うが、正規化語彙・schema 変更（Phase 1 以降）は含まない。

## 背景と目的

### 背景

Agent チャット（Claude / Codex セッション）の停止操作には、backend 間で信頼性の非対称がある。

- **Claude**: `interrupt()` が常に interrupt control request を CLI に書き込み、さらに約 10 秒（`ABORT_SYNTHESIS_DELAY`）の synthetic abort タイマで backend 無応答時に `TurnCompleted(Interrupted)` を合成する。結果、Stop は最悪 10 秒で必ず turn を終端する。
- **Codex**: `interrupt()` は `thread_id` と `turn_id` の両方が設定済みであることを要求する。`turn_id` は app-server の `turn/started` 通知でのみ設定されるため、「Releash が `turn/start` を書き込んでから `turn/started` を処理するまで」のウィンドウで押された Stop は、何も送信されず `Ok(())` を返して**無言で握りつぶされる**。合成 abort タイマも存在せず、usecase 層にもフォールバックが無い。

さらに frontend は Stop 押下時に楽観的に `interrupting=true` を立て、backend が `Ok` を返すため成功扱いになり、フラグは `turnPhase` が idle になるまで解除されない。Stop ボタンは `disabled={isInterrupting}` になるため、一度握りつぶされると turn が自然終了するまで**再押下する手段が UI に存在しない**。

加えて、`apply_runtime_event` は `TurnCompleted` を結果種別（Completed / Failed / Interrupted / Crash）に関わらず `actions.drain()` し、ユーザーが Stop で turn を abort した直後でも pending queue のメッセージが**即座に次 turn として実行開始される**（「止めたのに続く」）。

### 対応する監査問題

- **OB-1（high）**: Codex の interrupt は `turn_id` 未取得ウィンドウで無言 no-op になり、以後の停止操作も frontend が握りつぶす。
- **SD-2（medium）**: Stop の信頼性が backend で違う（Codex は `turn/started` 前の Stop を握りつぶし、フォールバックも無い）。OB-1 と同一根本原因（`turn_id` 未取得ウィンドウ）。
- **OB-5（medium）**: ユーザーの interrupt 直後に pending queue が無条件 drain され、次の queue メッセージが即座に実行開始される。

### 目的

1. **停止を常に受理する**: turn 開始直後（`turn_id` 未取得ウィンドウ）や backend ハング時でも Stop を受け付け、必ず terminal 状態へ着地させる。
2. **強制終端を両 backend の共通保証にする**: backend interrupt が不可能・無応答でも、最悪 10 秒でランタイムが turn を強制終端する（I5）。
3. **停止後の queue 自動再開を止める**: interrupt 時に queue を paused にし、再開はユーザーの明示操作に限定する（OB-5 / L-D5）。
4. **frontend の握りつぶしを解消する**: interrupt 中の再押下を強制終端要求として受け付ける。

## スコープ

L1 = interrupt（停止）の信頼性保証に限定する。

### 対象範囲

1. **interrupt 予約（Codex）**
   - `codex/session.rs` の `interrupt` で `turn_id` 未取得（`turn/started` 未受信）なら予約フラグを立て、`turn/started` 受信時に即 interrupt を送出する。
   - `StartingTurn`（`TurnStartRequested` 後 / ack 前）の Stop も常に受理し、`TurnStartState::InterruptRequested` へ遷移させる。late ack を通常の `TurnStarted` commit へ流さず、取得した provider turn を interrupt して `Interrupted` で finalize するか、ack / interrupt 結果不明なら同じ request id の TurnStart reconciliation へ畳む。

2. **強制終端の共通保証（両 backend）**
   - Claude の synthetic abort（約 10 秒）と同等の猶予タイマを runtime 共通（または codex session）に実装し、backend の停止が確認できなければ `Interrupted{Timeout}` で強制 finalize する。
   - I5「Stop は常に最悪 10 秒で終端する」を Claude / Codex 両方の保証に昇格させる（L-D2）。

3. **frontend の再押下受付**
   - interrupt 中の再押下を握りつぶす分岐（監査 OB-1 の frontend 側）を解消し、再押下を強制終端要求として受け付ける。

4. **queue paused（OB-5）**
   - interrupt 時の無条件 drain を廃止し `paused=true` にする（確定④ / L-D5）。paused は event log から projection して再起動後も復元する。
   - 再開は明示操作（queue chips の再開ボタン、暫定は既存 UI で可）。

### 最終設計ゲート追補（2026-07-15、本 ISSUE で満たす）

- Stop 受理時に `TurnInterruptRequested` を backend I/O 前に durable commit する。
- 起点や現在の item 有無に関係なく `QueuePaused` も同じ transaction へ含め、終端前 crash でも自動 drain しない（既に paused なら idempotent）。
- `StartingTurn`（`turn/start` ack 前）の Stop を明示遷移として扱い、late ack を通常の `TurnStarted` commit へ流さない。
- late ack で turn id 取得済みなら interrupt / finalize、ack / interrupt 結果不明なら同じ request id の TurnStart reconciliation へ送る。

## 非スコープ

- **queue の永続化と取消の完全化**: pending queue 本体の永続化は L3（#1404）で行う。本 ISSUE では pause/resume 状態のみを durable projection として扱う。
- **queue 再開 UI の作り込み**: 明示的な再開操作の受け皿は用意するが、専用 UI の作り込みは行わず暫定は既存 UI で可とする。
- **ユーザー入力の無損失保証（steer / stalled 送信）**: OB-2 は L2（#1403）の対象。
- **close / quit 時の finalize**: L4（#1405）の対象。
- **正規化語彙・schema 進化・typed wire 置換**: F 群・S 群（Phase 1 以降）の対象。
- **stall watchdog の挙動変更**: 停止の強制終端は stall（非終端シグナル）とは別経路とし、stall 診断の全 phase 化（L9 #1410）はスコープ外。

## 要求事項

### R1: 停止操作は常に受理される（I5）

- Stop 操作は、turn の phase（`StartingTurn` / `Streaming` / `WaitingPermission` / `Interrupting`）に関わらず常に受理する。
- backend への interrupt 送出が不可能・無応答でも、猶予後に必ず terminal 状態へ着地する。

### R2: Codex interrupt 予約

- `turn_id` 未取得ウィンドウで受けた Stop は無言 no-op にせず、予約として保持する。
- `turn/started` 受信時に、予約された interrupt を即送出する。
- `StartingTurn`（ack 前）の Stop は明示遷移として扱い、late ack を通常の `TurnStarted` commit に流さない。取得できた provider turn は interrupt / finalize し、結果不明なら TurnStart reconciliation へ畳む。

### R3: 強制終端の共通猶予タイマ（L-D2）

- Claude / Codex 両 backend で、Stop 受理後に backend の停止確認が取れない場合、最悪 10 秒（既存 Claude synthetic abort の実績値）で runtime が turn を `Interrupted{Timeout}` として強制 finalize する。

### R4: Stop 受理の durable commit（設計ゲート追補）

- Stop 受理時、`TurnInterruptRequested` と `QueuePaused` を backend I/O 前の同一 local atomic batch で durable commit する。
- 終端前に crash しても自動 drain しない。既に paused なら idempotent とする。

### R5: queue の paused 化（OB-5 / L-D5）

- interrupt 時、pending queue を無条件 drain せず `paused=true` にする。
- 停止直後に次の queue メッセージが自動実行開始されない。
- 再開はユーザーの明示操作（CAS 付き `QueueResumed` 相当）だけで行い、interrupt / backend / frontend の停止経路は queue を勝手に再開しない。

### R6: frontend の再押下受付

- interrupt 中の Stop 再押下を握りつぶさず、強制終端要求として backend へ伝える。
- Stop が握りつぶされたことによって turn の自然終了まで再送手段が失われる状態を作らない。

## 受け入れ基準の概要

Issue #1402 の受け入れ基準に準拠する。

- [ ] Codex で送信直後の Stop が最悪 10 秒で turn を終端する。
- [ ] Stop 後に queue のメッセージが自動実行されない。
- [ ] 停止ボタンの再押下が無視されない。
- [ ] lifecycle シナリオ表「ユーザー Stop」の保証（最悪 10s で Idle・queue は paused・入力欄 / queue は無損失）を満たす統合テストがある。

## 仮定

- **A1（spec-id）**: 本 Spec ディレクトリの識別子は、既存 `docs/specs/` の命名慣習に合わせて `issues-1402` とする。
- **A2（durable commit の範囲）**: 設計ゲート追補の「durable commit」は event log への append と projection 復元を指す。pending queue 本体の永続化（L3 #1404）はスコープ外だが、`QueuePaused` / CAS 付き `QueueResumed` は本 ISSUE で復元可能にする。
- **A3（10 秒の値）**: 強制終端猶予は既存 Claude `ABORT_SYNTHESIS_DELAY` の 10 秒を共通値として踏襲する（L-D2）。turn ごとの可変化は行わない。
- **A4（再開 UI）**: queue 再開の明示操作は暫定として既存 UI（queue chips）で受ける。専用 UI 追加は行わない。
- **A5（ロジック配置）**: 予約・猶予タイマ・queue paused・durable commit のロジックは Rust（infrastructure / usecase 層）に置き、frontend は再押下受付と表示に限定する（プロジェクトの Rust-first 原則）。

## Open Questions

なし。
