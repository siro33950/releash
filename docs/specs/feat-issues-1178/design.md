# Design

requirements.md / behavior.md を満たすための実装設計。Claude Agent SDK 経路（`claude-sdk-bridge.mjs` + `bridge_common.rs`）における stale turn 検出・復旧機構を、既存の turn ライフサイクルへ最小侵襲で組み込む。

## 概要

現状、turn 完了は SDK/bridge からの `result` → bridge の `turn_complete` 出力 → Rust 側 `run_turn_complete_transition_locked()`（`TurnPhase::Streaming` → `Idle`）だけで成立しており、`result` が来なければ永続 Thinking になる。

本設計では Rust runtime（`AgentProcess`）に **進捗時刻（liveness）** を記録し、turn ごとに **watchdog（tokio タスク）** を起動して、進捗のない `Streaming` turn / 一定経路の `WaitingPermission` turn を **stale** と判定する。stale 検出時は既存の終端遷移（`run_turn_complete_transition_locked`）へ **失敗 exit_code** で合流させて UI を回復させ、続けて bridge へ interrupt → grace 後に応答しなければプロセスグループを kill して次 turn 用に破棄する。`result` なしで bridge loop が閉じた場合も失敗完了として扱う。これらの遷移はすべて既存の per-session runtime lock 下で原子的に行い、Stop との二重完了を防ぐ。

正常系（`result` 受信）の挙動は一切変えない。

## 変更対象

### Rust（実装の中心）

- `src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs`
  - `AgentProcess` 構造体: liveness / origin / watchdog 管理フィールドを追加。
  - stdout reader ループ: 進捗イベント受信時に liveness を更新。
  - watchdog タスクの spawn / tick ロジックを新設。
  - stale 完了の終端遷移ヘルパー（`run_turn_complete_transition_locked` を失敗 exit_code で呼ぶ薄いラッパ）と冪等ガードを追加。
  - bridge 停止/破棄（interrupt → grace → `sweep_process_group`）の復旧シーケンスを追加。
  - `spawn_bridge_process()`: native の idle 防御を turn timeout と整合させる env を bridge プロセスに設定（`CLAUDE_STREAM_IDLE_TIMEOUT_MS` / `CLAUDE_ENABLE_STREAM_WATCHDOG` / `CLAUDE_ENABLE_BYTE_WATCHDOG` / `CLAUDE_CODE_MAX_RETRIES` / `API_TIMEOUT_MS`）。`CLAUDE_CODE_STREAM_CLOSE_TIMEOUT` は CLI v2.1.112 に存在しないため使わない。
  - 定数追加: `STALE_TIMEOUT_SECS`、`PERMISSION_TIMEOUT_SECS`、`STALE_RECOVERY_GRACE_SECS`、`WATCHDOG_TICK_SECS`。
  - `turn_origin_for_session()` で `ChatSession.workflow_step_session` を参照し、runtime 内で `Desktop` / `Headless` を導出する。`AgentMessageDispatchRequest` や remote handler へ origin フィールドは追加しない。

### Node bridge

- `src-tauri/resources/claude-sdk-bridge.mjs`
  - `query()` ループが `result` を受け取らずに終了（非 interrupt）した場合、`turn_complete` を **失敗 exit_code（非 0）** で emit する。
  - 既存の user-interrupt 経由の正常 abort（exit_code 0）は維持。

### フロント（表示のみ）

- 既存の `agent-session-state-changed`（`TurnPhase::Idle` + 失敗 exit_code）受信で永続 Thinking から復帰する経路は変更不要。stale 完了時に「Claude 応答が停止したため中断した。再試行できる」旨を表示するため、合成エラー part（後述）を `streaming_parts` に積んで既存の error part 表示経路へ載せる。新規 UI ロジックは持たせない（ロジックは Rust）。

## アーキテクチャと責務分割

```
[runtime: bridge_common.rs]
   send_agent_message_internal / start_agent_turn_*  ──┐
        │ turn 開始時:                                  │
        │   last_progress_at = now                       │
        │   turn_phase_since = now                       │
        │   turn_origin = turn_origin_for_session(session)│
        │   spawn_turn_watchdog(generation_id)  ─────────┘
        │
   stdout reader loop（進捗イベント）
        │   thinking/text/tool/progress delta 受信 → touch_liveness()
        │   permission_request → turn_phase_since = now
        │   turn_complete(result) → 正常終端（既存経路、watchdog 自然終了）
        │   EOF / result なし close → 失敗終端
        ▼
   turn watchdog（tokio task, per turn, generation_id で識別）
        │   WATCHDOG_TICK_SECS ごとに lock 取得して判定:
        │     Streaming かつ now - last_progress_at > STALE_TIMEOUT      → stale
        │     WaitingPermission かつ origin == Headless
        │            かつ now - turn_phase_since > PERMISSION_TIMEOUT     → timeout
        │   検出時:
        │     1) finalize_turn_as_failure_locked()（冪等・lock 下）→ UI 回復
        │     2) bridge interrupt 送信
        │     3) grace 後 Ready 未復帰なら sweep_process_group + map から破棄
        ▼
   既存: run_turn_complete_transition_locked()（TurnPhase::Idle, 失敗 exit_code）
```

責務:

- **検出** は watchdog タスクのみが担う（タイマー判定を一箇所に集約）。
- **終端遷移** は既存 `run_turn_complete_transition_locked()` に一本化し、stale 経路も同じ終端状態（`Idle` + 失敗 exit_code）へ収束させる（正常系と同じ最終形）。
- **bridge 破棄** は既存 `sweep_process_group()` / PGID 機構（`killpg`）を踏襲。
- **排他** は既存 `acquire_session_runtime_lock()` を使い、Stop / turn_complete / watchdog のすべてが同一 lock 下で終端遷移を行う。

## データモデルまたは型

### `AgentProcess` への追加フィールド（`bridge_common.rs`）

```rust
pub struct AgentProcess {
    // 既存 ...
    pub turn_phase: TurnPhase,
    pub generation_id: u64,            // 既存。watchdog と turn を対応付ける鍵として再利用

    // 追加:
    pub last_progress_at: Option<Instant>, // 最後に進捗イベントを受信した時刻
    pub turn_phase_since: Instant,         // 現 turn_phase に入った時刻（permission timeout 用）
    pub turn_origin: TurnOrigin,           // 現 turn の種別（desktop / headless）
    pub watchdog_active: bool,             // 現 turn の watchdog が稼働中か（多重 spawn 防止）
}
```

- `last_progress_at` は turn 開始時に `now` で初期化し、進捗イベントごとに `now` で更新。`Streaming` 以外では判定に使わない。
- `turn_phase_since` は `Streaming` / `WaitingPermission` への遷移時に `now` を記録。
- `generation_id` は既存の per-spawn 採番をそのまま使う。watchdog 起動時に値を capture し、tick 時に proc の現 `generation_id` と一致する場合のみ作用する（プロセス再生成・turn 入れ替わり後の誤作動を防ぐ）。さらに turn 単位の入れ替わりを区別するため、turn 開始ごとにインクリメントする `turn_seq: u64` を併用する（同一プロセス内で複数 turn が走るため、generation_id だけでは turn を一意にできない）。

### `TurnOrigin`（新規 enum）

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TurnOrigin {
    Desktop,         // Tauri command 経由。permission は無期限待ち
    Headless,        // workflow step / 自律実行。permission timeout 適用
}
```

- `Desktop` は permission timeout を適用しない（人が応答しうる）。
- `Headless` は permission timeout を適用する。
- Headless 判定は既存 `ChatSession.workflow_step_session` を流用し、true なら `Headless` とする。

### 合成エラー part

- stale 完了時、ユーザー向け文言を持つ `MessagePart::Error`（既存バリアント）を `streaming_parts` に push し、force flush + persist する。
- 文言（固定）: 「Claude 応答が停止したため中断しました。もう一度お試しください。」
- これにより behavior.md の「『Claude 応答が停止したため中断した。再試行できる』旨が表示される」を、既存 error part 表示経路で満たす。

### 定数（`bridge_common.rs`）

```rust
const STALE_TIMEOUT_SECS: u64 = 180;          // Streaming 無進捗の許容
const PERMISSION_TIMEOUT_SECS: u64 = 300;     // headless の permission 待ち許容
const STALE_RECOVERY_GRACE_SECS: u64 = 10;    // interrupt 後に Ready 復帰を待つ猶予
const WATCHDOG_TICK_SECS: u64 = 5;            // watchdog の判定間隔
```

native（`claude` CLI v2.1.112）の idle 防御を Releash の `STALE_TIMEOUT_SECS` と整合させるため、bridge env に以下を設定する（当初想定の `CLAUDE_CODE_STREAM_CLOSE_TIMEOUT` は **CLI に存在しない env** であり無効。requirements.md「native 側の既存防御と盲点」参照）。

| env | 役割 | 設定方針 |
|---|---|---|
| `CLAUDE_STREAM_IDLE_TIMEOUT_MS` | watchdog の idle 閾値 | event 層は**生値**を使用（既定 90000ms、clamp なし）。byte 層のみ `Math.max(…, 300000)` で 300000ms に下限 clamp。event 層を効かせたいので `STALE_TIMEOUT_SECS` 近傍（例 90000〜150000）に設定。byte 層は 300000ms 下限のまま |
| `CLAUDE_ENABLE_STREAM_WATCHDOG` | event 単位 watchdog（既定オフ）の有効化 | **`1` で有効化する**。有効時の挙動はコード上確定済み: 実イベント毎にタイマーをリセットし（SDK が `if(w.event==="ping")continue` で **ping を yield しないため ping ではリセットされない**）、`u6/2` で warning、`u6` 満了でストリームを abort・部分出力フラッシュ・clean な idle-timeout error を throw（`cli_streaming_idle_timeout` / `cli_stream_loop_exited_after_watchdog`）。これにより native 自身が ping 生存・content 停止の stall を検出して turn を失敗終端させられる |
| `CLAUDE_ENABLE_BYTE_WATCHDOG` | byte 単位 watchdog（既定オン）の制御 | 無効化しない（既定オンを維持） |
| `CLAUDE_CODE_MAX_RETRIES` | API リトライ回数（既定 10） | 既定維持（transient 失敗の透過リトライは native に委ねる） |
| `API_TIMEOUT_MS` | 1リクエスト上限（既定 600000ms） | 既定維持または turn timeout と整合する値を検討 |

> 重要: native の byte watchdog は SSE `ping` でリセットされ ping 生存 stall を拾えないが、event watchdog（`CLAUDE_ENABLE_STREAM_WATCHDOG=1`）は ping を除外するため拾える。それでも最終的な stale 検出は Rust 側 watchdog（進捗イベントベース）が担う。理由: (1) native の throw が agent-sdk `query()` の内部リトライ（既定 maxRetries まで）で握られると、bridge に伝播するまで最悪 `u6 × リトライ回数` かかりうる、(2) bridge/SDK プロセス自体がハングした場合は native watchdog では検出不能。Rust watchdog（`STALE_TIMEOUT_SECS=180`）はこれらの外側上限として機能する。env 設定は native の一次防御を働かせる補助であり、Rust watchdog を置き換えない。

## 処理フロー

### turn 開始（正常時の初期化）

1. `start_agent_turn()` / `start_agent_turn_locked()` が `turn_origin_for_chat_session()` を呼び、`ChatSession.workflow_step_session` から `Desktop` / `Headless` を導出する。
2. `send_agent_message_internal()` / `start_agent_turn_*_locked()` で turn が `Streaming` に入る際、lock 下で:
   - `last_progress_at = Some(now)`、`turn_phase_since = now`、`turn_origin = derived_origin`、`turn_seq += 1`。
   - watchdog 未稼働なら `spawn_turn_watchdog(generation_id, turn_seq)`。

### 進捗イベント受信（liveness 更新）

stdout reader ループの既存 dispatch に touch を追加する。

- 進捗として扱う（`last_progress_at = now`）: `thinking` delta、`text` delta、`tool_use` / `tool_result`、`progress` 通知。既存の streaming accumulate 経路（`accumulate_sdk_message()`）に入るメッセージはすべて liveness とみなす。
- 進捗として扱わない: stderr のみの api/request ログ、`session_ready`、`supported_commands` 等の制御系。stderr reader からは liveness を更新しない。
- `permission_request` 受信時は `turn_phase_since = now`（WaitingPermission 起点）に更新。`Streaming` の `last_progress_at` 判定からは外れる。

### watchdog tick（検出）

`tokio::spawn` した per-turn タスクが `WATCHDOG_TICK_SECS` 間隔で:

1. lock 取得（`acquire_session_runtime_lock`）。
2. proc の `generation_id` / `turn_seq` が capture 値と不一致、または `turn_phase == Idle` → この turn は既に終わっている。watchdog 終了。
3. 判定:
   - `turn_phase == Streaming` かつ `now - last_progress_at > STALE_TIMEOUT_SECS` → **stale**。
   - `turn_phase == WaitingPermission` かつ `turn_origin == Headless` かつ `now - turn_phase_since > PERMISSION_TIMEOUT_SECS` → **permission timeout**。
   - `turn_phase == WaitingPermission` かつ `turn_origin == Desktop` → 何もしない（無期限待ち）。
4. 非検出ならそのまま次 tick。検出時は recovery へ。

### recovery（復旧シーケンス）

検出した watchdog tick 内（lock 保持中）で:

1. `finalize_turn_as_failure_locked()` を呼ぶ:
   - 既に `Idle`（Stop / 正常 turn_complete が先着）なら **no-op**（冪等ガード）。
   - 合成エラー part を `streaming_parts` に push。
   - `run_turn_complete_transition_locked(proc, csid, exit_code = STALE_EXIT_CODE, emit)` を呼び、`TurnPhase::Idle` + `BridgeState::Crashed`（失敗）へ遷移。`STALE_EXIT_CODE` は非 0 の専用値（例: `124`、timeout 慣例）。
   - `agent-session-state-changed(Idle, exit_code=STALE_EXIT_CODE)` と `agent-streaming-updated` を emit（UI 即時回復）。
2. lock を解放し、bridge 復旧をバックグラウンドで実施:
   - captured `generation_id` / `turn_seq` が現在の proc と一致する場合のみ `write_bridge_command_for_captured_turn(interrupt)` を送信する。一致しない場合は新しい turn を中断しない。
   - interrupt を送信できた場合は `STALE_RECOVERY_GRACE_SECS` 待機する。
   - 再 lock し、proc の `generation_id` が変わらず `BridgeState` が `Ready` に戻っていなければ（＝ bridge が interrupt に応答していない）:
     - `sweep_process_group(pgid)`（SIGTERM → SIGKILL）でプロセスグループを kill。
     - `remove_pgid()` で PID ファイル削除、map から proc を除去（または `Crashed` 固定）。
   - これにより次 turn は `ensure_runtime_for_turn()` で **新規 bridge を spawn** し、壊れた SDK client 状態を持ち越さない。
3. watchdog タスク終了。

> stale 完了を **先に** 行い、bridge kill を後追いにする理由: UI の永続 Thinking を最優先で解消するため。bridge が interrupt に応答して自然に `turn_complete` を出すケースでは、その turn_complete は既に `Idle` なので冪等ガードで no-op になり二重完了しない。

### `result` なしで bridge loop が閉じた場合

- bridge 側: `query()` ループが `result` 未受信のまま終了し、かつ user-interrupt 由来でない場合、`turn_complete` を **exit_code = 非 0** で emit するよう `claude-sdk-bridge.mjs` を変更。
- Rust 側: 既存 `turn_complete` 経路がそのまま失敗終端（`Crashed` + `Idle`）に落とす。
- bridge プロセス自体が落ちて stdout が EOF になった場合は `run_bridge_eof_crash_transition_locked()` が `run_turn_complete_transition_locked()` に合流し、`Crashed` + `Idle` に遷移させる。この経路の exit_code が 0（正常）になっていないか実装時に確認し、失敗 exit_code を保証する（behavior「正常終了扱いにしない」）。

### Stop との競合

- Stop（`interrupt_agent_query`）は bridge へ interrupt を送り、bridge が `turn_complete`（exit 0）を返して既存経路で完了する。
- Stop と watchdog の stale 完了がほぼ同時でも、両者とも `acquire_session_runtime_lock` 下で `run_turn_complete_transition_locked` 系を呼び、**先着が `Idle` にした後は後着が冪等 no-op** になる（`turn_phase == Idle` ガード）。二重完了・二重エラーは発生しない。

## エラー処理

- **lock 取得失敗 / proc 消失**: watchdog tick で proc が map から消えていれば（セッションクローズ等）watchdog は静かに終了する。
- **interrupt 送信失敗**（stdin closed）: 既に bridge は死んでいるとみなし、grace を待たず即 `sweep_process_group` へ進む。
- **kill 失敗**: 既存 orphan cleanup（`cleanup_orphan_processes()`）が次回起動時に PID ファイルから回収する safety net を踏襲。
- **STALE_EXIT_CODE**: 正常系（0）・既存クラッシュ（1）と区別できる専用値（`124`）にし、フロントの表示分岐や将来のメトリクスで「stale 起因の失敗」を識別可能にする。ただし最終 `TurnPhase` は既存と同じ `Idle` で、非 0 はすべて失敗表示として扱う（既存挙動を変えない）。
- **部分出力の保持**: `run_turn_complete_transition_locked()` は既に `streaming_parts` をスナップショットし persist する。stale 経路も同関数を通すため、合成エラー part を含む部分出力が失われない。

## テスト方針

Rust unit test（`bridge_common.rs` の `#[cfg(test)]`）を中心に、既存のテスト用 helper（emit closure 注入パターン）と時刻注入で時間を制御する。`Instant::now()` 依存を避けるため、watchdog 判定ロジックを「現在時刻・最終進捗時刻・turn_phase・origin・経過秒を受け取り判定結果を返す純関数」`evaluate_turn_liveness(...)` に切り出し、これを単体テストする（時間を実際に待たない）。

検証ケース（requirements / behavior 対応）:

1. **stale 失敗完了**: `Streaming` で `last_progress_at` から 180 秒超過 → `evaluate_turn_liveness` が `Stale` を返し、`finalize_turn_as_failure_locked` で `TurnPhase::Idle` + 非 0 exit_code に遷移する（behavior: 終了通知も進捗も返らない turn）。
2. **進捗継続で非 timeout**: thinking / tool / progress / text delta を 180 秒未満間隔で touch し続けると `Stale` にならない（Scenario Outline 4 種を網羅）。
3. **最後の進捗からの計測**: 過去に進捗があっても、最後の touch から 180 秒超過で `Stale`。
4. **api/request 補助ログは非進捗**: stderr 由来ログでは `last_progress_at` が更新されず stale 判定に至る。
5. **result なし close**: `turn_complete(exit_code != 0)` / EOF crash で `Idle` + 失敗、`Ready` へ戻らないこと（次 turn が再 spawn する状態）。
6. **bridge 再生成**: stale 後に interrupt → grace 内で Ready 未復帰なら破棄され、次 turn が新規 spawn される（map から proc が除去される / `Crashed` 固定であることを確認）。kill 呼び出しはテストでは実プロセスを起動せず、proc 状態と PID ファイル操作の分岐で検証する。
7. **permission timeout（headless のみ）**: `WaitingPermission` + `Headless` で 300 秒超過 → `PermissionTimeout`。`Desktop` では超過しても `Ok`（無期限待ち）。
8. **二重完了防止**: `Idle` 済みの proc に `finalize_turn_as_failure_locked` を呼んでも no-op（既存 message / state が変わらない）。Stop 先着 → watchdog 後着の順で完了が一度きりであること。
9. **正常系不変**: `result` → `turn_complete(0)` の既存遷移テストが従来どおり通り、watchdog が誤検出しない。

統合テスト（任意・可能なら）: 無音 stall を模した stub bridge スクリプト（`result` を返さず一定時間ハングする `.mjs`）でセッションを起動し、180 秒未満に短縮した timeout 定数（テスト用に env / feature で短縮可能にするか、判定純関数経由で検証）で stale 完了→次メッセージ送信可を確認。なお実 180 秒を待つテストは CI で回さない（純関数テストでカバー）。

## リスクと代替案

- **誤中断リスク（正当に長い Opus thinking）**: thinking delta を確実に liveness に含めることで緩和。180 秒は保守的設定。SDK が thinking 中に delta を一切出さないモデル設定があると誤検出し得る → 実装時に Opus 4.8 の thinking delta 出力有無を確認し、必要なら timeout を引き上げる（定数で調整可能）。
- **watchdog の多重起動**: `watchdog_active` フラグ + `generation_id`/`turn_seq` capture で防止。turn 完了時にフラグを倒す。
- **代替案A（watchdog を既存 streaming timer に相乗り）**: `spawn_streaming_timer()` は「pending delta があり Streaming のとき」だけ回り、無進捗（pending 0）では自然停止するため stale 検出に使えない。→ 専用 watchdog を採用。
- **代替案B（絶対 timeout のみ）**: 進捗無視の固定上限。正当に長い thinking を切るため不採用。idle（無進捗）ベースを採用。
- **代替案C（bridge 側 watchdog）**: bridge 内で無音検出。だが「全ロジックは Rust」方針と、bridge 自体がハングした場合に検出不能なため Rust 側に置く。bridge は native の idle 防御 env 設定（上表）と result なし close の失敗 emit のみ担う。
- **代替案D（native の防御に全面依存）**: `claude` CLI v2.1.112 はリトライ（既定10回）・byte watchdog（既定オン、最低300秒）・event watchdog（既定オフ）を持つ。event watchdog を `CLAUDE_ENABLE_STREAM_WATCHDOG=1` で有効化すれば ping 生存 stall も検出でき、これを一次防御に併用する。ただし native 防御のみへの全面依存は不採用: (1) リトライは HTTP エラー応答時のみ発火し無音 stall を拾わない、(2) byte watchdog は SSE `ping` でリセットされ content 停止を拾えない、(3) event watchdog の throw が agent-sdk 内部リトライで握られると bridge 伝播が遅延しうる、(4) bridge/SDK 自体のハングは native では検出不能。よって Rust 側 watchdog を最終手段として採用し、native 防御は env で併用する（requirements.md「native 側の既存防御と盲点」参照）。
- **origin 判定の境界**: handler/dispatcher/gateway へ origin を引き回すと変更範囲が広がるため採用しない。runtime が `ChatSession.workflow_step_session` から `Desktop` / `Headless` を導出し、未取得時はエラーとして turn 開始を止める。

## 仮定

- 対象は Claude Agent SDK 経路（`claude-sdk-bridge.mjs` + `bridge_common.rs`）に限定。他 backend は対象外。
- timeout は固定定数（stale 180 秒 / permission 300 秒 / grace 10 秒 / tick 5 秒）。ユーザー設定 UI は設けない。
- 失敗完了後の回復はユーザーの手動再試行。自動リトライ / 自動 resume は行わない。
- turn の origin は runtime 内で `ChatSession.workflow_step_session` から導出する。`workflow_step_session == true` のセッションは `Headless` として permission timeout を適用し、それ以外は `Desktop` として permission を無期限に待つ。
- 進捗イベントは thinking / text / tool / progress delta を含み、stderr のみの api/request ログは含めない。
- `STALE_EXIT_CODE` は非 0 の専用値（`124`）。最終 `TurnPhase` は既存と同じ `Idle`、非 0 は失敗として既存表示経路で扱う。
- bridge env は実在する native lever（`CLAUDE_STREAM_IDLE_TIMEOUT_MS` / `CLAUDE_ENABLE_STREAM_WATCHDOG` / `CLAUDE_ENABLE_BYTE_WATCHDOG` / `CLAUDE_CODE_MAX_RETRIES` / `API_TIMEOUT_MS`）で設定する。`CLAUDE_CODE_STREAM_CLOSE_TIMEOUT` は CLI v2.1.112 に存在しないため使わない。`CLAUDE_ENABLE_STREAM_WATCHDOG=1` 有効時の挙動はコード調査で確定済み（event 層タイマー、ping 非リセット、abort＋部分フラッシュ＋clean throw、`CLAUDE_STREAM_IDLE_TIMEOUT_MS` は event 層で clamp なし）。native の throw が agent-sdk 内部リトライで握られた場合の伝播遅延のみ実機で確認するが、Rust watchdog（180秒）が外側上限を担保するため設計判断には影響しない。
- watchdog 判定の中核は純関数 `evaluate_turn_liveness` に切り出し、時間を実待ちせず単体テストする。
- Spec ディレクトリ ID は `docs/specs/feat-issues-1178`。

## Open Questions

なし
