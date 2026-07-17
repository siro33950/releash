# Design

関連: #1405 / requirements.md / behavior.md / milestone 84（Agentチャット安定化・Phase 0 / L4）/ 監査 RT-1 / 不変条件 I1（turn 終端保証）/ 語彙 V-D7（`InterruptReason::SessionClosed` の additive 追加）

## 概要

進行中 turn を持つ Agent セッションを、正常な `close_session`（タブを閉じる）・`set_session_backend`（backend 切替）・`close_all`（アプリ終了）のいずれで閉じても、その turn を必ず「streaming flush 強制 → `SessionClosed` 理由で finalize → runtime close」の順で終端させる（I1）。これにより再オープン後・再起動後に、本文欠落・permission 残骸・ツール実行残骸・terminal event 欠落（監査 RT-1）が残らないことを保証する。

現行の `close_session` は sessions map から state を先に remove し、in-flight ランタイムイベントを epoch guard で捨てたうえで `runtime.close()` するだけであり、`flush_streaming_update` / finalize を一切呼ばない。本設計はこの終了順序を是正し、既存の finalize 経路（`complete_turn` → `terminal_projection` → `append_final_turn_events` → `finalize_turn`）を close 経路から再利用する。新規の修復経路は作らない。

語彙面では domain / event log の両 `InterruptReason` に `SessionClosed` を additive で追加する（F3 additive 規約）。

## 変更対象

| ファイル | 変更内容 |
|---|---|
| `src-tauri/src/domain/agent_session/entities/turn.rs` | `InterruptReason` に `SessionClosed` を追加 |
| `src-tauri/src/usecase/agent_session/event_log/events.rs` | event log 側 `InterruptReason` に `SessionClosed`（serde: `"session_closed"`）を追加し、`label()` に対応を加える |
| `src-tauri/src/usecase/agent_session/event_log/projector.rs` | `project_status` の `TerminalEvent::Interrupted` 分岐に `SessionClosed` を追加し、最新 turn の durable な中断情報を射影する read model を追加 |
| `src-tauri/src/usecase/agent_session/session/mod.rs` | `SessionMeta` と `GetSessionResponse` に最新 turn の中断情報を追加 |
| `src-tauri/src/usecase/agent_session/runtime/usecase.rs` | `terminal_projection` に `SessionClosed` の写像を追加。`close_session` / `close_all` / `set_session_backend` の終了順序を「flush → drain → finalize → runtime close → state remove」へ是正する共通ヘルパを追加 |
| `src-tauri/src/usecase/application_lifecycle.rs` | agent finalize → workflow command shutdown → local API shutdown のアプリケーション終了順序と失敗時の短絡を port 境界で所有 |
| `src-tauri/src/adaptor/gateway/agent_session/session_storage/event_store.rs` | terminal event batch を temp segment へ完全書込み後、atomic rename で commit する append-only batch 保存を追加 |
| `src-tauri/src/adaptor/controller/application_lifecycle.rs` | lifecycle usecase の起動、成功後の process exit、失敗時の log に限定 |
| `src/types/session.ts` / `src/hooks/useSessionStore.ts` / `src/components/panels/AgentChatPanel/ChatSessionView.tsx` | backend-owned の中断 read model を表示用型へ変換し、対象 agent message に中断チップを描画 |

フロントエンドの close フローは変更せず、引き続き `close_session` を invoke する薄い経路のままにする。中断判定と reason は Rust の durable read model が所有し、フロントエンドは返された値を表示するだけとする（rust-first-logic）。

## アーキテクチャと責務分割

- **domain（`entities/turn.rs`）**: `InterruptReason` の語彙拡張のみ。外部依存なし。
- **usecase / event_log（`finalization.rs` / `events.rs` / `projector.rs`）**: 中断 turn の畳み込みロジック（未解決 permission → `Cancelled`、未完了 ToolCall → `ToolCallFailed`、`TurnInterrupted` 記録）は既存 `finalize_turn` をそのまま再利用する。`SessionClosed` を新しい中断理由として通せるよう enum と写像だけを拡張する。
- **usecase / runtime（`runtime/usecase.rs`）**: 終了手順の統一。`close_session` / `close_all` / `set_session_backend` が共通の「終端ヘルパ」を経由するようにする。ヘルパは進行中 turn の有無を判定し、進行中なら flush → drain → finalize → close → remove を実行する。finalize は既存 `complete_turn` を `TurnResult::Interrupted { reason: SessionClosed }` で呼ぶだけとし、terminal event 記録・permission 畳み込み・ToolCall 畳み込みは既存経路に委譲する。
- **usecase / application lifecycle（`application_lifecycle.rs`）**: agent session、workflow command、local API の shutdown port を受け、agent finalize に失敗した場合は不可逆な後続 shutdown を呼ばない業務順序を所有する。controller は usecase の起動と process exit / log だけを担当する。
- **usecase / session query**: event append 時に最新 turn terminal を `SessionMeta.last_turn_interruption` へ、最新 turn id を `SessionMeta.last_turn_id` へ射影する。`GetSessionResponse` は `TurnInterrupted` の場合だけ message id と reason を載せる。通常の turn id 採番と TurnStarted 射影は軽量 meta の bounded read/update で行い、query と start のたびに event history 全量をロードしない。旧 meta に `last_turn_id` が無い場合だけ互換 fallback として一度 event log から採番する。
- **adaptor / gateway**: terminal event 群は既存 `events.json` を truncate/rewrite せず、session 単位の append-only batch segment として保存する。temp file の write/flush が完了した segment だけを atomic rename で公開し、未完 segment は read model から無視する。closed フラグによる emit 抑止（`claude/session.rs` / `codex/session.rs`）は変更しない。

### 終了順序の是正（現状 → 是正後）

現状（`close_session`, `runtime/usecase.rs:839`）:

1. sessions map から state を remove（この時点で in-flight event は epoch/None guard で捨てられる）
2. `runtime.close()`（closed フラグで backend の終了イベント emit を抑止）

是正後（共通ヘルパ `finalize_and_close_session`）:

1. 進行中 turn 判定（`state.phase != Idle`）。進行中でなければ 5〜6 のみ（従来どおり、新規中断 turn を作らない）。
2. `flush_streaming_update(ctx, session_id, force_persist=true)`：最後の定期スナップショット以降のストリーミング本文・pending parts を durable 化する。
3. **drain（有界待ち）**：runtime を開いたまま、event pump が既に emit 済みの最終イベント（`TurnCompleted` 等）を適用しきる短時間の窓を与える。窓内に backend の実 `TurnCompleted` が届いて `complete_turn` が phase を `Idle` に落とせば、次段の SessionClosed finalize は idempotent に no-op となる（最終イベントを取りこぼさない）。
4. `complete_turn(ctx, session_id, None, TurnResult::Interrupted { reason: SessionClosed, error: None })`：既存 finalize 経路で terminal event 記録・permission/ToolCall 畳み込みを行う。terminal event 群は batch で原子的に append し、message parts と session state の永続化まで成功してから in-memory phase を `Idle` に落とす。途中で失敗した場合は runtime/state を保持し、同じ close を再試行できる。phase が既に `Idle` の場合は skip される（idempotent）。
5. `runtime.close()`：closed フラグを立て process を shutdown。
6. sessions map から state を remove。

`close_all` は保持している各セッションについて同じヘルパを適用する。`set_session_backend` は既に内部で `close_session` を呼んでいるため（`runtime/usecase.rs:820`）、`close_session` の是正がそのまま反映される。

### drain の設計

- drain は「state を remove せず・runtime を close せず」に、event pump タスク（`spawn_event_pump_task`）が mpsc に積まれた既 emit 済みイベントを処理しきる猶予を与えるための有界待ちである。
- 実装は sessions ロックを保持しない有界ループとする。上限 `CLOSE_DRAIN_TIMEOUT`（既定 200ms、`runtime/usecase.rs` 内の const）まで、短い間隔（例 10ms）で `state.phase` を再読して、`Idle` に落ちたら早期終了する。落ちなければ上限で打ち切って次段（SessionClosed finalize）へ進む。
- drain 中に epoch guard（`apply_runtime_event` の `is_current_runtime`）は state 未 remove のため一致し続けるため、in-flight event は破棄されず適用される。これが RT-1 の「state 先行 remove により in-flight event を捨てる」経路の是正になる。
- drain は backend の応答を無制限には待たない（有界）。上限到達時は SessionClosed で確定的に終端する。
- closed フラグ（emit 抑止）は drain 前に立てない。runtime.close() は finalize の後に呼ぶため、drain 中は backend の最終イベントが正常に emit・適用される。

## データモデルまたは型

### domain `InterruptReason`（`entities/turn.rs`）

```rust
pub enum InterruptReason {
    Abort,
    Timeout,
    Crash,
    SessionClosed, // 追加: 正常な close / backend 切替 / アプリ終了で turn を終端した理由
}
```

### event log `InterruptReason`（`event_log/events.rs`）

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptReason {
    Abort,
    Timeout,
    Crash,
    SessionClosed, // serde 表現: "session_closed"
}

impl InterruptReason {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Abort => "abort",
            Self::Timeout => "timeout",
            Self::Crash => "crash",
            Self::SessionClosed => "session_closed",
        }
    }
}
```

- serde は `rename_all = "snake_case"` の unit variant であり、追加は additive。永続化済み event（`"abort"` / `"timeout"` / `"crash"`）の deserialize は影響を受けない（F3 additive 規約）。
- `TurnInterrupted` イベントの永続表現は `"reason": "session_closed"` となる。

### `terminal_projection` の写像（`runtime/usecase.rs`）

`TurnResult::Interrupted { reason: DomainInterruptReason::SessionClosed, .. }` を次へ写像する:

- `exit_code = 0`
- `session_state = SessionState::Idle`
- `interrupted = true`
- `event = TerminalEventProjection::Interrupted { reason: EventInterruptReason::SessionClosed, error }`

`SessionClosed` は利用者/システムによる正常な終了であり、`Abort` と同様にエラーではない扱い（exit_code 0 / Idle）とする（後述の仮定 A1）。

### `project_status` の分岐（`event_log/projector.rs`）

`TerminalEvent::Interrupted` の既存分岐に `SessionClosed` を追加し、`Abort` と同じく `SessionState::Idle` / `TurnPhase::Idle` に射影する。

```rust
TerminalEvent::Interrupted {
    reason: InterruptReason::Abort | InterruptReason::SessionClosed,
    ..
} => ProjectedStatus { session_state: SessionState::Idle, turn_phase: TurnPhase::Idle },
TerminalEvent::Interrupted { .. } => ProjectedStatus {
    session_state: SessionState::Error, turn_phase: TurnPhase::Idle,
},
```

## 処理フロー

### close_session（是正後）

1. session lock を blocking acquire し、進行中の `send_message` / `respond_permission` 等が完了するまで待つ。既に guard を持つ node lifecycle 経路は guard を解放してから public close を呼ぶ。
2. sessions ロックを取り、`state.closing=true` として新しい外部操作を拒否し、`state.phase`・`state.runtime` を読む（remove しない）。
3. `phase == Idle` かつ runtime なし → 従来どおり state remove のみで終了（新規中断 turn を作らない）。
4. `phase != Idle`（進行中 turn あり）:
   1. `flush_streaming_update(force_persist=true)`。
   2. session lock を一時的に解放して drain（有界待ち、上限 `CLOSE_DRAIN_TIMEOUT`）。event pump だけが terminal event を適用でき、新しい外部操作は `closing` guard で拒否される。途中で `phase == Idle` になれば早期終了。
   3. session lock を再取得し、`complete_turn(.., TurnResult::Interrupted { reason: SessionClosed, error: None })`。
      - 内部で再度 `flush_streaming_update(force=true)` → `terminal_projection` → `append_final_turn_events` → （Interrupted 分岐で）`finalize_turn`。
      - `finalize_turn` が未完了 ToolCall を `ToolCallFailed`（"session_closed により中断"）へ、未解決 permission を `PermissionResolved{ decision: Cancelled }` へ畳み、`TurnInterrupted { reason: SessionClosed, exit_code: 0 }` を記録。
      - 既に terminal がある turn（drain 中に実 `TurnCompleted` が届いた等）は `has_turn_terminal` / phase==Idle で no-op。
   4. `runtime.close()`。
   5. sessions map から state remove。

### close_all（アプリ終了）

- `application_lifecycle.rs` の `shutdown_application_services` が `close_all().await` を `app.exit(0)` 前に await する（既存）。
- 現行の `drain()` 一括 remove をやめ、保持セッションを列挙し、各セッションに対して close_session と同じ終端ヘルパを適用してから remove する。全セッション分の drain が直列だと終了が遅延しうるため、セッション単位の finalize は並行実行（`join_all` 等）し、各々の drain 上限は `CLOSE_DRAIN_TIMEOUT` で有界に保つ。

### set_session_backend（backend 切替）

- `set_session_backend` 全体を同じ per-session transition として直列化する。まず旧 runtime を close と同じ手順で finalize/remove し、その完了後にだけ新しい backend selection を永続化する。drain 中は session lock を一時解放するが `closing=true` により競合する `send_message` を拒否し、finalize/remove から backend selection 更新までは再取得した lock を保持する。
- provider 切替に伴う configuration handoff の再設計は行わない（非スコープ）。

### 再オープン時の観測

- event log を `TurnEventLog::from_events(...).project()` で再構成すると、`TurnInterrupted { reason: SessionClosed }` が `TerminalEvent::Interrupted { reason: SessionClosed }` に射影され、status は `Idle`、ToolCall は失敗（中断）、permission は `Cancelled` として現れる。
- terminal append と同時に `SessionMeta.last_turn_interruption` を更新する。`GetSessionResponse.lastTurnInterruption` はこの軽量 projection から、最新 turn が interrupted terminal を持つ場合だけ agent message id と reason を返す。再オープン・再起動後も durable meta から復元され、会話ビューは対象 message に中断チップを描画する。

## エラー処理

- `flush_streaming_update(force=true)` / terminal event batch append / terminal message parts / terminal session state のいずれかの永続化に失敗した場合、`complete_turn` から `close_session` / `set_session_backend` / `close_all` へエラーを伝播する。runtime close と state remove は行わず、in-memory turn state を保持して再試行可能にする。
- terminal event 群は、test hook を全 event に事前適用した後、temp batch segment を完全書込みし、atomic rename で commit する。既存 event prefix は再構築しない。未 commit の temp segment は全体を不可視とし、batch commit 後の message/meta 更新に失敗した場合も、既存 terminal を検出して重複 append せず後続永続化だけを再試行する。
- `close_all` の一部 session が失敗した場合は全 session の試行を完了した後に集約エラーを返す。application shutdown はエラー時に process exit へ進まず、terminal guarantee を silent success にしない。
- drain の上限到達は正常系（エラーではない）。上限で打ち切って SessionClosed finalize に進む。
- runtime が存在しない（既に閉じている）セッションでは finalize をスキップし、state remove のみ行う。
- `complete_turn` は phase / generation / `has_turn_terminal` の各 guard により idempotent。二重終端・Crash 上書きは発生しない。

## テスト方針

Rust のシナリオテスト（`runtime/usecase.rs` の `#[cfg(test)]`、および `event_log` / `projector` の該当 module）で固定する。外部プロセスは起動せず、`AgentSessionRuntime` のテスト double（既存の fake runtime）と `SessionStore`（temp data_dir）を用いる。

1. **streaming 中 close → 再オープン**（RT-1 主シナリオ / behavior Rule 群）:
   - streaming 本文を出し、未完了 ToolCall と未送信 `Pending` permission を持つ turn を進行中にする。
   - `close_session` を呼ぶ。
   - event log を再ロード・project して検証:
     - `TurnInterrupted { reason: SessionClosed }` が記録されている。
     - `run_in_background=true` の未完了 Task が `ToolCallFailed` と `TaskStatusChanged(stopped)` に畳まれ、実行中スピナー相当が残らない。
     - 未送信 permission が `PermissionResolved { decision: Cancelled }` に畳まれ、`latest_unresolved_permission_request` が `None`。
     - 最後のスナップショット以降のストリーミング本文・pending parts がメッセージに反映されている（flush 済み）。
     - status が `Idle`（`Active/Streaming` に残らない）で、query response に message id と `SessionClosed` reason が載る。
2. **進行中 turn が無いセッションの close**: 新規中断 turn が生成されず、履歴が保持されること。
3. **backend 切替（`set_session_backend`）**: 進行中 turn が finalize（reason=SessionClosed）されること。
4. **アプリ終了（`close_all`）→ 再ロード**: 保持セッションの進行中 turn が finalize され、再構成後に残骸が無く中断（SessionClosed）で終端していること。
5. **drain（close と競合する最終イベント）**: close 手順の途中で fake runtime に `TurnCompleted`（正常完了）を emit させ、drain 窓内に適用されて turn が正常 `Completed` で終端し、SessionClosed で上書きされないこと（idempotent）。
6. **SessionClosed は Crash を上書きしない**: 既に `TurnInterrupted { reason: Crash }` を持つ turn に close 経路が関与しても reason が SessionClosed に変わらないこと（`has_turn_terminal` guard）。
7. **event log 後方互換**: `"abort"` / `"timeout"` / `"crash"` を含む既存 event の deserialize / project が SessionClosed 追加後も不変であること（`event_log/tests.rs`）。
8. `project_status` / `terminal_projection` の SessionClosed 分岐 unit test。
9. `send_message` / `respond_permission` と close の競合で、close が先行操作の完了を待ち、確定済み応答を Cancelled で上書きしないこと。
10. **backend 切替との競合**: transition 中の `send_message` が拒否され、旧 runtime の finalize/remove 前に新 backend metadata が公開されず、切替後の runtime state と durable backend が一致すること。
11. **bounded query**: 長い event log を持つ session でも `get_session` が event history をロードせず、meta projection から `lastTurnInterruption` を返すこと。
12. **永続化失敗の再試行**: force flush、terminal batch append、terminal commit 後の message parts persist をそれぞれ失敗注入し、close がエラーを返して runtime/state を保持すること。再試行後に terminal が一件だけ記録され close が完了すること。
13. **session 間の分離**: 一方の session の TurnStarted 永続化を停止しても、他方の session が close/state 遷移できること。TurnStarted の採番・meta 射影で event history をロードせず、global sessions lock を保持しないこと。
14. **application lifecycle 境界**: agent finalize → workflow command shutdown → local API shutdown の成功順序と、agent finalize 失敗時に後続 port を呼ばないこと。
15. **terminal batch fault**: batch の各 event 境界で temp segment が途切れても新規 event は 0 件だけ可視であり、再試行 commit 後は全件が一度ずつ可視になること。

CI 同一コマンド（`pnpm lint` / `pnpm test` / `pnpm build` / `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`）が通ること。

## リスクと代替案

- **drain 上限の妥当性**: 200ms は「短時間の有界待ち」の初期値。短すぎると競合最終イベントの取りこぼし（ただし SessionClosed finalize が保険）、長すぎるとアプリ終了が遅延する。`phase==Idle` 早期終了と `CLOSE_DRAIN_TIMEOUT` の const 化で調整可能にする。代替案として drain を「backend からの明示的 result 到達 or timeout の select」にする案もあるが、既 emit 済みイベントの適用は event pump タスク経由で非同期に進むため、phase ポーリングの方が結合が薄く堅い。
- **emit 抑止の解除はしない**: closed フラグを触ると crash 検出（RT-2）や既存の abort 経路に影響が及ぶ。本設計は close 前に runtime を開いたまま finalize することで terminal event を確実に適用し、抑止解除を回避する。代替（抑止解除）は RT-2 / #1407 との干渉リスクが高く却下。
- **`close_all` の並行 finalize**: 多数セッション時の終了遅延を避けるため finalize を並行化する。共有 `RuntimeContext` への並行アクセスは既存の per-session ロックで直列化されるため安全。
- **中断チップの reason 表示粒度**: reason の判定は Rust read model が行い、フロントエンドは `SessionClosed` を表示用文言へ整形するだけとする。見た目は behavior の対象外。

## 仮定

- **A1**: `SessionClosed` は正常な利用者/システム操作による終了であり、`terminal_projection` / `project_status` では `Abort` と同様にエラー扱いしない（exit_code 0 / `SessionState::Idle`）。
- **A2**: drain の上限は `CLOSE_DRAIN_TIMEOUT = 200ms`（ポーリング間隔 10ms）を初期値とする。無制限に close をブロックしない有界待ちである。
- **A3**: closed フラグ（backend の terminal event emit 抑止）は変更しない。close 前に runtime を開いたまま finalize を済ませることで terminal event の適用を保証する。
- **A4**: 対象は正常な `close_session` / `set_session_backend` / `close_all` の三経路に閉じる。プロセスクラッシュ・強制終了時の起動時 dangling turn 回収（`Crash` 理由）は RT-2 / #1407 の非スコープであり、`SessionClosed` は `has_turn_terminal` / phase guard により `Crash` 理由を上書きしない。
- **A5**: streaming flush の損失許容窓は現行の定期 flush 間隔（1 秒）に準ずる。close 時は `force_persist=true` によりこの窓内までの本文を確実に durable 化する。
- **A6**: write-ahead 済み（`Responding` / `Resolving`）permission の扱いは既存 `finalize_turn` 経路の規約に従う。本設計は未送信 `Pending` の `Cancelled` 畳み込みのみを新規保証として固定する。
- **A7**: `set_session_backend` は現行 UI 上「空セッション限定」で主経路ではないが、監査 RT-1 の指摘に従い finalize 手順の適用対象に含める。provider 切替の configuration handoff 再設計は行わない。
- **A8**: フロントエンドは `close_session` を invoke する薄い経路のままとし、interrupt 判断・finalize は Rust（usecase / domain）が所有する（rust-first-logic）。

## Open Questions

なし。
