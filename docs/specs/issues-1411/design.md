# Design

関連: #1411（milestone 84「Agentチャット安定化」／ Phase 0 ／ L10）

正本参照:
- 要求: `docs/specs/issues-1411/requirements.md`
- 振る舞い: `docs/specs/issues-1411/behavior.md`
- 問題インベントリ: `specs/milestone-84-agent-chat-stabilization/agent-chat-instability-audit.md` の **ST-5**
- ライフサイクル理想形: `specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-lifecycle.md` の **I13**

## 概要

session runtime lock 機構（`SessionRuntimeLocks` / `acquire_session_runtime_lock` / `acquire_session_lock` / `SessionRuntimeLockGuard`）の構造的安定化を行う。backend（Rust）内部完結であり、外部から観測可能な UI/CLI の振る舞いは追加・変更しない。

本変更は次の 4 点で構成する。

1. **lock 保持規約の rustdoc 明文化**（R1）。
2. **既存 lock 保持経路の棚卸しと是正／列挙**（R2）。
3. **prune のランタイム非依存化**（R3・R4）: `Drop` の `Handle::try_current()` 依存を廃止し、解放時に prune 候補を同期的な pending 集合へ登録、次回 `acquire_session_runtime_lock` で未参照エントリを掃除する。
4. **テストビルド限定の lock 再入検出**（R5）: `#[cfg(test)]` 限定の task owner keyed 共有 registry + `assert!`。

既存の per-session 排他モデルは維持する。runtime 状態機械そのものの分解（ST-3 / #1412）は非スコープ。

## 変更対象

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs`
  - 型定義 `SessionRuntimeLock` / `SessionRuntimeLocks`（`:76-77`）
  - `acquire_session_runtime_lock`（`:2153`）
  - `SessionRuntimeLockGuard` と `Drop` 実装（`:2147-2196`）
  - `acquire_session_lock` の rustdoc（`:1091`）
  - `RuntimeContext.session_locks` フィールド（`:201`）と生成箇所
  - lock 保持経路の呼び出し箇所（棚卸し対象: `:279` / `:284` / `:438` / `:484` / `:1826` / `:1880` / `:2087` / `:2259`）
- `src-tauri/src/adaptor/gateway/workflow/runtime_session.rs`
  - `start_fanout_child_sessions` の単一 task による複数 session guard 同時保持を、child ごとの reservation task に分離する
  - 全 child の activation を予約してから snapshot / tab を公開し、外部操作に対する既存の起動順序を維持する
  - reservation task は取得した guard を親 activation future へ順に引き渡し、`start_turn_locked` は親 future が実行する。これにより cancel decision 待ちでは child start も親とともに静止し、rollback 時は同じ future を再開できる
  - reservation task の abort handle と完了通知を追跡し、commit 時に全 task の終了を確認してから terminal cleanup へ進む
- `src-tauri/src/adaptor/gateway/workflow/runtime_engine_impl.rs`
  - contract repair turn の開始結果取得直後に session guard を解放し、開始失敗時の永続化・workflow state broadcast を lock 外へ移す
  - fan-out activation の cancel cleanup を activation lock の解放前に完了する

frontend への変更は無い（R7、rust-first-logic）。

## アーキテクチャと責務分割

lock 機構は usecase 層の内部詳細であり、レイヤー境界は変えない。責務は次のとおり分割する。

- **`SessionRuntimeLocks`（データ構造）**: per-session の `Arc<Mutex<()>>` を保持する map の source of truth。加えて、`Drop` から同期的に登録される prune 候補（pending 集合）を保持する。
- **`acquire_session_runtime_lock`（取得）**: (1) テストビルドでは owner を RAII 予約、(2) pending 集合を掃除（未参照エントリ除去）、(3) 対象 session のエントリを取得または生成、(4) per-session lock を取得、(5) `SessionRuntimeLockGuard` へ owner 予約を移して返す。
- **`SessionRuntimeLockGuard`（解放）**: `Drop` で per-session guard を落とし、自 session_id を pending 集合へ**同期的に**登録するだけ。tokio runtime handle には依存しない。実際の map からの除去は次回 `acquire` に委譲する。テストビルドでは内包する RAII owner 予約の `Drop` が共有 registry から owner を除去する。
- **再入検出（テストビルド限定）**: process 共有の「owner → 予約中または保持中の session」registry。owner は Tokio task 内では `tokio::task::Id`、task 外では thread ID とする。per-session lock の await 前に RAII 予約を登録し、同じ owner の要素があれば規約違反として `assert!` で失敗させる。future の cancel / panic 時は RAII の `Drop` で予約を除去し、取得成功時は予約を guard へ移す。task が await 後に別 worker thread へ移動しても task ID は不変であり、Drop も取得 worker に依存せず同じ要素を除去できる。fan-out reservation task から親 future へ guard を引き渡す場合は registry の owner も受取側へ移し、受取側の再入を引き続き検出する。別 task の正当な待機は再入に数えない。production ビルドには一切コンパイルされない。
- **fan-out activation task 所有**: 各 child reservation task は自身の session lock だけを取得し、開始指示を受けると guard を親 activation future へ返して終了する。親 future が guard と `start_turn_locked` future を所有するため、cancel acknowledgment から decision まで parent を poll しなければ activation tree 全体が静止し、rollback は同じ future を継続できる。共有 tracker は各 reservation task の `AbortHandle` と Drop ベースの完了通知を保持し、cancel commit 時は activation future を drop して全 handle を abort した後、全完了通知を待って activation lock を解放する。abort terminal cleanup はこの activation lock 取得後に行う。

## データモデルまたは型

現状:

```rust
type SessionRuntimeLock = Arc<Mutex<()>>;                              // tokio::sync::Mutex
type SessionRuntimeLocks = Arc<Mutex<HashMap<String, SessionRuntimeLock>>>;
```

変更後（pending 集合を同居させる構造体へ）:

```rust
type SessionRuntimeLock = Arc<Mutex<()>>; // tokio::sync::Mutex（従来どおり）

struct SessionRuntimeLockRegistry {
    /// 各 session の排他 lock 本体。取得・生成・除去は async 文脈で行う。
    map: Mutex<HashMap<String, SessionRuntimeLock>>, // tokio::sync::Mutex
    /// Drop 時に同期登録される prune 候補。次回 acquire で掃除する。
    /// std::sync::Mutex なので Drop（非 async / runtime 無し）から触れる。
    pending_prune: std::sync::Mutex<HashSet<String>>,
}

type SessionRuntimeLocks = Arc<SessionRuntimeLockRegistry>;
```

`SessionRuntimeLockGuard` は production では現状の 3 フィールド（`session_id` / `guard: Option<OwnedMutexGuard<()>>` / `locks: SessionRuntimeLocks`）を維持する。`locks` の型は上記 `SessionRuntimeLocks`（`Arc<SessionRuntimeLockRegistry>`）に置き換わる。テストビルドだけ、取得前に registry entry を所有する `TestSessionRuntimeLockOwnerReservation` を持つ。

テストビルド限定の再入検出用共有 registry:

```rust
#[cfg(test)]
enum TestSessionRuntimeLockOwner {
    Task(tokio::task::Id),
    Thread(std::thread::ThreadId),
}

static HELD_SESSION_LOCKS: OnceLock<
    std::sync::Mutex<HashMap<TestSessionRuntimeLockOwner, String>>
>;
```

### pending 集合方式を採る理由（仮定 A2 の具体化）

`Drop` は非 async かつ tokio runtime handle を前提にできない。`map`（tokio `Mutex`）を `Drop` 内で確実に掃除するには async lock か `try_lock` が要り、いずれも「runtime 有無」や「競合有無」に依存して skip し得る。そこで **除去処理そのものを次回 `acquire`（必ず async 文脈）へ委譲**し、`Drop` は `std::sync::Mutex<HashSet<String>>` への登録だけを同期実行する。これにより「runtime の有無に関わらず、解放済み未参照エントリが次回 acquire までに必ず掃除される」（R4／behavior「prune はランタイムハンドルの有無に依存しない」）を満たす。

## 処理フロー

### acquire（`acquire_session_runtime_lock`）

1. **（テストビルドのみ）owner 予約と再入検出**: `tokio::task::try_id()` で取得フローを識別し、per-session lock の await 前に RAII 予約を共有 `HELD_SESSION_LOCKS` へ登録する。同じ owner が既に登録済みなら `assert!` で失敗させ、同一 owner の acquire future が並行 poll される場合も test profile や worker 移動に依存せず検出する。別 task は許可し、cancel / panic 時は RAII の `Drop` で予約を除去する。
2. **pending 掃除**: `map` を async lock。`pending_prune`（std mutex）を lock して drain したうえで、各候補 id について `map.get(id)` が `Arc::strong_count == 1` なら `map.remove(id)`。保持中（strong_count > 1）や、掃除までに再取得された id は残す。
   - 掃除は「対象 session に限らず、pending に溜まった全 id」を対象にする（多数 session 繰り返しでも収束させるため）。
3. **エントリ取得**: `map.entry(session_id).or_insert_with(|| Arc::new(Mutex::new(()))).clone()`。
4. `map` の async guard を drop（`clone` 済みの `Arc` のみ保持）。
5. `lock.lock_owned().await` で per-session 排他を取得。
6. **（テストビルドのみ）** 取得前の RAII owner 予約を `SessionRuntimeLockGuard` へ移す。
7. `SessionRuntimeLockGuard` を返す。

pending 掃除を acquire 冒頭（per-session lock 取得前）に置くことで、`map` の async lock 保持は最小範囲（掃除 + entry 取得）に留まり、per-session lock の待機（`lock_owned().await`）中は map lock を握らない。

### 解放（`SessionRuntimeLockGuard::Drop`）

1. `self.guard.take()` で `OwnedMutexGuard<()>` を drop（per-session 排他を解放し、この guard が握っていた `Arc` 参照を落とす）。
2. `self.locks.pending_prune`（std mutex）を lock し、`self.session_id` を insert。`Handle::try_current()` / `spawn` は使わない。
3. **（テストビルドのみ）** `HELD_SESSION_LOCKS` から取得時に保存した `(owner, session_id)` と一致する要素を除去する。

手順 1 を手順 2 より先に行うことで、次回 acquire 時に他の保持者がいなければ `strong_count == 1`（map の 1 本のみ）となり除去対象になる。

### 既存経路の棚卸し（R2）

`acquire_session_lock` / `acquire_session_runtime_lock` を呼ぶ全経路（`:279` `:284` `:438` `:484` `:1826` `:1880` `:2087` `:2259`）を規約 (a)(b)(c) に照らして確認する。実装時に確定するが、コードリーディング時点の予備観察は次のとおり。

- **(a) 別 session lock を保持中に取得**: `usecase.rs` 内の各経路は単一 session に対する guard を取り、その保持スコープ内で別 session の `acquire_*` を呼ばない（`:2259` の再取得は post-actions 実行後、先行 guard の外）。公開 `acquire_session_lock` の呼び出し元まで広げた棚卸しでは、`start_fanout_child_sessions` が単一 task で全 child session guard を取得していた。各 child 専用の reservation task が自身の guard だけを取得し、予約完了を親 task へ通知して start 指示を待ち、指示後は guard を親 activation future へ引き渡す構造へ是正する。親 future は全予約完了後に snapshot / tab を公開し、child を従来順に start するため、先頭 child の start 待機中も後続 child の外部操作は workflow activation を追い越さない。
- **(b) backend I/O await の最小化**: lock 保持中に `ensure_runtime`（process spawn）や session store I/O、runtime への stdin write を await する経路が存在する（例: `start_session:438`, `send_message:279/284`）。これらは per-session 直列化のために保持が必要な範囲であり、別 session を巻き込まないため deadlock 要因ではない。過剰な await 範囲があれば縮小するが、**排他の正しさを崩す縮小はしない**。過大な是正が必要な箇所は列挙し分割判断する。
- **(c) emit はロック外**: `emit_session_state_change` 等の通知が guard 保持中に呼ばれる箇所を確認する。lock 外へ移せるもの（通知が排他状態に依存しないもの）は移す。移動が状態順序を壊す恐れがある箇所は、違反一覧として列挙し分割判断する。

棚卸し結果（是正 or「違反なし」or 列挙 + 分割判断）は本 ISSUE / requirements の棚卸し欄に追記する。

## エラー処理

- `pending_prune`（std `Mutex`）の lock は poisoning し得る。`lock().unwrap_or_else(|e| e.into_inner())` で poison を無視して継続する（保持データは単なる `HashSet<String>` で、途中 panic による不整合が排他の正しさに波及しない）。
- prune の除去判定は `Arc::strong_count == 1` を唯一の基準にする。判定に失敗しても（該当 id が既に無い等）単に skip し、エラーにしない。prune は best-effort だが、pending に積まれた id は除去されるまで残り続けるため「無期限蓄積しない」保証は維持される。
- 再入検出の `assert!` は `#[cfg(test)]` ブロック内にあり production ではコンパイルされない。検出発火は debug/release の両 test profile でテスト失敗として表面化し、runtime のエラー型（`AgentRuntimeError`）には載せない。

## テスト方針

`usecase.rs` 内 `#[cfg(test)] mod tests` と workflow runtime の既存テスト module に Rust テストを追加する（配置規約に従う）。再入・Drop は multi-thread runtime でも検証する。

- **prune のランタイム非依存化（R3・behavior「prune はランタイムハンドルの有無に依存しない」）**
  - guard を drop 後、次回 `acquire` で未参照エントリが map から除去されることを確認。
  - guard 保持中は別 session の acquire を挟んでも保持中 session のエントリが残ることを確認。
  - 多数 session の取得・解放を繰り返し、後続 acquire ごとに map サイズが保持中 lock 数相当に収束することを確認（無限蓄積しない）。
- **prune の同期性**: `Handle::try_current()` に依存しない（Drop が spawn を要求しない）ことを、drop 直後に別スレッド/同一 runtime で acquire して除去される形で確認。
- **再入検出（R5・behavior 該当 Rule）**
  - session A の guard 保持中に session B を acquire → `assert!` 発火（`#[should_panic]`、debug/release test profile 共通）。
  - multi-thread runtime の task でも同じ再入が検出されることを確認。
  - guard を取得 worker とは別 thread で Drop しても owner が除去され、その後の逐次 acquire が成功することを確認。
  - guard を別 task へ引き渡して owner を移した後、受取側での再入が検出されることを確認。
  - 未保持状態からの acquire は検出されない・正常完了。
  - A を取得・解放後に B を逐次取得 → 検出されない・正常完了。
- **fan-out cancel lifecycle**: 先頭 child の backend start を停止して explicit stop し、stop が backend 待機の解除なしに完了すること、全 child lock が回収されること、待機解除後も後続 start が発生しないことを確認する。abort は cancel acknowledgment 後かつ durable decision 前に backend 待機を解除しても child start が進まず、commit 後の terminal cleanup が全 reservation task の終了確認後に行われ、live runtime が残らないことを確認する。
- **排他の維持（behavior「同一 session は直列化 / 異なる session は独立」）**
  - 既存の排他テストがあれば維持。無ければ、同一 session の二重 acquire が直列化され、異なる session は独立取得できることを確認。
- **production 非影響（R5・behavior「production ビルドの挙動・性能は変わらない」）**
  - 再入検出コードが `#[cfg(test)]` で隔離され、production ビルドに含まれないことをコード構造で担保（レビュー観点）。ランタイム挙動・排他・prune はビルド構成に依らず同一。

CI 同等の `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`（`src-tauri/`）で AC4 を満たす。

## リスクと代替案

- **R-1: テスト専用共有 registry の残留**。panic や異なる worker での Drop でも owner を確実に除去する必要がある。
  - 緩和: owner を guard 自体に保存し、Drop は current worker を再計算せず保存済み owner を共有 registry から除去する。multi-thread runtime と別 thread Drop のテストで検証する。production/dev には `#[cfg(test)]` によりコンパイルされない。
- **R-2: pending 集合の遅延掃除**。除去は次回 acquire 時のため、acquire が全く起きない期間は解放済みエントリが pending に残る。これは「無期限蓄積」ではなく「次回 acquire まで」の有界遅延であり、要求（R4）を満たす。定期掃除タスクは非スコープ。
  - 代替案: `Drop` 内で tokio `Mutex::try_lock()` を試し成功時は即時除去、失敗時のみ pending 登録するハイブリッド。即時性は上がるが「runtime 有無に非依存」を満たすうえで必須ではなく、経路が二重化して複雑になるため採らない。次回 acquire 委譲に一本化する。
- **R-3: `SessionRuntimeLocks` の型変更（`Arc<Mutex<HashMap>>` → `Arc<struct>`）**が `RuntimeContext` 生成箇所や `Arc::clone` 箇所へ波及する。影響は同一ファイル内に閉じる見込み。コンパイラで網羅的に洗い出せる。
- **R-4: 棚卸しで大きな違反が見つかった場合**のスコープ膨張。requirements の分割判断に従い、局所的な (a) 違反は本変更で是正し、状態機械や post-action 境界の変更が必要な (b)(c) は違反一覧（ファイル・関数・違反種別）を列挙して分割 ISSUE 化する。

## 仮定

- 仮定 D1: pending 掃除は `std::sync::Mutex<HashSet<String>>` を `SessionRuntimeLockRegistry` に同居させ、`Drop` は登録のみ・`acquire` が掃除を担う（requirements 仮定 A2 の「次回 acquire で掃除」案を採用）。
- 仮定 D2: 再入検出は `#[cfg(test)]` 限定の task owner keyed 共有 registry と `assert!` で行い、検出粒度は「同一 task が lock を 1 つでも保持したまま acquire する」= 任意 session 保持中の同一実行フロー再入（同一/別 session を包含）とする。これは behavior 仮定 3 の最低要件「別 session の lock を保持したまま acquire」を包含し、別 taskの正当な待機を妨げず、test profile や worker 移動に依存しない。
- 仮定 D3: 再入検出と guard Drop は current-thread と multi-thread の双方で検証する。
- 仮定 D4: 実装時の棚卸しで `start_fanout_child_sessions` に単一 task による明確な (a) 別 session lock 同時保持を確認したため、child ごとの reservation task に分離する。全 activation を公開前に予約し、guard を親 activation future へ順に引き渡して child の start 順序と cancel/rollback の所有境界を維持する。(b)(c) の分割対象は requirements の棚卸し結果へ列挙する（requirements 仮定 A4）。
- 仮定 D5: 外部（UI/CLI）から観測可能な振る舞い変更は無い（requirements 仮定 A5・behavior 仮定 5）。

## Open Questions

なし
