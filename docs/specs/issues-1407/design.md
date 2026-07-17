# design: L6 resume 回復の統一（BackendSessionCleared 配線）

- Spec ID: issues-1407
- 対応 Issue: #1407 「[Agentチャット安定化] L6: resume 回復の統一（BackendSessionCleared 配線）」
- 位置づけ: milestone 84「Agentチャット安定化」／ Phase 0（監査 SD-1 / OB-8、ライフサイクル I9）
- 参照: `requirements.md` / `behavior.md` / 正本 vocabulary `specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-vocabulary.md`（§1299-1337 回復イベント型、§9.5 Local atomic event transaction、§1675 recovery narrative、§1775 移行表）/ lifecycle `agent-chat-ideal-lifecycle.md` I9（L101-105）

---

## 概要

backend session の resume 失敗を、Claude / Codex 双方で同一の回復経路（I9）へ統一する。

現状の非対称:

- **Claude**: resume mismatch を `AgentRuntimeEvent::SessionEstablished { resume: Mismatch }` として event pump が受信し、`handle_resume_mismatch`（`usecase.rs:2095-2145`）が runtime を閉じ、実行中 turn を pending queue 先頭へ戻し、resume metadata を消して新規 backend で自動再開する。ただし**無言**（通知なし）で、requeue 時に `editor_context` が脱落する（OB-8）。
- **Codex**: thread/resume の失敗を `AgentRuntimeEvent::BackendSessionCleared + Fatal` へ変換する（`codex/convert.rs:59-67`）が、この event は `open_once` の起動フェーズでしか emit されず、`open_session` が `wait_for_thread_id` の `Err` で失敗するため `take_events` に到達せず、**受信者のいないチャネルごと drop される**（`codex/session.rs:59-76`, `442-465`）。結果、`BackendSessionCleared` を処理する pump 経路（`usecase.rs:2432-2450`）に決して到達せず、Codex は backend thread 消失後に恒久死する。`domain/agent_session/gateway.rs:62-64` の `#[allow(dead_code)]` がこの未配線を明記している。

本設計は次を行う。

1. Codex の resume 失敗を、起動フェーズ失敗ではなく**回復可能なシグナル**として usecase へ届け、両 backend を共通の回復ルーチンへ集約する（R1 / R2 / AC1 / AC4）。
2. 回復時にユーザーへ「backend セッションを作り直したため文脈は引き継がれません」を通知する（Claude 無言復旧を含む）（R3 / R4 / AC1 / AC2）。
3. resume mismatch requeue の `editor_context` 脱落を解消する（R5 / OB-8 / AC3）。
4. 回復を単一 `recovery_id` で相関付け、resume metadata clear・回復開始・（configuration/Goal）reactivation・回復完了を定義された境界で確定し、部分適用状態を公開しない（R6 / R7 / AC5）。**#1407 は Phase 0 のため、configuration aggregate（#1397 Phase 3）・Goal aggregate（#1449 Phase 4）が未導入。本 issue は回復配線と相関スキャフォルドまでを担い、aggregate 依存の完全 reactivation は後続 phase が拡張する（下記「範囲確定」参照）。**

---

## 変更対象

| ファイル | 変更 |
|---|---|
| `domain/agent_session/gateway.rs` | `AgentRuntimeEvent::BackendSessionCleared` の `#[allow(dead_code)]` 除去。`AgentBackendError` に resume 失敗を表す variant（後述）を追加。 |
| `infrastructure/agent_session/codex/session.rs` | resume 失敗を「起動失敗（Fatal）」ではなく回復シグナルとして `open_session` 呼び出し元へ届ける（`open` / `open_once` / `wait_for_thread_id` の分岐）。 |
| `infrastructure/agent_session/codex/convert.rs` | 確立済みセッションへの turn/start が dead-thread エラーを返す経路でも resume 失敗を検出（現状は `METHOD_TURN_START` の一般 Error part 化に落ちている）。 |
| `usecase/agent_session/runtime/usecase.rs` | 共通回復ルーチン `recover_backend_session` の新設。`handle_resume_mismatch` を回復ルーチンへ寄せて通知追加。`start_turn_for_session`（`:1446`）と `send_message`（`:415`）の `editor_context` 保全。runtime open 経路（`open_runtime_for_session` / `ensure_runtime` / `start_next_queued_turn`）で resume 失敗を捕捉して回復ルーチンへ接続。 |
| `usecase/agent_session/runtime/usecase.rs`（型） | `From<EditorContext> for AgentEditorContext` の逆変換追加、または `TurnStartPayload.editor_context` を `AgentEditorContext` で保持（後述）。 |
| `usecase/agent_session/event_log/events.rs` | 回復イベント（`BackendSessionRecoveryStarted` / `SessionConfigurationReactivated` / `SessionGoalReactivated` / `BackendSessionRecoveryCompleted`）を `AgentSessionEvent` に追加（範囲は Q1）。 |
| `usecase/agent_session/session/store.rs` / `SessionMeta` | `provider_session_generation: u64` の追加。回復イベントのバッチ append（範囲は Q1）。 |
| `domain/agent_session/value_objects/system_notification_type.rs` | 通知用 `SessionRecovery` variant 追加（Notice 手段。詳細は「エラー処理／通知」）。 |
| 各 `#[cfg(test)]` | 統合テスト追加（R8 / AC6）。 |

**非変更**: `requirements.md` / `behavior.md`、backend CLI（Claude Code / Codex）側の resume 実装、回復通知の恒久 UI、MessagePart の domain/usecase 二重定義（G-1）。

---

## アーキテクチャと責務分割

Rust がロジックを所有する原則に従い、**回復判断・トランザクション制御は usecase/domain に置き、backend adaptor は resume 失敗を「シグナル」するだけ**とする（vocabulary/lifecycle が回復 orchestration を Rust usecase に置く方針と一致）。frontend は通知パートを表示するのみ。

### 回復シグナルの到達点を統一する

resume 失敗は 2 つの構造的に異なる地点で発生する。

- **Claude mismatch**: `open_session` は成功し、`take_events` の stream に `SessionEstablished { resume: Mismatch }` が流れる（pump が受信）。
- **Codex thread 消失**: `open_session` 自体が失敗する（resume 要求への app-server error）。または確立済みセッションへ turn/start した際の error。

両者を 1 本化するため、Codex の resume 失敗を **backend error の型で表現**し、runtime open を行う usecase 側で捕捉する。

- `domain/agent_session/gateway.rs` の `AgentBackendError` に
  `BackendSessionLost { requested_resume_id: String }` を追加する。
- `codex/session.rs`: `open`（リトライループ）で `wait_for_thread_id` が startup error を返したとき、その startup error が **resume 要求に対するもの**（`requested_resume_id.is_some()` かつ app-server が resume を拒否）であれば `AgentBackendError::BackendSessionLost { .. }` を返す。それ以外の startup error は従来どおり `Other` / `StartupTimeout`。
  - resume 起因かどうかは、`read_loop` が `BackendSessionCleared` を emit する条件（`convert.rs:59-67` の `startup_request_id == id && requested_resume_id.is_some()`）と同一判定を `startup_error` に反映して伝える（例: `CodexRuntimeState` に `resume_rejected: bool` を持たせ、`wait_for_thread_id` が `startup_error` 検出時にこれを見る）。
- **確立済みセッション**への turn/start が dead-thread エラー（"not found" 等）を返す経路（`convert.rs` の `source_method == METHOD_TURN_START` 分岐、`:70-82`）でも、error が thread 消失に該当する場合は `BackendSessionCleared` を event stream へ流す。これは pump が受信できる（take_events 済み）ため、そのまま共通回復へ入る。
  - dead-thread 判定は Codex app-server の error（code / message）を wire 層で分類する。判定不能なものは従来どおり turn Error part+Failed（挙動退行なし）。

> 仮定: Codex thread 消失後の「次送信」は、実運用では live runtime が無い状態からの `open_runtime_for_session`（resume 付き）で始まる。したがって主経路は `open_session` の `BackendSessionLost` 捕捉。live runtime が残ったまま dead-thread へ turn した場合の副経路として、上記 turn/start error → `BackendSessionCleared` event 化を用意する。

### 共通回復ルーチン `recover_backend_session`

usecase に単一のエントリを新設し、Claude mismatch・Codex `BackendSessionLost`・Codex `BackendSessionCleared` の 3 到達点すべてをここへ集約する。

```
recover_backend_session(ctx, session_id, reason) -> RecoveryOutcome
  0. recovery_id を採番。old_provider_session_generation を取得。
  1. [Start commit] resume metadata clear + BackendSessionRecoveryStarted を確定
     （configuration/Goal を「回復中」として block 開始）。
  2. 実行中 turn があれば editor_context を保持したまま pending queue 先頭へ requeue。
  3. 新規 establish（resume=None で open_runtime_for_session）。
  4. [Reactivation] SessionConfigurationReactivated（新 generation・consumed observation）
     + SessionGoalReactivated（GoalReactivationOutcome 網羅）を確定。
  5. [Complete commit] BackendSessionRecoveryCompleted を確定 → Synced/公開。
  6. 回復 Notice をチャットへ emit。
  7. queue drain を再開（送信内容を新セッションで処理）。
```

- Claude 経路: `apply_runtime_event` の `SessionEstablished { Mismatch }` 分岐（`:2396-2400`）は `handle_resume_mismatch` の代わりに `recover_backend_session` を呼ぶ。`handle_resume_mismatch` の既存処理（runtime close・requeue・metadata clear・state Active 化・drain）は回復ルーチンのステップへ吸収する。
- Codex 経路: `open_runtime_for_session` / `ensure_runtime` が `AgentBackendError::BackendSessionLost` を返したら回復ルーチンへ。`BackendSessionCleared` event（副経路）は `apply_runtime_event`（`:2432`）で受信し回復ルーチンへ。
- pump の `SessionEstablished`（Resumed/NotRequested）と `BackendSessionCleared` の既存 metadata 更新（`:2402-2450`）は、回復ルーチンが所有する commit と重複しないよう整理する（回復中は pump 側の単独 metadata 更新を抑止し、回復ルーチンの commit に一元化）。

### editor_context 保全（OB-8）

現状 `send_message` が `req.editor_context`(`AgentEditorContext`) を `EditorContext`(domain) へ即時変換し（`:415`）、`start_turn_for_session` が `QueuedTurnInput::new` の editor_context に `None` を渡す（`:1446`）。requeue 時に queue 側 `editor_context` が唯一の情報源（`:3521`）になるため脱落する。

- `TurnStartPayload.editor_context` を `Option<AgentEditorContext>` で保持する（`EditorContext` への変換は `runtime.start_turn` 直前に一度だけ行う）。**または** `From<EditorContext> for AgentEditorContext` の逆変換を追加する。前者を採用（早期変換で情報を落とさない・二重定義追加を避ける）。
- `start_turn_for_session:1446` は `payload.editor_context.clone()`（`AgentEditorContext`）を `QueuedTurnInput::new` に渡す。これで requeue → `start_next_queued_turn`（`:3521`）が `queued.editor_context` から復元でき、Codex の additionalContext ワイヤ送信・Claude の system prompt 再構築の双方でエディタ状態が保たれる。
- `mentions` / `images` は既に保持されているため、`editor_context` を揃えることで対称になる（behavior「保全は対称」）。

---

## データモデルまたは型

### 回復イベント（正本 vocabulary §1299-1337 準拠）

`AgentSessionEvent`（`event_log/events.rs`）へ追加する（フィールドは vocabulary を subset で採用）。

- `BackendSessionRecoveryStarted { recovery_id, old_provider_session_generation, reason, at }`
- `SessionConfigurationReactivated { recovery_id, provider_session_generation, consumed_observation_id: Option<String>, at }`
- `SessionGoalReactivated { recovery_id, outcome: GoalReactivationOutcome, provider_session_generation, restoring_turn_id: Option<String>, consumed_observation_id: Option<String>, at }`
- `BackendSessionRecoveryCompleted { recovery_id, provider_session_generation, at }`

`GoalReactivationOutcome`（vocabulary §1315-1320）:
`NoCurrentGoal | TerminalGoalUnchanged { .. } | Restored { .. } | ObservedUnchanged { .. }`。

> 仮定: `SessionConfigurationReactivated.configuration`（vocabulary の `AgentEffectiveConfiguration`）は、agent_session に configuration aggregate が存在しないため本 issue では持たない（Q1 で確定）。現状の「configuration」は resume metadata＋selected model / permission mode / plan mode の再適用に相当し、新 provider session 確立時に `open_runtime_for_session` が `session.selected_model` 等から再導出する（旧 effective 値の無検証流用をしない＝R7 の configuration 側を満たす）。

### provider session generation

`SessionMeta` に `provider_session_generation: u64`（default 0）を追加。`SessionEstablished`（Resumed/NotRequested）成功時に increment。回復開始で `old_provider_session_generation` を捕捉し、reactivation/completed に新値を載せる。旧 generation の effective 値を新 generation へ流用しないための識別子（R7）。

### editor_context

- `EditorContext`（domain, `value_objects/editor_context.rs`）と `AgentEditorContext`（usecase, `usecase.rs:81`）は現状のまま。
- `TurnStartPayload.editor_context: Option<AgentEditorContext>` へ変更（現 `Option<EditorContext>`）。
- `QueuedTurnInput.editor_context` は `Option<AgentEditorContext>` のまま（変更不要）。

### 通知パート

`SystemNotificationType`（現状 `Compaction` のみ）へ `SessionRecovery` を追加。回復 Notice は
`MessagePart::SystemNotification { notification_type: SessionRecovery, status: "recovered", label: "backend セッションを作り直したため文脈は引き継がれません", detail: None, hook_id: None }`
として emit する。これを behavior の「Notice part」とみなす。Notice を用いず Error part を採る場合は `MessagePart::Error { content: <同文言> }`（暫定手段）。

---

## 処理フロー

### Codex: 確立後 thread 消失 →次送信で復活（AC1 / R2）

1. 会話進行中に backend thread が消失（codex home 変更・rollout 削除・GC）。
2. 利用者が次送信 → live runtime 無し → `open_runtime_for_session(resume=旧thread_id)`。
3. `open` 内 `open_once` が thread/resume を送信 → app-server error → `read_loop` が resume 拒否を検出し `startup_error`＋`resume_rejected` を設定。
4. `wait_for_thread_id` が resume 起因 startup error を検出 → `open` が `AgentBackendError::BackendSessionLost { requested_resume_id }` を返す。
5. usecase が捕捉 → `recover_backend_session(reason=BackendSessionLost)`。
6. Start commit（metadata clear＋RecoveryStarted）→ resume=None で再 open → 新 thread 確立（`SessionEstablished { NotRequested }`, generation+1）→ Reactivation → Complete commit → Notice emit → queue drain。
7. 送信内容は新セッションで処理され、セッションは Error に留まらない。以後の送信は新 thread を使い、死んだ thread への resume を繰り返さない（behavior「後続送信は死んだ thread を再利用しない」）。

### Claude: resume mismatch →通知付き自動復旧（AC2 / R4）

1. `open_session` 成功、pump が `SessionEstablished { Mismatch { actual } }` を受信（`:2396`）。
2. `recover_backend_session(reason=ResumeMismatch)` を呼ぶ。
3. runtime close・実行中 turn を editor_context 保持で requeue・metadata clear（Start commit）→新規 establish（NotRequested）→ Reactivation → Complete → Notice emit → drain。
4. behavior「文脈が静かに消える（通知なし復旧）ことはない」を満たす。

### editor_context 保全（AC3 / R5）

1. 実行中 turn が editor_context（アクティブファイル・選択範囲）を持つ。
2. mismatch で `recover_backend_session` が `current_turn_input`（editor_context 保持済み）を queue 先頭へ requeue。
3. `start_next_queued_turn` が `queued.editor_context`（= 元の `AgentEditorContext`）から `EditorContext` を復元し `runtime.start_turn` へ（`:3521`）。
4. Codex は additionalContext としてワイヤ送信、Claude は system prompt 再構築へ反映。

### 回復の相関・順序・部分適用非公開（AC4 / AC5 / R6）

- 全回復イベントは同一 `recovery_id` を持つ。
- 公開順序は `BackendSessionRecoveryStarted → SessionConfigurationReactivated / SessionGoalReactivated → BackendSessionRecoveryCompleted`。
- `BackendSessionRecoveryCompleted` 確定まで、途中状態（回復中の configuration / Goal / セッション状態）は Synced/公開しない。
- 回復中はそのセッションの configuration / Goal 変更要求を保留（block）し、回復確定後に通常操作へ戻す。
- `BackendSessionCleared` は production 経路（上記 Codex 副経路・Claude mismatch・Codex `BackendSessionLost`）から到達可能となり dead code が解消（AC4）。

### Goal reactivation の網羅（AC5 / R7）

- agent_session に Goal aggregate が無い現状では、回復時の Goal 状態は常に「無し」。したがって `SessionGoalReactivated { outcome: NoCurrentGoal }` を必ず append する（記録漏れなし）。
- Goal aggregate が導入された後（#1449）は `TerminalGoalUnchanged / ObservedUnchanged / Restored` を網羅し、`Restored` の strategy が `StartsTurn` の場合は `TurnStarted`（evidence 付き）を最終 transaction へ含め early stream を buffer する。本 issue では拡張点（`GoalReactivationOutcome` の分岐と最終 commit へ turn を織り込む口）を用意し、実挙動は `NoCurrentGoal` に閉じる。

---

## エラー処理

- **回復自体の失敗**（新規 establish が再度失敗、reactivation/commit 失敗）: 部分適用を公開せず、セッションを `Error` にし、reconciliation 相当（現状は `SessionState::Error`＋Error part）へ送る。behavior「結果不明は reconciliation へ送る」に対応。回復イベントの Complete が append できない場合は Synced にしない。
- **resume 拒否でない startup error / timeout**: 従来どおり `Other` / `StartupTimeout`。回復ルーチンへ入れない（挙動退行なし）。
- **dead-thread 判定不能な turn error**: 従来の turn Error part + `Failed` 完了を維持（behavior のリグレッション否定シナリオは「生 JSON-RPC エラーを毎回表示して復旧不能」を否定するもので、判定可能な thread 消失に限り回復へ接続する）。
- **通知経路未整備時**: `SessionRecovery` Notice を採らず Error part を暫定手段とする（behavior の Scenario Outline: Notice 経路あり→Notice part / なし→Error part）。
- **generation 巻き戻り / 二重回復**: 同一 `recovery_id` の idempotent 再適用のみ許可し、回復中の再入は block する。

---

## テスト方針

Rust の該当 module 内 `#[cfg(test)]`。外部プロセスは実行しない。

### 統合テスト（R8 / AC6）

- **Codex thread 消失からの復活＋通知**（AC1）: fake backend で「resume 要求に対し error を返し、resume=None なら新 thread を確立する」挙動を再現（codex home / rollout 差し替え相当をテスト内 fixture で表現）。次送信でセッションが Active に復活し、`SessionRecovery` Notice（または Error part）が出て、送信が新 thread で処理されることを検証。既存 `usecase.rs:7474-7503`（`BackendSessionCleared` emit → agent_session_id None）を拡張。
- **後続送信が死んだ thread を再利用しない**（behavior）: 回復後の 2 通目が resume=None（新 thread）で送られることを検証。
- **Claude resume mismatch の通知**（AC2）: 既存 `usecase.rs:7380-7443`（mismatch → agent_session_id None）を拡張し、Notice/Error part が出ることを検証。
- **リトライ turn の editor_context 保全**（AC3）: editor_context 付き turn を mismatch でリトライし、requeue 後の `runtime.start_turn` に渡る `TurnInput.editor_context` が元と同一・`None` でないことを fake backend で検証。`mentions` / `images` と対称であることも確認。
- **回復の相関・順序**（AC5）: `recovery_id` が全イベントで一致し、`Started → (Config/Goal)Reactivated → Completed` の順で append され、Completed 前に Synced 公開が起きないことを検証。
- **Goal 網羅**（R7）: 現状 `NoCurrentGoal` が必ず 1 件 append されることを検証。

### 単体

- `From<EditorContext>`↔`AgentEditorContext`（採用方式）の往復・逆変換の正しさ。
- `convert.rs`: 確立済み turn/start の dead-thread error → `BackendSessionCleared` 化の分類（判定可能／不能）。
- `wait_for_thread_id`: resume 拒否 → `BackendSessionLost`、非 resume startup error → `Other`。

### 品質ゲート

`cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` を green（AC6）。frontend に通知表示追加が及ぶ場合は `pnpm lint` / `pnpm test`。

---

## リスクと代替案

- **R-1（本質・範囲確定済み）: 回復トランザクション基盤は後続 phase 所有**。vocabulary §9.5 の `LocalEventTransactionStore`、configuration/Goal aggregate、`sync_state` は agent_session に未実装で、**configuration aggregate は #1397（Phase 3）、Goal aggregate は #1449（Phase 4、#1397 依存）が所有**。#1407 は **Phase 0** のため、これらを本 issue で新設するのは phase 順序に反する。移行表（§1775）は `BackendSessionRecovery*` を #1397/#1407/#1449 の共同所有とし、#1449 本文は「backend recovery にも同じ evidence protocol を使う」と明記＝回復時の完全な Goal/configuration reactivation は後続 phase で層状に完成させる設計。
  - **採用（範囲確定）**: 本 issue は SD-1/OB-8 の回復配線・通知・editor_context 保全を完遂し、回復イベントは event log への最小バッチ append で相関・順序・部分適用非公開を成立させる。`provider_session_generation` を `SessionMeta` に追加。configuration reactivation は「resume metadata＋selected model/permission の再適用（旧値無検証流用なし）」相当、Goal は aggregate 未導入のため `NoCurrentGoal` に閉じる。cross-aggregate の完全 atomic transaction・`AgentEffectiveConfiguration` reactivation・Goal restore（StartsTurn）は #1397/#1449 が後続で拡張する（本 issue は `GoalReactivationOutcome` 分岐と最終 commit へ turn を織り込む拡張点だけ用意）。
  - 却下: #1407 で `LocalEventTransactionStore` と configuration/Goal aggregate reactivation を全新設する案は、Phase 3/4 の #1397/#1449 と大きく重複し phase 順序・非スコープ宣言に反する規模膨張。
- **R-2: backend adaptor への回復ロジック混入**。回復判断は usecase に置き、adaptor は `BackendSessionLost` / `BackendSessionCleared` の**シグナル**に限定する。代替（adaptor 内で自動 fresh-thread フォールバック）は無言復旧に戻り通知を失うため不可。
- **R-3: 確立済み live runtime での dead-thread 検出漏れ**。Codex error 分類が不十分だと副経路（turn/start error → BackendSessionCleared）が発火しない。主経路（`open_session` の `BackendSessionLost`）で大半を捕捉し、副経路は分類可能な error に限定、判定不能は従来挙動維持で退行を防ぐ。
- **R-4: pump 側 metadata 更新と回復 commit の二重更新**。回復中は pump 単独の resume metadata 更新を抑止し、回復ルーチンへ一元化する。

---

## 仮定

- Spec ディレクトリは `docs/specs/issues-1407/`。
- ロジックは全て Rust（usecase / domain / infrastructure）。frontend は通知パート表示のみ。
- 回復通知文言は日本語「backend セッションを作り直したため文脈は引き継がれません」。Notice 手段は `SystemNotificationType::SessionRecovery`、未整備時は Error part 暫定。
- thread 消失の模擬は外部プロセスを使わず fake backend / fixture 差し替えで再現。
- 共有回復イベント型は vocabulary §1299-1337 の定義に subset で準拠。`configuration` フィールドや Goal aggregate 依存部分は #1397/#1449 の aggregate 導入後に拡張し、本 issue では回復経路が必要とする最小限（相関・順序・generation・NoCurrentGoal）に留める（Q1 の確定に従う）。
- Codex の resume 起因失敗判定は、既存の `convert.rs:59-67`（`startup_request_id` 一致かつ `requested_resume_id` あり）と同一意味を `wait_for_thread_id` へ伝播して行う。

---

## Open Questions

なし。

（解消記録）R6/R7/AC5 の実装深度は、GitHub Issue の phase 順序で確定した。#1407 は **Phase 0**、configuration aggregate を持つ #1397 は **Phase 3**、Goal aggregate を持つ #1449 は **Phase 4（#1397 依存）**。#1449 は「backend recovery にも同じ evidence protocol を使う」と明記しており、回復時の完全な configuration/Goal reactivation は後続 phase が層状に完成させる設計である。したがって Phase 0 の本 issue は上記「範囲確定（採用）」のとおり、SD-1/OB-8 の回復配線・通知・editor_context 保全＋回復相関スキャフォルド（`recovery_id`・`provider_session_generation`・回復 4 イベントの順序付き最小バッチ append・`NoCurrentGoal`）までを担い、`LocalEventTransactionStore` と aggregate reactivation の新設は #1397/#1449 に委ねる。
