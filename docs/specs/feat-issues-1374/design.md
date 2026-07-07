# Design

## 概要

無出力 timeout（stale watchdog）が turn のエラー終端・interrupt・runtime close を合成する現行経路を廃止する。turn の完了・失敗・中断は backend 由来の明示的終端イベント（`AgentRuntimeEvent::TurnCompleted` → `complete_turn`）でのみ確定させる。

無出力 timeout は、session / runtime を破棄しない補助 signal として再定義する。到達時は「無反応の観測」を非破壊な介入点として提示し、backend の終端イベントを引き続き待てる状態を保つ。暴走を防ぐため、signal 再発火・自動 recovery には上限を設ける。

本 Spec の中心変更は `src-tauri/src/usecase/agent_session/runtime/` の `stale.rs` と `usecase.rs`（`spawn_stale_watchdog_task`）に閉じる。全ロジックは Rust に置き、frontend は signal を受けて表示・介入操作の invoke に徹する（`rust-first-logic`）。

## 変更対象

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs`
  - `spawn_stale_watchdog_task`: stale 判定成立時の `complete_turn(Interrupted{Timeout})` → `runtime.interrupt()` → `STALE_CLOSE_GRACE` → `runtime.close()` を廃止し、非破壊 signal 経路へ差し替える。
  - watchdog ループの再武装（rearm）ロジックを、破棄せず「再評価しながら待ち続ける」形へ変更する。
- `src-tauri/src/usecase/agent_session/runtime/stale.rs`
  - `STALE_TIMEOUT_MESSAGE` / `STALE_CLOSE_GRACE` の用途変更または削除。
  - stall signal 用の判定・上限ヘルパの追加。
  - `turn_is_stale` / `effective_stale_timeout` / `has_in_flight_tool_use` / `remaining_until_stale` は再利用する（signal 発火閾値の計算に流用）。
- `src-tauri/src/usecase/agent_session/runtime/session_state.rs`
  - stall 観測の再発火・自動 recovery 試行回数を保持するカウンタを `RuntimeSessionState` に追加。`reset_for_turn` / `rollback_started_turn` でリセットする。
- `src-tauri/src/usecase/agent_session/runtime/ports.rs`
  - 非終端の stall signal を frontend / WS へ伝えるための notifier method を `AgentSessionEventNotifier` に追加（後述の代替案で確定）。
- `src-tauri/src/domain/agent_session/gateway.rs`
  - `AgentSessionRuntime` に非破壊 recovery 用の optional capability method `reconnect`（`steer` と同じく default で `Unavailable` を返す）を追加。stream / transport の再接続と backend-owned state 再読込を、runtime / session を破棄せずに行う。
- backend 実装（Codex / Claude runtime）
  - `reconnect` を実装できる backend は実装。未対応 backend は default（`Unavailable`）のままとし、watchdog 側は介入点提示へフォールバックする。
- 上記 notifier method に対応する adaptor（presenter / protocol / ws_bridge）と frontend 受信側。frontend はロジックを持たず、signal 表示と retry / continue / abort の invoke に徹する。

`requirements.md` / `behavior.md` は変更しない。

## アーキテクチャと責務分割

責務を「turn 終端の確定」と「無反応の観測」に完全分離する。

| 関心事 | 所有者 | 役割 |
|---|---|---|
| turn 終端の確定 | `complete_turn`（backend event 経由のみ） | `TurnCompleted` を受けて turn を完了・失敗・中断で確定。無出力を根拠にしない。 |
| 無反応の観測 | stale watchdog（再定義後） | 閾値到達を非破壊 signal として観測・通知。session / runtime を破棄しない。 |
| 別経路の終端 | 既存経路（user cancel / workflow timeout / backend fatal / tool timeout） | 従来どおり turn / session を終端。本変更の影響外。 |
| signal の表示・介入 | frontend（invoke のみ） | stall signal を表示し、利用者の retry / continue / abort を backend command へ委譲。 |

### watchdog の新しい役割

現行の watchdog は「stale 判定 → turn 合成終端 → runtime 破棄」を一括で行う。変更後は次に限定する。

1. `turn_is_stale` が成立したら、turn 状態を一切終端させず、非破壊の stall signal を発火する（notifier 経由で frontend / workflow へ）。
2. 上限内であれば非破壊 recovery（`runtime.reconnect()` による stream / transport 再接続と backend-owned state 再読込）を自動試行する。runtime / session は破棄しない。`reconnect` が `Unavailable`（未対応 backend）の場合は recovery をスキップし介入点提示に留める。
3. signal 発火後も session が `Streaming` / `WaitingPermission` かつ同一 generation である限り、watchdog は終了せず待機を再武装する。以降の再発火・自動 recovery は上限までに制限する。
4. backend の `TurnCompleted` が届いた時点で通常経路の `complete_turn` が turn を確定させ、generation 差により watchdog は自然に終了する（`stale_watchdog_should_continue_waiting` が false を返す）。

watchdog はいかなる場合も `complete_turn` / `interrupt` / `close` を呼ばない。呼び出しうる runtime 操作は非破壊の `reconnect` のみ。これが本 Spec の核心。

## データモデルまたは型

### `RuntimeSessionState` への追加（`session_state.rs`）

```rust
/// 現 turn（generation）で無反応 signal を何回発火したか。
pub stall_signal_count: u32,
/// 現 turn で非破壊 recovery（reconnect）を何回自動試行したか。
pub stall_recovery_attempts: u32,
```

- 初期値 0。`new()` / `reset_for_turn()` / `rollback_started_turn()` で 0 にリセットする（turn 単位で暴走上限を数える）。

### stall signal notification（新規）

非終端の観測 signal を表す payload を `ports.rs` に追加する。

```rust
#[derive(Debug, Clone)]
pub(crate) struct AgentStallObservedPayload {
    pub chat_session_id: String,
    /// signal 発火時点の phase（Streaming / WaitingPermission）。
    pub turn_phase: TurnPhase,
    /// 無出力継続時間（秒）。表示・判断材料。
    pub idle_secs: u64,
    /// 現 turn での通算 signal 回数（1 始まり）。
    pub signal_count: u32,
    /// 上限到達フラグ。true のとき自動再発火は止まり、介入点のみが残る。
    pub cap_reached: bool,
}
```

`AgentSessionEventNotifier` に非終端メソッドを追加する。

```rust
fn stall_observed(&self, payload: AgentStallObservedPayload);
```

`session_state_changed`（`SessionState` を運ぶ）は turn 終端・状態遷移用であり、stall は状態遷移ではないため再利用しない。stall は「turn は Streaming のまま」で発火する観測 signal として独立させる。

### `stale.rs` の定数・ヘルパ

```rust
/// 現 turn で許容する無反応 signal の最大発火回数（暴走防止）。
const MAX_STALL_SIGNALS: u32 = 3;
/// 現 turn で許容する非破壊 recovery（reconnect）の最大自動試行回数。
const MAX_STALL_RECOVERY_ATTEMPTS: u32 = 3;

/// signal 発火上限到達判定。
pub(crate) fn stall_cap_reached(signal_count: u32) -> bool {
    signal_count >= MAX_STALL_SIGNALS
}

/// 自動 recovery 試行上限到達判定。上限後は介入点提示のみに委ねる。
pub(crate) fn recovery_cap_reached(recovery_attempts: u32) -> bool {
    recovery_attempts >= MAX_STALL_RECOVERY_ATTEMPTS
}
```

- `STALE_TIMEOUT_MESSAGE`（エラー中断メッセージ）と `STALE_CLOSE_GRACE`（close 前 grace）は破壊経路廃止に伴い削除する。表示文言が必要な場合は frontend 側の非エラー文言として持つ（backend からはエラーとして送らない）。

### `AgentSessionRuntime::reconnect`（`gateway.rs`）

`steer` と同じ optional capability 方式で追加する。

```rust
/// 非破壊 recovery: stream / transport を再接続し、backend-owned state を再読込する。
/// runtime / session は破棄しない。未対応 backend は default の Unavailable を返す。
async fn reconnect(&self) -> Result<(), AgentBackendError> {
    Err(AgentBackendError::Unavailable(
        "session reconnect is not available for this backend".to_string(),
    ))
}
```

- `Unavailable` は「recovery 手段が無い」ことを示すだけで error ではない。watchdog はこれを介入点提示へのフォールバックとして扱い、turn を終端しない。
- `reconnect` 成功後も turn 状態・generation は変えない。再接続した event stream から遅れて `TurnCompleted` が届けば通常経路で確定する。

## 処理フロー

### stall watchdog（変更後）

1. turn 開始時に `spawn_stale_watchdog_task(session_id, generation, timeout)` を spawn（起動点は現行どおり）。
2. ループ:
   - `effective_stale_timeout`（tool in-flight 中は上限まで延長）で閾値を計算。
   - `turn_is_stale` が false かつ `stale_watchdog_should_continue_waiting` が true → `remaining_until_stale` 相当の delay で sleep して再評価。false（Idle / generation 不一致）→ **return（watchdog 終了）**。
   - `turn_is_stale` が true → stall 観測ブロックへ。
3. stall 観測ブロック（session lock 下で count のみ increment。turn 状態・phase・generation は変更しない）:
   - `state.stall_signal_count += 1`。
   - `cap_reached = stall_cap_reached(state.stall_signal_count)`。
   - `notifier.stall_observed(payload)` を発火（`idle_secs`・`signal_count`・`cap_reached`）。
   - **turn 状態・phase・generation は一切変更しない。** `complete_turn` / `interrupt` / `close` は呼ばない。
4. 非破壊 recovery（session lock を解放してから実行。lock 保持中に await しない）:
   - `recovery_cap_reached(state.stall_recovery_attempts)` が false のとき → `state.stall_recovery_attempts += 1` した上で `runtime.reconnect()` を試行。
     - `Ok(())` → 再接続成功。turn 状態は変えず、再接続後の stream から `TurnCompleted` を待つ。
     - `Err(Unavailable)` → 未対応 backend。recovery をスキップし介入点提示のみ。
     - その他 `Err` → `log::warn!` に留め turn を終端しない。次回 stall で再試行（上限まで）。
   - recovery 上限到達 → 以降 reconnect は行わず、利用者・workflow 介入点へ委ねる。
5. 再武装:
   - signal / recovery のいずれかが上限未満 → `last_progress_at` は signal 発火で動かさず、次の閾値（`timeout` 基準）で再度 sleep し、無出力が続けば再評価。
   - 両上限に到達 → 以降 stall signal も recovery も行わない。watchdog は generation 変化（backend event による turn 確定）または phase が Idle になるまで待つか、そのまま終了する。session / runtime は破棄しない。
6. backend の `TurnCompleted` 受信 → 既存の runtime event 経路が `complete_turn` を呼び、`state.phase = Idle` / generation 更新。watchdog は次評価で `stale_watchdog_should_continue_waiting == false` により return。

再発火の判定に「無出力の連続」を使うため、signal 発火自体は `last_progress_at` を更新しない（backend の実出力のみが `last_progress_at` を進める、という現行契機を維持）。これにより「1 回の無反応区間で 3 回まで signal → 以後は沈黙」という有限な挙動になる。

### 介入操作（frontend → backend）

stall signal を受けた frontend は、既存の backend command を invoke する。新規ロジックは追加しない。

- retry / continue: 既存の send message / steer 経路。
- abort: 既存の user cancel（明示 cancel）経路。これは「別経路の終端」であり従来どおり turn / session を終端する。

介入は利用者の明示操作としてのみ turn を終端させる。watchdog は終端を起こさない。

### 別経路の終端（不変）

user 明示 cancel、workflow の wall-clock / run timeout、backend の明示 terminal / fatal event、tool 固有 timeout は現行コードのまま。本変更は無出力 timeout 経路のみを触る。

## エラー処理

- watchdog は turn を Error にしない。無出力継続は「観測」であり error ではない。
- backend の異常（transport 断・process 異常）が実際に起きた場合は、従来どおり backend event（fatal / crash）経路が `complete_turn(Failed)` 相当で確定させる。watchdog はそれを待つだけで、無出力を error に昇格しない。
- notifier / lock 取得失敗などの内部エラーは既存方針どおり `log::warn!` に留め、turn 状態を破壊しない。
- session が watchdog 待機中に別経路で終端した場合、generation 不一致で watchdog は安全に return する（既存ガードを踏襲）。

## テスト方針

`stale.rs` / `usecase.rs` の `#[cfg(test)] mod tests` に追加する。外部プロセスは起動しない。

### `stale.rs`（純関数ユニット）

- `stall_cap_reached` / `recovery_cap_reached`: 上限未満で false、上限到達・超過で true。
- `turn_is_stale` / `effective_stale_timeout` / `remaining_until_stale`: 既存テストを維持（挙動不変）。

### `usecase.rs`（watchdog 挙動、既存の spawner / notifier fake を利用）

- **無出力 timeout 到達でエラー中断・close が起きない**: streaming で閾値到達 → `complete_turn` 未呼び出し・`runtime.interrupt/close` 未呼び出し・session が Streaming のまま・`stall_observed` が発火。
- **生きているが無出力の区間**（reasoning 中 / ToolUse 未到着 / KeepAlive 途絶）で turn がエラー終端されない。
- **非破壊 recovery を試行する**: 閾値到達で `runtime.reconnect()` が上限内で呼ばれ、turn 状態が変わらない（fake runtime で reconnect 呼び出しを観測）。`Unavailable` を返す backend では reconnect スキップ・turn 継続・介入点提示のみ。
- **backend 終端イベントで確定**: 待機中に `TurnCompleted` → `complete_turn` が正しく完了させ、watchdog が終了する。
- **上限到達で自動処理が止まる**: 連続無出力で signal が `MAX_STALL_SIGNALS` 回・reconnect が `MAX_STALL_RECOVERY_ATTEMPTS` 回まで実行され、それ以降は発火・reconnect ともに止まり、session / runtime が破棄されない（`cap_reached=true` payload を最後に観測）。
- **別経路の終端は不変**: user cancel / workflow timeout / backend fatal による終端が従来どおり動く（既存テストを退行させない）。
- **687cb7c9 との整合**: WaitingPermission 中の再武装、tool in-flight 中の延長など直近修正の挙動を退行させない。

## リスクと代替案

### リスク

- **本当に死んだ session が Streaming のまま残る**: 無出力を error にしないため、backend が二度と終端イベントを出さないケースで session が張り付く。→ stall signal による可視化と、利用者の明示 abort・workflow timeout・backend fatal 経路で回収する。watchdog 自身は破棄しない方針を維持する（要件 3・4）。
- **signal 通知過多**: 上限（`MAX_STALL_SIGNALS`）と「turn 単位でカウントリセット」で有界化する。
- **frontend / protocol の追加**: 新 notifier method に伴う adaptor 追加が必要。既存の `session_state_changed` 経路に相乗りしないことで、状態遷移との混線を避ける。

### 代替案

1. **stall signal を新規 notifier method で送る（採用）**: turn 状態と独立した非終端 signal として最も素直。protocol 追加コストはあるが責務が明確。
2. **`SessionState` に非終端 `Stalled` variant を追加**: 変更は小さいが、`SessionState` は終端・状態遷移の source of truth であり、非終端の観測を混ぜると full-retention/状態機械が濁る。不採用。
3. **watchdog を廃止し signal を出さない**: 要件 4「介入点の提示」を満たさず、無反応が完全に不可視になるため不採用。
4. **自動 transport reconnect の実装方式**: `AgentSessionRuntime` に reconnect primitive が現状存在しない（`start_turn` / `steer` / `interrupt` / `close` のみ）。`steer` と同じ optional capability 方式（default `Unavailable`）で `reconnect` を追加する案を採用。対応 backend は再接続、未対応 backend は介入点提示へフォールバックする。全 backend に一律実装を強制せず、要件 4 の非破壊 recovery を段階的に満たせる。

## 仮定

- 現行挙動（`spawn_stale_watchdog_task` が Timeout interrupt → close、`DEFAULT_STALE_TIMEOUT` 180 秒 / 上限 1800 秒、`last_progress_at` の 3 更新契機）を現行仕様として扱う。
- backend の明示的終端イベントは `AgentRuntimeEvent::TurnCompleted` として既に runtime usecase へ届いており、完了判定の正系として利用できる。
- `workflow_step_context.stale_timeout_secs` は「stall signal の発火閾値」として意味づけを保つ（廃止しない・後方互換を壊さない）。この意味変更方針の確定は後続 Spec に委ねる。
- 暴走防止上限は turn 単位で `MAX_STALL_SIGNALS = 3` / `MAX_STALL_RECOVERY_ATTEMPTS = 3` を初期値とする。具体値は運用で調整可能とし、設定化は本 Spec では行わない。
- stall signal は「turn は Streaming のまま」で送る非終端 signal であり、`SessionState` の遷移を伴わない。
- `reconnect` は `steer` に倣った optional capability method（default `Unavailable`）とし、未対応 backend では介入点提示にフォールバックする。各 backend の reconnect 具体実装（Codex / Claude それぞれの再接続手順）は backend 実装側の詳細として扱う。

## Open Questions

なし（要件 4 の非破壊 recovery は `AgentSessionRuntime::reconnect` を optional capability として追加し、上限内で自動試行・未対応 backend は介入点提示へフォールバックする方針で確定）。
