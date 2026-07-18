# Design

対象 Issue: #1402「[Agentチャット安定化] L1: 停止（interrupt）の信頼性保証」

関連: `requirements.md` / `behavior.md` / マイルストーン84 `agent-chat-ideal-lifecycle.md`（不変条件 I5、判断 L-D2 / L-D5）

本書は requirements（R1〜R6）と behavior の観測可能な保証を、現行コードベースの実装経路に落とし込む。解消する監査問題は OB-1 / SD-2 / OB-5。

## 概要

停止（interrupt）の信頼性は現状 backend 間で非対称である。根本原因は 3 つに分かれる。

1. **Codex interrupt の無言 no-op**（OB-1 / SD-2）
   `infrastructure/agent_session/codex/session.rs` の `interrupt()`（215-238 行）は `thread_id` と `turn_id` の両方が `Some` でないと `Ok(())` を返して何も送らない。`turn_id` は app-server の `turn/started` 通知を read_loop（370 行）が処理して初めて設定される。この「送信直後〜`turn_id` 取得前」ウィンドウで押された Stop は握りつぶされる。予約も猶予タイマも無い。

2. **強制終端タイマが Claude だけにある**（SD-2 / L-D2）
   Claude は `interrupt()` で `spawn_abort_synthesis_timer`（`claude/session.rs` 496-535 行）を起動し、`ABORT_SYNTHESIS_DELAY = 10s`（30 行）後に backend が無応答なら合成 `TurnCompleted(Interrupted{Abort})` を emit する。Codex にも usecase 層にも同等の保証が無い。

3. **interrupt 後に queue が無条件 drain**（OB-5 / L-D5）
   `usecase/agent_session/runtime/usecase.rs` の `apply_runtime_event`（2370 行）は `TurnCompleted` を結果種別（Completed / Failed / Interrupted / Crash）に関わらず `complete_turn` 後に `actions.drain()`（2542-2546 行）する。ユーザー Stop 直後でも pending queue が次 turn として即実行される。

さらに frontend は `useAgentChat.ts` の `interrupt`（754-766 行）で `if (interruptingRef.current[sessionId]) return;`（757 行）により再押下を握りつぶし、`MessageInput.tsx` は `disabled={isInterrupting}`（805 行）で停止中の再押下を不能にする。一度握りつぶされると turn 自然終了まで再送手段が無い（OB-1 frontend 側 / R6）。

本設計は次の 4 系統で解消する。

- **Codex interrupt 予約**（R2）: `turn_id` 未取得ウィンドウの Stop を予約フラグとして保持し、`turn/started` 受信時に即送出する。
- **共通強制終端 watchdog**（R3）: usecase 層に backend 非依存の abort watchdog を置き、Stop 受理後 10s 以内に必ず `Interrupted{Timeout}` へ着地させる。
- **queue paused**（R5）: interrupt 受理時に `queue_paused` を立て、`QueuePaused` / CAS 付き `QueueResumed` を projection して再起動時に hydrate する。`TurnCompleted` 時の drain を条件化し、再開はユーザー明示操作の usecase だけが行う。
- **durable commit と frontend 再押下**（R4 / R6）: Stop 受理を backend I/O 前に event log へ atomic に append し、frontend の握りつぶし分岐を撤去する。

## 変更対象

### backend（Rust）

- `src-tauri/src/infrastructure/agent_session/codex/session.rs`
  - `CodexRuntimeState` に interrupt 予約フラグを追加。
  - `interrupt()` を「`turn_id` 未取得なら予約」へ変更。
  - `read_loop` に write 用 handle を渡し、`turn_id` 確定時に予約 interrupt を送出。
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs`
  - `interrupt()`（466 行）: Stop 受理の durable commit、`queue_paused` セット、共通 abort watchdog spawn、再押下（強制終端要求）処理。
  - `apply_runtime_event` の `TurnCompleted` 分岐（2542 行）: 無条件 drain を `queue_paused` による条件付き drain へ変更。
  - queue 再開 usecase（`resume_queue`）を追加。
- `src-tauri/src/usecase/agent_session/runtime/session_state.rs`
  - `RuntimeSessionState` に `queue_paused`、pause revision、interrupt 対象 generation を追加。event projection から初期化し、`reset_for_turn` では変更しない。
- `src-tauri/src/usecase/agent_session/event_log/events.rs`
  - `AgentSessionEvent` に `TurnInterruptRequested`、`QueuePaused`、CAS 付き `QueueResumed` を追加。
- `src-tauri/src/adaptor/gateway/agent_session/session_storage/event_store.rs`（+ trait 定義 `session_storage.rs`）
  - 複数イベントを単一 batch envelope として末尾 append する API を追加。履歴全体の read/rewrite を避け、途中 crash では envelope 全体を不可視にする。
- `src-tauri/src/adaptor/controller/command/agent_session/session.rs`
  - queue 再開 command（`resume_agent_queue`）を追加。`interrupt_agent_query` は変更不要（冪等な再呼び出しを許容）。

### frontend（TypeScript）

- `src/hooks/useAgentChat.ts`
  - `interrupt` の握りつぶし分岐（757 行）を撤去し、再押下でも `interrupt_agent_query` を再送。
  - queue 再開を呼ぶ `resumeQueue`（`invoke("resume_agent_queue")`）を追加。
- `src/hooks/agentChatReducer.ts` / `useAgentChat.ts`
  - `queuePaused` の read model（backend の StateChange から反映）を保持し、queue chips 表示へ渡す。
- `src/components/panels/AgentChatPanel/MessageInput.tsx`
  - 停止ボタンの `disabled={isInterrupting}`（805 行）を撤去。表示は「Stopping…」を維持しつつ押下は受け付ける。
- `src/components/panels/AgentChatPanel/BoundSessionChat.tsx`
  - queue chips の再開操作を `resumeQueue` に接続（暫定 UI）。

frontend に判断ロジックは置かない。paused の source of truth は backend、frontend は表示と invoke に徹する（A5 / Rust-first）。

## アーキテクチャと責務分割

```
frontend (表示・invoke)
  ├─ 停止ボタン再押下 → invoke("interrupt_agent_query")   … 握りつぶさない
  └─ queue 再開操作   → invoke("resume_agent_queue")

adaptor/controller/command/agent_session/session.rs
  ├─ interrupt_agent_query → usecase.interrupt()
  └─ resume_agent_queue    → usecase.resume_queue()

usecase/agent_session/runtime/usecase.rs        … 送信・runtime event の orchestration
  ├─ interrupt() / resume_queue() から transition collaborator を利用
  └─ apply_runtime_event(TurnCompleted): queue_paused を見て drain 判定

usecase/agent_session/runtime/transitions.rs    … per-session transition owner
  ├─ Stop / resume / terminal を直列化する単一 transition coordinator
  └─ timeout 後に古い provider await から command waiter を切り離す世代付き command lock

usecase/agent_session/runtime/session_state.rs  … projected pause 状態の live mirror
usecase/agent_session/event_log/events.rs       … TurnInterruptRequested / QueuePaused / QueueResumed

infrastructure/agent_session/codex/session.rs   … turn_id 未取得ウィンドウの予約送出
infrastructure/agent_session/claude/session.rs  … 既存 synthetic abort（変更なし）
```

責務境界:

- **強制終端の保証（I5 / R3）は usecase 層が所有する**。backend session 層の実装差（Codex は予約、Claude は synthetic abort）に依存せず、usecase の abort watchdog が「最悪 10s で terminal」を両 backend に共通で保証する。backend interrupt はベストエフォートの高速経路、watchdog は最終保証。
- **Stop / resume / terminal の同期境界は `SessionTransitionCoordinator` が所有する**。runtime open / start failure も同じ境界で `complete_turn` へ入り、Stop commit が先なら `Interrupted` へ reconcile し、failure terminal が先なら Stop batch を append しない。watchdog terminal 時は command lock entry を世代更新し、返らない provider await を保持する古い guard から resume / follow-up command を切り離す。
- **queue paused の durable source of truth は event log projection**。`RuntimeSessionState.queue_paused` は live mirror とし、session 初期化時に projection から hydrate する。frontend / backend session / abort watchdog は false に戻さず、CAS 付き `QueueResumed` を commit できる `resume_queue` だけが解除する（L-D5 / R5）。
- **Codex の予約は infrastructure 層の閉じた実装詳細**。usecase から見れば `interrupt()` は「受理される（no-op にならない）」ことだけを保証する。予約フラグや `turn/started` 連動は codex session 内部に閉じる。

### StartingTurn phase の扱い

現行 `RuntimeSessionPhase` は `Idle / Streaming / WaitingPermission` の 3 値で、独立した `StartingTurn` は存在しない。`start_turn`（usecase.rs 1432-1550 行）は `reset_for_turn` で即 `phase = Streaming` にした後、同期的に `runtime.start_turn()`（backend への `turn/start` 書き込み）を実行する。

したがって requirements/behavior の「StartingTurn（backend ack 前）」は、実装上は **phase=Streaming かつ Codex `turn_id` 未取得** の状態に対応する。新 phase は導入せず、この状態での Stop を「Codex interrupt 予約 + usecase abort watchdog」でカバーする（R2 の予約と R3 の watchdog が同一ウィンドウを二重に守る）。

「late ack を通常の TurnStarted commit に流さない」は Codex session 層で実現する。予約フラグが立った状態で `turn/started` を受信したら、その turn を通常継続させず即 `turn/interrupt` を送出する（下記フロー）。backend / interrupt 結果が不明なまま app-server が終端通知を返さない場合は、usecase watchdog が `Interrupted{Timeout}` へ畳む（reconciliation の最終保証）。

## データモデルまたは型

### `CodexRuntimeState`（codex/session.rs）

```rust
struct CodexRuntimeState {
    thread_id: Option<String>,
    turn_id: Option<String>,
    // turn/start request と、その request にだけ有効な Stop 予約。
    active_turn_start_request_id: Option<u64>,
    interrupt_requested_for: Option<u64>,
    // ... 既存フィールド
}
```

- `start_turn()`: write 対象の request id を `active_turn_start_request_id` に保持する。
- `interrupt()`: `thread_id` が無ければ従来どおり `Ok(())`。`turn_id` が無い場合は active な `turn/start` request id があるときだけ `interrupt_requested_for` に同じ id を予約する。provider terminal 後の `turn_id=None` では予約しない。両 id があれば即 `turn/interrupt` を送出する。
- `read_loop`: `turn_id` 確定時、予約 id と active request id が一致する場合だけ `turn/interrupt` を write する。
- `TurnCompleted` / `turn/start` write failure で active request と予約を同時に消し、完了通知を usecase/UI が消費する前の Stop が次 turn へ漏れないようにする。

### `RuntimeSessionState`（session_state.rs）

```rust
pub(crate) struct RuntimeSessionState {
    // ... 既存
    // projection から hydrate する live mirror。
    pub queue_paused: bool,
    pub queue_paused_at: Option<f64>,
    pub interrupt_requested_generation: Option<u64>,
}
```

- `new`: event projection の pause revision があれば `queue_paused: true` として hydrate。
- `reset_for_turn` / `rollback_started_turn`: `queue_paused` は **触らない**（turn ライフサイクルと独立。paused は明示 resume まで持続する）。ただし `resume_queue` 成功で次 turn を起動した経路では resume 側が false 済み。

### `AgentSessionEvent`（event_log/events.rs）

```rust
pub enum AgentSessionEvent {
    // ... 既存
    // Stop 受理の durable 記録。backend I/O 前に append する（R4）。
    TurnInterruptRequested {
        turn_id: TurnId,
        at: f64,
    },
    // queue paused の durable 記録。TurnInterruptRequested と同一 batch で append。
    QueuePaused {
        at: f64,
    },
    QueueResumed {
        expected_paused_at: f64,
        at: f64,
    },
}
```

- projection は最後の `QueuePaused.at` を revision として保持する。`QueueResumed.expected_paused_at` が現在 revision と一致するときだけ解除するため、古い resume が新しい pause を上書きしない。GetSession と RuntimeSessionState はこの projection から hydrate する。

### batch append API（event_store.rs / trait）

```rust
// 複数イベントを単一 envelope として末尾 append する。
// 途中 crash で envelope 内の部分適用が観測されないことを保証する。
fn append_session_events(
    &self,
    data_dir: &Path,
    session_id: &str,
    events: &[AgentSessionEvent],
) -> Result<(), String>;
```

既存 event log の配列要素として batch envelope を append し、reader が単一 event と envelope を透過的に展開する。末尾破損 recovery は未完了 envelope 全体を破棄する。

## 処理フロー

### F1. 通常ウィンドウの Stop（Streaming / WaitingPermission、Codex `turn_id` 取得済み）

1. frontend `interrupt` → `invoke("interrupt_agent_query")` → controller → `usecase.interrupt(session_id)`。
2. usecase `interrupt()`:
   1. global runtime-state mutex の短い区間で generation / turn / runtime を snapshot し、`queue_paused` と対象 generation を確定する。
   2. `TurnInterruptRequested` + `QueuePaused` を blocking I/O 経路の単一 envelope で **backend I/O 前に** durable append（R4）。event log 履歴全体は rewrite しない。append が失敗した場合は Stop 未受理としてここで失敗し、live state を変更しない。
   3. durable commit 成功後に live `queue_paused` / interrupt generation を反映し、abort watchdog を arm する（F3）。
   4. `runtime.interrupt().await`（backend へ interrupt 送出）。失敗しても watchdog が終端を保証する（エラーは log）。
3. backend が interrupt を処理 → app-server が turn 終了通知 → `TurnCompleted(Interrupted{Abort})` → `apply_runtime_event`（F4）。

### F2. Codex `turn_id` 未取得ウィンドウの Stop（R2 / OB-1 / SD-2）

1. `usecase.interrupt()` は F1 と同じ（durable commit → live paused 確定 → watchdog arm → `runtime.interrupt()`）。
2. codex `interrupt()`: `turn_id` が無いので active `turn/start` request id に紐付く予約を保持して `Ok(())`。**no-op にしない**。
3. app-server から `turn/started` 到着 → read_loop が `turn_id` を確定 → request id が一致する予約だけを即 `turn/interrupt` として送出する。その turn は通常継続しない。
4. backend が終端通知 → `TurnCompleted(Interrupted)` → F4。
5. `turn/started` が来ない / interrupt 応答が来ない場合 → F3 の watchdog が 10s で強制終端。

### F3. 共通 abort watchdog（R3 / I5 / L-D2）

```
usecase.interrupt() 内:
  let generation = current generation を snapshot;
  tokio::spawn(async move {
    sleep(ABORT_SYNTHESIS_DELAY /* 10s, A3 */);
    session guard を取得;
    if state.generation == generation && state.phase != Idle {
      // backend 停止が確認できなかった → 強制終端
      complete_turn(Interrupted{Timeout}) 相当を合成し、
      apply 後は queue_paused により drain しない（F4）。
    }
  });
```

- `generation` は `reset_for_turn` でインクリメントされる turn 世代。watchdog 発火時に generation 不一致 or phase=Idle なら「既に別経路で終端済み」を意味し何もしない（冪等）。
- Claude 既存 `spawn_abort_synthesis_timer` は撤去しない。Claude では session 層タイマと usecase watchdog が二重に走るが、先に届いた `TurnCompleted` が `complete_turn` の generation/phase guard（3157-3169 行）で終端し、後続は no-op になる。二重終端は起きない。
- reason は `InterruptReason::Timeout`（既存 enum に定義済み・現状 dead_code の値を本 Issue で活性化）。ユーザー明示 Stop で backend 応答があった通常経路は従来どおり `Abort`。
- watchdog 強制終端時は runtime を state から detach し `runtime_epoch` を進めてから finalize する。さらに世代付き command lock を更新し、古い `open_session().await` / `start_turn().await` が返らなくても、その guard を待つ resume / follow-up command を新世代で続行可能にする。旧 runtime の close は post action で行い、resume は新 runtime を open する。runtime open の attach と `start_turn().await` 完了側も runtime epoch / generation / phase / runtime identity を再確認し、timeout 後の遅延成功・失敗通知を捨てる。
- Stop 受理済み generation に backend の `Completed` / `Failed` が届いた場合は、通常 terminal として確定せず `Interrupted{Abort}` へ reconciliation する。

### F4. `TurnCompleted` 受信時の drain 条件化（R5 / OB-5）

```
apply_runtime_event(TurnCompleted(result)):
  let notification = complete_turn(...).await;
  let mut actions = RuntimeEventPostActions::workflow(notification);
  if !state.queue_paused {          // ← 変更点（従来は無条件 drain）
    actions.drain();
  }
  return actions;
```

- interrupt 経由（queue_paused=true）では drain しない → 次 queue メッセージは自動実行されない（R5 / behavior「停止直後に自動実行されない」）。
- pending_queue の中身は保持（削除しない）。無損失（behavior A6）。
- 通常完了（interrupt していない）では `queue_paused=false` のまま → 従来どおり drain。回帰なし。

### F5. queue 再開（R5 / L-D5）

1. frontend queue chips の再開操作 → `invoke("resume_agent_queue")` → controller → `usecase.resume_queue(session_id)`。
2. `resume_queue()`:
   1. projection / live mirror の pause revision を読む。
   2. `QueueResumed { expected_paused_at }` を durable append し、同じ revision の場合だけ live mirror を false にする（CAS）。
   3. `phase == Idle && !pending_queue.is_empty()` なら `start_next_queued_turn` を呼ぶ。
3. paused を false に戻せるのはこの経路だけ。interrupt / backend / frontend の停止経路は queue を再開しない（behavior「停止経路は勝手に再開しない」）。

### F6. frontend 再押下（R6 / OB-1 frontend）

- `useAgentChat.ts` `interrupt`: 757 行の `if (interruptingRef.current[sessionId]) return;` を撤去。再押下でも `invoke("interrupt_agent_query")` を再送する。
- `MessageInput.tsx`: 停止ボタンの `disabled={isInterrupting}` を撤去（表示ラベルは「Stopping…」を維持してよいが押下可能）。
- backend 側の再押下（二度目の `interrupt()`）は **強制終端要求** として扱う: `queue_paused` は既に true（durable commit / paused は冪等 = 再 append しない）。二度目は残り猶予を待たず watchdog を即発火させる（generation が同一かつ phase != Idle なら即 `Interrupted{Timeout}` 合成）。これにより「停止が効かないまま再送手段が失われる」状態を作らない（behavior）。

### F7. crash 耐性（R4）

- Stop 受理は F1-2 の通り backend I/O 前に `TurnInterruptRequested` + `QueuePaused` を 1 batch で durable append する。
- pending_queue 本体は L3 #1404 まで in-memory だが、pause revision は本 Issue で projection し、GetSession と RuntimeSessionState の初期化時に復元する。終端前 crash 後も `queue_paused=true` を返し、自動 drain しない。
- 既に paused 済みで再度 Stop 受理された場合は、event の再 append を行わず（冪等）、状態も paused のまま（behavior「冪等」）。

## エラー処理

- **backend interrupt 送出失敗**（`runtime.interrupt()` が `Err`）: 握りつぶさず log に記録するが、usecase `interrupt()` 自体は成功として扱う（Stop は受理済み）。abort watchdog が終端を保証するため、frontend にエラーを返して再送を促す必要はない。
- **durable append 失敗**（`append_session_events` が `Err`）: Stop は未受理として呼び出し元へエラーを返す。live `queue_paused` / interrupt generation、watchdog、backend interrupt、通知は変更・実行しない。durable projection を source of truth とするため、永続化できなかった Stop を live state だけで成功扱いにしない。
- **Codex 予約 interrupt の write 失敗**（read_loop での送出失敗）: log に記録。watchdog が 10s で終端を保証する。
- **watchdog 発火時に session 消滅**（close 済み等）: generation/phase guard で no-op。パニックしない。
- **resume_queue の CAS 失敗**（paused でない）: no-op（冪等）。エラーにしない。
- **queue resume durable append 失敗**: live mirror を解除せずエラーを返す。再起動時に古い pause が復活する状態を作らない。
- **予約が次 turn へ漏れる**リスク: 予約を active `turn/start` request id に紐付け、provider terminal / write failure 時に request・予約・turn id を同時にリセットして防ぐ。

## テスト方針

配置は既存規約に従い、各 module 内の `#[cfg(test)] mod tests` と frontend の隣接 `*.test.ts(x)`。外部プロセスは起動しない（app-server / CLI はモック）。

### backend unit

- `codex/session.rs`
  - active `turn/start` の `turn_id` 未取得で `interrupt()` → 同じ request id の予約、write なし、`Ok(())`（no-op でない）。
  - 予約状態で `turn/started`（turn_id 確定）→ `turn/interrupt` が write される。予約が通常 turn として継続しない。
  - `turn_id` 取得済みで `interrupt()` → 即 `turn/interrupt`。
  - provider terminal 後、usecase 通知前に Stop が競合しても次 turn の予約にならない。
- `session_state.rs`
  - `queue_paused` の初期値、`reset_for_turn` / `rollback_started_turn` で不変であること。
- `transitions.rs`
  - 同一 session の Stop / resume / terminal が単一 coordinator で直列化されること。
  - watchdog の command lock 世代更新で、古い provider await の guard を解放しなくても待機中 command が続行できること。
- `usecase.rs`
  - `interrupt()` で durable append（2 イベント）が backend I/O 前に呼ばれる（呼び出し順序）。
  - `interrupt()` で `queue_paused = true`。
  - abort watchdog: production の `interrupt()` が arm した task を tokio 仮想時刻で 9s 前は active、10s 境界で `Interrupted{Timeout}` と検証。timeout 後は runtime detach / epoch 更新 / close、新 runtime resume、旧 runtime late event drop も検証。
  - Stop 後に到着した `Completed` / `Failed` が `Interrupted` として確定すること。
  - `apply_runtime_event(TurnCompleted)`: `queue_paused=true` で drain しない、`false` で従来どおり drain。
  - `resume_queue()`: paused=true で false 化 + `start_next_queued_turn` 起動、paused=false で no-op。
  - 再押下（二度目 interrupt）で watchdog 即発火。paused 再 append なし（冪等）。
  - durable Stop append と runtime open / start failure の競合で、event log・live pause・GetSession・通知が同じ terminal 状態へ reconcile されること。
- `event_store.rs`
  - `append_session_events` が複数イベントを atomic に書く（途中失敗で部分適用が残らない）。

### frontend unit

- `useAgentChat` `interrupt`: 再押下で `invoke("interrupt_agent_query")` が 2 回呼ばれる（握りつぶさない）。
- `MessageInput`: interrupt 中でも停止ボタンが `disabled` にならず onInterrupt が呼べる。
- queue chips 再開操作で `invoke("resume_agent_queue")` が呼ばれる。

### 統合テスト（受け入れ基準 / behavior「ユーザー Stop の一括保証」）

`pnpm test:integration`（または backend 統合テスト経路）で、Codex セッションの「送信直後 Stop」シナリオ:

- turn が最悪 10s で Idle（terminal）へ着地。
- queue が paused、queue メッセージが自動実行されない。
- 入力欄テキスト・添付・queue の各メッセージが保持される。
- 停止ボタン再押下が無視されない。

## リスクと代替案

- **R3 の実装場所（usecase 共通 vs codex session）**: requirements R3 は「runtime 共通（または codex session）」を許容する。本設計は **usecase 共通 watchdog** を採用した。理由: (1) backend 非依存で両 backend を 1 経路で保証でき I5 の「共通保証」に直接対応する、(2) `complete_turn` の generation guard を再利用でき二重終端を防げる、(3) Claude 既存の session タイマを撤去せず段階移行できる。代替案（codex session にも synthetic timer を持たせる）は Claude と対称になるが、保証が backend 実装に分散し I5 の一元保証から遠ざかるため退けた。
- **Claude タイマの二重化**: usecase watchdog と Claude session タイマが同時に走る。generation/phase guard で無害だが、将来的には Claude session タイマを usecase watchdog に一本化する余地がある（本 Issue のスコープ外・別 Issue で検討）。
- **durable commit の L1/L3 境界**: pause/resume revision の event append・projection・hydrate は本 Issue で完結する。L3（#1404）へ残すのは pending queue 本体の永続化・取消であり、paused 状態そのものは再起動後も保証する。
- **queue_paused と通常完了の相互作用**: interrupt せずに通常完了した turn は `queue_paused=false` のままなので従来どおり drain される。回帰リスクは低いが、`reset_for_turn` で paused を触らない設計のため「paused 中に何らかの経路で turn が開始・完了した」場合に drain されない可能性がある。これは意図どおり（paused は resume まで持続）だが、`resume_queue` 以外で turn が起動する経路が無いことをテストで担保する。
- **予約 interrupt の write を read_loop で行う**: read_loop に handle を渡すことで責務がやや増える。代替は「usecase 層で `turn/started` 相当イベントを検知して再度 interrupt を呼ぶ」だが、`turn_id` の source of truth が codex session 内部にあるため、予約〜送出を session 内で閉じる方が状態の所有が明確（full-retention/recompute を増やさない）。

## 仮定

- **A1（新 phase を導入しない）**: `StartingTurn` は独立 phase を追加せず、「phase=Streaming かつ Codex `turn_id` 未取得」で表現する。予約（R2）と watchdog（R3）が同ウィンドウを守る。
- **A2（R3 は usecase 共通 watchdog）**: 強制終端タイマは usecase 層に置き、両 backend を 1 経路で保証する。Claude 既存の session タイマは撤去せず併存させる（generation guard で無害）。
- **A3（10s は既存値踏襲）**: 猶予上限は `ABORT_SYNTHESIS_DELAY = 10s`（requirements A3 / L-D2）。turn ごとの可変化はしない。
- **A4（Timeout reason を活性化）**: 強制終端は `InterruptReason::Timeout`（既存 enum・現状 dead_code）を用いる。backend 応答があった通常 Stop は `Abort`。
- **A5（queue_paused は durable projection + live mirror）**: event log の `QueuePaused` / `QueueResumed` が durable source of truth、RuntimeSessionState はそこから hydrate する live mirror とする（requirements A2）。
- **A6（durable batch append を追加）**: `append_session_events` は複数イベントを単一 envelope として末尾 append し、履歴量に比例する full-log rewrite を行わない。
- **A7（再開 UI は暫定既存 UI）**: queue 再開は queue chips 相当の既存 UI から `resume_agent_queue` を呼ぶ。専用 UI は作らない（requirements A4）。
- **A8（再押下 = 即強制終端要求）**: interrupt 中の再押下は残り猶予を待たず watchdog を即発火させる。paused の再 append はしない（冪等）。
- **A9（ロジックは Rust）**: 予約・watchdog・paused・durable commit・drain 判定・CAS 再開はすべて Rust に置く。frontend は握りつぶし撤去・invoke・表示のみ（Rust-first / requirements A5）。

## Open Questions

なし。
