# Design: persist 失敗の可視化と event log 自己修復（#1409）

## 概要

milestone 84「Agent チャット安定化」Phase 0 として、Agent チャット（Claude / Codex）の永続化経路にある無言失敗を 3 件解消する。

- **ST-4**: production 永続化経路の `let _ =` による失敗握りつぶしを廃し、「短い backoff で有限回リトライ → 継続失敗はエラー伝搬＋可視化＋構造化ログ」の統一挙動へ置換する。
- **RT-4**: event log の append 経路に、読み取り側と同等の末尾破損自己修復を追加し、修復後は append を継続できるようにする。
- **RT-8**: `complete_turn` の最終永続化で `FinalPartsRecorded` の append が失敗したとき、projection 由来の tool-only parts で persist 済み本文（Text/Thinking を含む）を上書きしない。

可視化は本 ISSUE では **session-scoped バナー（transient 許容の唯一の例外）＋構造化ログ**に限定する。バナーは backend-owned state から供給し、frontend は表示に徹する（requirements A2、rust-first-logic）。`Notice(PersistFailure)` 語彙への置換は S5（#1393）が担当するため、その置換点となる **backend 側の単一の接合関数**を用意する。

## 変更対象

| 対象 | 種別 | 目的 |
|---|---|---|
| `usecase/agent_session/runtime/usecase.rs` | 修正 | ST-4（`:3378` / `:3534`）と RT-8（`complete_turn` / `append_final_turn_events`）の挙動置換、可視化呼び出し |
| `usecase/agent_session/session/store.rs` | 追加/修正 | persist リトライヘルパの提供、または呼び出し側へのエラー返却経路の確認 |
| `adaptor/gateway/agent_session/session_storage/event_store.rs` | 修正 | append 側の末尾破損自己修復（RT-4） |
| `usecase/agent_session/status.rs`（または runtime/ports.rs） | 追加 | session-scoped notice（バナー）の backend-owned state と通知経路（S5 置換点） |
| `protocol/` + `ws_bridge.rs` / `ws_server/`（該当 presenter/controller） | 追加 | notice を WebSocket / Tauri で frontend へ供給する DTO・配信 |
| `src/`（該当 hook / component） | 追加 | notice バナーの表示のみ（ロジックなし） |
| 各対象 module の `#[cfg(test)]` / 既存 `session_storage/event_store.rs::tests` / 既存 `test_support`（`set_append_event_hook_for_test`） | 追加 | R4 の固定テスト |

非スコープ（requirements の非スコープに従う）:

- RT-7 のキュー回収再設計（`:3378` は握りつぶし解消のみ）。
- `Notice(PersistFailure)` 語彙の正式導入（S5）。
- `event_log/projector.rs:632` の非 persist な `let _ =`、テストコード内 `let _ =`（`:4792`）。
- frontend の恒久エラー表示・チャット内 Error block（FE-2 等別 ISSUE）。

## アーキテクチャと責務分割

レイヤー責務（`src-tauri/AGENTS.md`）に従う。

- **domain**: 変更なし。破損検出・修復・リトライは「永続化 I/O の技術的関心事」であり domain ロジックではない。
- **usecase（runtime/usecase.rs, session/store.rs）**: リトライ回数・backoff・エラー伝搬・可視化トリガーの業務手順を所有する。ST-4 / RT-8 の挙動はここで確定する。
- **adaptor/gateway（event_store.rs）**: event log ファイルの物理修復（RT-4）を所有する。読み取り側の `recover_unclosed_session_events` を append 経路でも使えるようにする。
- **adaptor/protocol・presenter・controller**: notice DTO と配信。
- **frontend**: notice バナーの描画のみ。

### 可視化（バナー）の所有と接合点

- notice は backend が所有する。`AgentStatusCenter` に session-scoped の最新 notice を保持する `RwLock<HashMap<session_id, SessionNotice>>` を追加する（transient: durable 保存はしない。requirements の「唯一の例外」に一致）。
- 供給は 2 経路:
  - **push**: `AgentSessionEventNotifier` に `persist_notice(session_id, SessionNotice)` を追加し、発生時に即時配信する。
  - **snapshot**: 既存 session status snapshot 読み取りに notice を同梱し、reconnect / 再オープン後も直近 notice を再取得できるようにする（backend-owned state を任意 client が読める形にする、という原則に沿う）。
- **S5 置換点**: 発火は usecase 内の単一ヘルパ `report_persist_failure(session_id, ctx, PersistFailureKind)` に集約する。S5 ではこのヘルパ内部を `Notice(PersistFailure)` 生成へ差し替えれば済むようにし、呼び出し側（ST-4 / RT-4 / RT-8）は変更不要にする。

### リトライの責務配置

- `append_session_event_and_project_state` / `set_session_state` / `append_session_event`（`store.rs` / `event_store.rs`）は **単発の Result を返す責務のまま**にする（リトライを内部に隠さない）。
- リトライは **async な呼び出し側（runtime/usecase.rs）** で行う。対象の store メソッドは sync だが呼び出し文脈は async のため、試行間の backoff は `tokio::time::sleep` で待機し、tokio worker をブロックしない。
- 共通ヘルパ `persist_with_retry(op, kind, ctx, session_id)` を usecase に置く。`op: impl FnMut() -> Result<T, String>` を N 回試行し、成功で `Ok(T)`、全失敗で最後の Err を返す。全失敗時にヘルパ内で `report_persist_failure` を呼び、構造化ログを出す。
- event append と meta projection は部分成功し得るため、retry 対象を分離する。event append が成功した後は、その event を再 append せず `set_session_state` のみを retry する。

## データモデルまたは型

### SessionNotice（新規・usecase）

```rust
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNotice {
    pub session_id: String,
    pub kind: SessionNoticeKind,
    pub message: String,
    pub created_at: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionNoticeKind {
    /// 永続化がリトライ後も失敗し操作がエラー化した（ST-4 / RT-8）
    PersistFailure,
    /// 破損 event log を append 前に自己修復した（RT-4）
    EventLogRecovered,
}
```

- S5 でこの `SessionNoticeKind::PersistFailure` が `Notice(PersistFailure)` 語彙へ吸収される前提。本 ISSUE では暫定表現とする。

### PersistFailureKind（内部・可視化文言とログ dims の分岐用）

```rust
enum PersistFailureKind {
    ReopenRuntime,        // usecase.rs:3378 相当
    QueuedTurnInterrupt,  // usecase.rs:3534 相当
    FinalPartsRecorded,   // complete_turn 相当（RT-8）
}
```

### 変更する既存シグネチャ

- `event_store.rs::append_session_event_to_dir` は gateway 内部の `AppendOutcome { recovered: bool }` を返す。domain の `AgentSessionWriter` は従来の汎用戻り値を維持し、JSON event log の物理修復詳細を持たない。
- gateway から usecase へは専用の `SessionEventLogRecoverySignal` を介して修復事実を一度だけ伝え、`SessionStore` が `EventLogRecovered` notice を発火する。

## 処理フロー

### ST-4: reopen runtime 失敗時（usecase.rs:3378）

1. `open_runtime_for_session` が Err。
2. 現行の `let _ = set_session_state(Error)` を `persist_with_retry(|| set_session_state(..Error), ReopenRuntime, ..)` に置換。
3. 全リトライ失敗時: `report_persist_failure` で `PersistFailure` バナー＋構造化ログ。in-memory の後続（`emit_session_state_change`）は現行どおり実行するが、durable 更新に失敗した事実を可視化した状態で進む（無言乖離をなくす）。
   - RT-7 のキュー回収は非スコープ。ここでは「握りつぶしをなくす」のみ。

### ST-4: queued turn の start_turn 失敗時（usecase.rs:3534）

1. `runtime.start_turn` が Err。rollback 実行（現行どおり）。
2. `let _ = append_session_event_and_project_state(TurnInterrupted)` を `persist_with_retry` に置換。
3. 全失敗時: `report_persist_failure(QueuedTurnInterrupt)`。この関数自体はイベント発火型のため、失敗を呼び出し元へ伝搬すべき文脈（この関数は `-> ()` の spawn 経路）では、エラーを可視化＋ログで確実に露出させ、in-memory を無言で進めない。

### RT-4: append 側自己修復（event_store.rs）

1. `append_session_event_to_dir` で file を開き末尾非空白バイトを取得。
2. `]` でない（＝末尾破損: 欠け `]`・中途行）場合:
   a. file 全体を読み、`recover_unclosed_session_events(content)` で有効イベント列へ復元。
   b. 復元できたら、有効な JSON 配列（`[ ... ]`）としてファイルを書き直す（既存の pretty + indent 形式で再シリアライズ）。復元不能なら現行どおり Err。
   c. 修復後、通常の追記分岐（末尾 `]` を truncate → `,` 追記 → `]` 再クローズ）へ合流する。
3. gateway 内部で `AppendOutcome { recovered: true }` を受け、専用 recovery signal を立てる。上位 usecase は signal を消費して `EventLogRecovered` バナーを発火する。修復事実は必ずログへ残す。
4. 以降の append は通常成功する（読み取り側 `parse_session_events_content` も引き続き整合）。

`recover_unclosed_session_events` は既存の private fn。append 経路から使えるよう可視性を `pub(super)` へ広げるか、`FileSessionStorage` の実装ブロック内から参照可能な位置へ配置する。

### RT-8: FinalPartsRecorded 失敗時の本文保持（complete_turn）

現行（usecase.rs:3224-3260）の問題:

- `append_final_turn_events` が Err でも警告ログのみで継続。
- 続く projection（`load_session_events` → `agent_parts_for_message`）は、FinalPartsRecorded が未追記のため tool イベントのみから tool-only parts を導出する。
- `parts_to_persist` は「projection が非空なら projection、空なら live parts」。tool-only は非空になるため **live parts（Text/Thinking 含む）が捨てられ tool-only で上書き**される。

変更:

1. `append_final_turn_events` 内の `FinalPartsRecorded` append を `persist_with_retry(FinalPartsRecorded)` にする（短い backoff で有限回）。
2. `append_final_turn_events` の成否を complete_turn 側で分岐する:
   - **成功時**: 現行どおり projection 由来 parts を採用（FinalPartsRecorded が反映済みなので projection は Text/Thinking を含み、正しい）。
   - **失敗時（リトライ尽き）**: projection 由来 parts で上書きしない。`parts_to_persist` に **live `parts`（完全な本文）** を用いる。つまり「projection が tool-only でも live を優先」する分岐にする。加えて `report_persist_failure(FinalPartsRecorded)` を発火し、persist 済み本文を保持する（`persist_message_parts` は live parts で上書き＝本文維持）。
3. 結果として reload 後も本文（Text/Thinking）が残る。純テキスト turn は元々 live フォールバックで無傷（requirements A5）。

要点: **「FinalPartsRecorded が durable に反映されたときだけ projection を信頼する」**。未反映なら live parts が唯一の完全な真実源であり、それを保持する。

## エラー処理

- store / event_store の各メソッドは従来どおり `Result<_, String>` を返す。握りつぶし（`let _ =`）を production 永続化経路から全廃する（R1 / 受け入れ基準）。
- `persist_with_retry`: 各試行の Err はログ（`log::warn!`）に残し、最終失敗時のみ `report_persist_failure` を 1 回発火（バナー多重発火を避ける）。
- 後続の persist 成功時は backend の `PersistFailure` notice を clear し、更新済み `SessionStatus` を push / snapshot の双方へ反映する。`EventLogRecovered` 等の別 kind は成功時に誤って clear しない。
- `report_persist_failure`: `SessionNotice{ PersistFailure }` を status center へ格納し notifier で push、`log::error!` で構造化（session_id, kind, error, attempts）を残す。
- RT-4 修復: 復元不能な破損（配列開始 `[` すら無い等）は Err のまま。既存 `invalid_sessions` 経路と整合させ、無限修復ループを作らない。
- 可視化は transient。失敗が解消（次の成功 append 等）したときにバナーを消す/上書きする挙動は、status center の最新 notice 差し替えで表現する（恒久エラー表示は非スコープ）。

## テスト方針

配置は対象 module 隣接（Rust は `#[cfg(test)] mod tests`）。外部プロセス・実 I/O 障害は使わず、既存注入機構（`set_append_event_hook_for_test` 等）を利用（requirements A6）。

### RT-4（event_store.rs::tests、既存 test 群へ追加）

- `append_session_event_recovers_unclosed_log_then_appends`: 末尾 `]` 欠けの fixture を書き、append が成功し、読み戻すと元イベント＋新イベントが揃う。
- `append_session_event_recovers_trailing_partial_event`: 中途行（壊れた末尾オブジェクト）を含む fixture で、壊れた末尾は落として有効分＋新規が揃う。
- `append_session_event_returns_recovered_outcome`: gateway 内部では修復時 `AppendOutcome.recovered == true`、正常時 `false`。
- 復元不能 fixture では従来どおり Err。

### ST-4（runtime/usecase.rs::tests）

- `reopen_runtime_persist_failure_is_visible_not_silent`: `set_session_state` を注入で失敗させ、リトライ後に notice（PersistFailure）が発火し `let _ =` 無言化しないことを固定。
- `queued_turn_interrupt_append_retries_then_reports`: `append_event_hook` で TurnInterrupted append を失敗させ、リトライ→最終 report を固定。
- `transient_failure_recovers_within_retry`: 1 回だけ失敗させ、リトライ内成功で操作が成功・notice 非発火（behavior「一時失敗はリトライで回復」）。
- `persist_failure_then_success_clears_notice`: 継続失敗で表示された `PersistFailure` が次の成功 persist で backend snapshot と push の双方から消える。
- `post_append_projection_failure_does_not_duplicate_event`: event append 成功後の meta projection を失敗注入し、projection retry 後も同一 event が durable log に 1 件だけ存在する。

### RT-8（runtime/usecase.rs::tests）

- `final_parts_append_failure_keeps_body_not_tool_only`: tool part を含む turn で FinalPartsRecorded の append のみ失敗注入。persist される parts が live（Text/Thinking 含む）で、tool-only へ置換されないこと、reload 相当（再 projection）で本文が残ることを固定。
- 静的棚卸し確認: `rg '^\s*let _ = ' usecase/agent_session/` の production 永続化ヒットが 0 件（projector.rs:632・test 用は対象外として明記）。

### 可視化配信

- notice が status snapshot に含まれること・notifier push が呼ばれることを usecase テストで確認。protocol DTO の serialize 形状を presenter/protocol テストで確認。frontend は表示のみのため表示分岐の最小 component テスト。

CI（`cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` / `pnpm lint` / `pnpm test`）を通す。

## リスクと代替案

- **R-1: recovery signal の取りこぼし**。gateway 内部の修復 outcome を domain API に載せず専用 signal で伝えるため、signal は session_id 単位で保持し usecase が成功 append 直後に一度だけ消費する。これにより domain 境界を保ったまま R2 の可視化を満たす。
- **R-2: リトライ中の backoff がユーザー体感遅延を増やす**。`tokio::time::sleep` で非ブロックにし、N と間隔を小さく保つ（下記 A-D2）。
- **R-3: RT-8 で live parts を優先すると、正常時に projection の正規化（重複マージ等）を失う懸念**。→ 分岐は「append 失敗時のみ live 優先」。成功時は現行どおり projection を使うため正常系の挙動は不変。
- **R-4: バナーの transient 設計が durable-first 原則に反する**。requirements A2 が「唯一の例外」として明示的に許容。S5 で `Notice` へ移行する接合点を残すことで技術的負債を局所化。
- **R-5: 修復書き直し中のクラッシュで二重破損**。修復は truncate 前に有効イベント列を確定してから単一の書き直しを行い、既存の append と同じ file_lock 下で実行する。fsync までは要求しない（既存 append と同水準）。

## 仮定

- **A-D1（修復通知境界）**: `AppendOutcome` は gateway 内部に閉じ、domain の writer API は変更しない。修復事実は usecase port の `SessionEventLogRecoverySignal` で session_id 単位に伝える。いずれでも「修復はログに必ず残す」。
- **A-D2（リトライ値）**: `persist_with_retry` は **最大 3 回リトライ（初回含め計 4 試行）**、backoff は **50ms → 100ms → 200ms の指数**。試行間は `tokio::time::sleep`。これは requirements A4 の「短い backoff で有限回」を満たす具体値であり、ローカル file I/O の一時失敗（ロック競合・瞬間的 EBUSY 等）を吸収する範囲。
- **A-D3（可視化配信先）**: バナーは `AgentStatusCenter` が session-scoped に所有し、`AgentSessionEventNotifier` の新メソッドで push＋status snapshot に同梱する。frontend は表示のみ。
- **A-D4（発火の集約）**: ST-4 / RT-4 / RT-8 の可視化は共通ヘルパ（`report_persist_failure` / notice 発火）に集約し、S5（#1393）ではヘルパ内部のみ差し替える。呼び出し側は不変。
- requirements の A1〜A6 を前提として引き継ぐ。

## Open Questions

なし
