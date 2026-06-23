# Design

Terminal / PTY ライフサイクルに cap・eviction・idle timeout を導入し、メモリ常駐量とピークを境界づけるための実装設計。requirements.md / behavior.md の R1〜R8・AC1〜AC6 を満たす。

cap 値・idle 時間などの具体値は requirements.md の A2/A5/A6/A7 に従う暫定値とし、本設計では Rust 側のチューニング可能なパラメータ（`PtyLifecycleConfig`）として扱う。

---

## 概要

現状の課題（requirements.md 背景）に対し、以下の方針で対応する。

- **境界づけの正本を Rust 側に置く**（`.claude/rules/rust-first-logic.md` 準拠）。cap / LRU eviction / idle timeout の判定・enforcement はすべて `domain` / `usecase` 層で行い、frontend は Rust が公開する状態・イベントを表示・反映するだけにする。
- **frontend は inactive xterm を DOM から unmount する**。PTY 本体は Rust 側に session_key で生存し続けるため、再アクティブ化時は既存の `get_or_spawn_pty` 経路で `buffered_output`（直近スクロールバック）を取得して xterm へ書き戻して復元する。
- **PTY output buffer の正本を Rust runtime 側に一本化**する（A3）。`ws_bridge.rs` の独立バッファ（`pty_output_buffers`）を廃止し、remote subscriber には接続時に runtime buffer からスクロールバックを供給する。

新しいユーザー向け機能は追加しない。既存の PTY 生成・復帰・GC 経路（`get_or_spawn_pty` / `gc_ptys_for_worktree` / `kill_pty`）に lifecycle policy を組み込む。

---

## 変更対象

### Backend (src-tauri/src)

| パス | 変更概要 |
| --- | --- |
| `domain/pty_session/entities/pty_session_registry.rs` | activity 追跡・pinned 集合・active pin token・cap 判定・LRU/idle eviction 対象選定を追加 |
| `domain/pty_session/value_objects/`（新規 `pty_lifecycle_config.rs`） | `PtyLifecycleConfig`（cap・idle timeout・buffer cap）を定義 |
| `usecase/pty_session/spawn_usecase.rs` | spawn 前の cap enforcement（LRU eviction or 上限制御）を追加 |
| `usecase/pty_session/lifecycle_usecase.rs` | `sweep_idle`（idle timeout eviction）・`record_activity`・active terminal 登録/解除を追加 |
| `adaptor/gateway/pty_session/backend_impl.rs` | output 読み取り時の activity 記録、idle sweeper タスク起動、eviction 時のイベント emit、`buffered_output` を remote へ供給する経路 |
| `adaptor/controller/command/pty_session/commands.rs` | `register_active_terminal` / `unregister_active_terminal` コマンド追加。`get_or_spawn_pty` に active/pinned 情報を渡す |
| `ws_bridge.rs` | `pty_output_buffers` / `PTY_OUTPUT_BUFFER_SIZE` / `remove_pty_output_buffer` を削除。subscriber 不在時 broadcast を抑止 |
| `ws_server/session.rs` | subscriber 接続時に alive PTY の `buffered_output` を runtime から取得して replay する |
| `adaptor/protocol/pty.rs`・`protocol/mod.rs` | eviction 通知用 `PtyEvictedMsg` / `pty-evicted` を追加 |

### Frontend (src)

| パス | 変更概要 |
| --- | --- |
| `components/panels/TerminalTabPanel.tsx` | `forceMount` + `data-[state=inactive]:hidden` をやめ、active tab のみ条件マウントにする |
| `components/panels/PaneLeafContainer.tsx` | active pane の xterm のみマウント、inactive pane は軽量プレースホルダ |
| `hooks/useTerminal.ts` | remount 時に `get_or_spawn_pty` → `buffered_output` を write して復元。active pin token の登録/解除と `pty-evicted` リスナ追加 |
| `hooks/useTerminalPanes.ts` | `pty-evicted` を受けて該当 pane を `tabStateCache` から除去。active tab/pane に応じた mount 状態を `useTerminal` へ反映 |

---

## アーキテクチャと責務分割

レイヤ責務は既存のクリーンアーキテクチャ構成を踏襲する。

```
frontend (表示/入力/invoke)
   │  invoke: get_or_spawn_pty / write_pty / resize_pty / register_active_terminal / unregister_active_terminal / kill_pty / gc_ptys_for_worktree
   │  listen: pty-output / pty-exit / pty-evicted
   ▼
adaptor/controller/command  …… Tauri コマンド境界。DTO 変換のみ
   ▼
usecase/pty_session         …… spawn 時 cap enforcement・idle sweep・activity 記録のオーケストレーション
   ▼
domain/pty_session          …… registry の cap/LRU/idle 判定（純粋ロジック・clock 注入で決定論的）
   ▲
adaptor/gateway (backend_impl) …… 実 PTY 操作・output 読み取り・event emit・sweeper タスク・clock 供給
```

### 責務の置き場所（重要判断）

- **「どの PTY を evict するか」の判定は domain（registry）に置く。** cap 超過判定・LRU 対象選定・idle 超過判定は `PtySessionRegistry` の純粋メソッドにし、時刻は引数（`now_ms`）で受け取る。これにより requirements の「ポリシーは Rust 側で enforce」を満たしつつ、PTY 実体なしで単体テスト可能にする。
- **「いつ判定を回すか」「実際に kill する」は usecase/gateway に置く。** idle sweeper の周期実行とイベント emit は副作用なので gateway 側。
- **frontend の `tabStateCache` はレイアウト UI 状態（tab/pane 構成・名前・sessionKey）のみを保持する。** これは表示状態であり rust-first-logic の例外には当たらない。pane の生存可否（cap/idle eviction）は Rust が決定し、frontend は `pty-evicted` を受けてキャッシュから当該 pane を除去するだけ（Rust が enforcement の正本、frontend は mirror）。

### frontend remount と「lightweight state」（R1/R2）

- inactive な tab/pane は xterm を **unmount** する（DOM から除去）。PTY は Rust に session_key で生存。
- 再アクティブ化時、`useTerminal` は保持していた `sessionKey` で `get_or_spawn_pty` を呼ぶ。既存 PTY がヒットすれば `is_new=false` で `buffered_output`（runtime の 64KB ring 由来）が返るので、それを `terminal.write` して直近スクロールバックを復元する。
- **カーソル状態**は output buffer 内の生バイト（ANSI 制御列含む）の replay により再現される（buffered_output を write すれば xterm が解釈してカーソル位置も再現）。
- **サイズ**は remount 後の fit / `resize_pty` で active 領域に追随させる（R8 後段）。
- 復元範囲は output buffer cap（既定 64KB ring）相当の直近分まで（A7）。フルスクロールバックは保持しない。

---

## データモデルまたは型

### PtyLifecycleConfig（新規 value object）

```rust
pub struct PtyLifecycleConfig {
    /// worktree ごとの alive PTY 上限。初期値 32 (= MAX_PANES_PER_TAB 4 × MAX_TABS 8, A2/A5)。
    pub per_worktree_cap: usize,
    /// 全 worktree 合計の alive PTY 上限。初期値 64（暫定, A2 に明示なし→仮定）。
    pub max_panes_total: usize,
    /// alive PTY の idle timeout。初期値 300 秒 (= 既存 delayed cleanup の 5 分, A2)。
    pub idle_timeout: Duration,
    /// output buffer cap（runtime ring）。初期値 64KB (= OUTPUT_BUFFER_CAPACITY, A2)。
    pub output_buffer_cap: usize,
    /// idle sweeper の実行周期。初期値 60 秒（暫定）。
    pub sweep_interval: Duration,
}
```

`output_buffer_cap` は既存 `OUTPUT_BUFFER_CAPACITY`（`backend_impl` services.rs）と統合し、定数の重複を解消する。

### PtySessionRegistry（拡張）

既存:

```rust
pub struct PtySessionRegistry {
    sessions: HashMap<u64, PtySession>,
    next_pty_id: u64,
}
```

追加するフィールドとメソッド（`PtySession` 自体は `PartialEq/Eq` を維持するため activity は registry 側で持つ）:

```rust
    // pty_id -> 最終アクティビティ（論理 ms。clock は呼び出し側が注入）
    activity: HashMap<u64, u64>,
    // eviction 対象から除外する session_key 集合
    pinned: HashSet<String>,
    // mounted useTerminal instance ごとの active pin token
    active_pin_tokens: HashMap<String, ActivePin>,
    // unregister が先着した token の stale register を拒否する tombstone
    retired_active_tokens: HashSet<String>,
    config: PtyLifecycleConfig,
```

| メソッド | 用途 |
| --- | --- |
| `record_activity(pty_id, now_ms)` | 入出力/resize 時刻の更新 |
| `register_active_terminal(worktree, session_key, token)` / `unregister_active_terminal(worktree, session_key, token)` | useTerminal instance 単位で active/可視端末を pin/unpin（R3/R8 の active 除外）。古い unregister は別 token の新しい mount を解除できない |
| `count_for_worktree(worktree) -> usize` / `count_total() -> usize` | cap 判定 |
| `select_evictable_for_worktree(worktree, now_ms) -> Option<u64>` | cap 超過時の LRU eviction 対象（pinned 除外・idle 優先・最も古い activity を選ぶ） |
| `select_idle_timed_out(now_ms) -> Vec<u64>` | idle timeout 超過 PTY（pinned 除外） |
| `would_exceed_worktree_cap(worktree)` / `would_exceed_total_cap()` | 新規許可可否 |

「idle」の定義: `now_ms - activity[pty_id] >= idle_timeout`。入力（`write_pty`）・出力（reader）・`resize_pty` のいずれかで activity を更新する。pinned（可視 active）な PTY は idle 判定・LRU eviction の双方から除外する。

### eviction 通知メッセージ（新規）

```rust
pub struct PtyEvictedMsg {
    pub pty_id: u64,
    pub session_key: String,
    pub reason: PtyEvictReason, // Idle | CapExceeded
}
```

`pty-exit`（プロセス終了 = pane を残し `[Process exited]` 表示）と、`pty-evicted`（lifecycle による解放 = pane 自体を除去）を区別する。frontend は両者で異なる UI 反応をするため、別イベントにする。

---

## 処理フロー

### F1. spawn 時の cap enforcement（R3/R4, AC4）

`usecase::spawn_usecase::get_or_spawn`:

1. `session_key` 指定があり既存 PTY がヒット → 従来通り `buffered_output` を返す（`is_new=false`）。activity を更新。
2. 新規 spawn 要求のとき、registry の `reserve_spawn_slot` で cap 判定・eviction target・spawn slot を単一境界で予約する。対象なし（全て pinned/非 idle）なら上限制御（spawn 拒否し `UsecaseError::CapReached` を返す）。
3. runtime 作成・registry 挿入・reader 起動がすべて成功した後に、予約済み eviction target を kill（→ `pty-evicted` emit）して spawn slot を commit する。`spawn_backend` / reader 起動 / eviction revalidate が失敗した場合は予約を rollback し、作成済み新規 PTY は kill/remove して half-created session を残さない。
4. spawn 後、新 PTY の activity を記録し、pinned 集合に追加（直後は可視のため）。

frontend の `MAX_TABS` / `MAX_PANES_PER_TAB` は UI 上のガードとして残す（A5）が、最終的な cap enforcement は上記 Rust 経路が正本。

### F2. idle sweep（R5, AC4）

`backend_impl` 起動時に tokio タスクを 1 本起動:

```
loop {
    sleep(config.sweep_interval)
    let now_ms = clock.now_ms()
    let targets = usecase::lifecycle_usecase::sweep_idle(gateway, now_ms) // = registry.select_idle_timed_out
    for pty_id in targets { kill + emit pty-evicted(reason=Idle) }
}
```

pinned（可視 active）は除外されるため、表示中の端末は idle でも解放されない（R8）。

### F3. activity 記録

- 入力: `write_pty` コマンド処理時に `record_activity`。
- 出力: `spawn_output_reader` の読み取りループで chunk 受信時に `record_activity`。高頻度出力でのロック競合を避けるため、activity 更新は **最短 1 秒に 1 回へスロットル**する（前回更新からの差分が閾値未満ならスキップ）。
- resize: `resize_pty` 時に `record_activity`。

### F4. active/pinned の同期（R3/R8）

frontend は `useTerminal` mount ごとに一意な active token を作り、PTY 確定時に `register_active_terminal(worktree_path, session_key, active_token)`、unmount / eviction / pending spawn 後の cleanup 時に `unregister_active_terminal(...)` を送る。Rust は token 単位で active pin を保持し、同じ session_key でも古い token の unregister が新しい mount の token を解除できないようにする。unregister が register より先着した token は retired として記録し、後着した stale register を拒否する。

### F5. PTY output buffer 一本化（R6, AC3）

- 正本: `PtyRuntime.output_buffer`（`VecDeque<u8>`, cap = `output_buffer_cap`）。
- `ws_bridge.rs` の `pty_output_buffers` / `PTY_OUTPUT_BUFFER_SIZE` / `remove_pty_output_buffer` を削除。`WsBroadcaster::try_send` はバッファリングせず送信のみ行う。
- スクロールバック取得は常に `gateway.buffered_output(pty_id)`（runtime ring）から行う。inactive 復元（F の get_or_spawn 経路）も remote 供給（F6）も同一ソース。

### F6. remote subscriber 供給と最小化（R7, AC5）

- **broadcast 抑止**: `spawn_output_reader` は出力を `app.emit("pty-output", ...)`（desktop 用、常時）し、WS 側は **subscriber が存在するときのみ** `ws.try_send` を呼ぶ。`WsBroadcaster` に `has_subscriber()`（`sender.is_some()`）を設け、不在時はバイト列の整形・送信自体をスキップする。runtime ring は subscriber 有無に関わらず維持されるため、buffer 蓄積は subscriber 有無で増えない（むしろ独立バッファ廃止でメモリ減）。
- **接続時 replay**: `ws_server::session::handle_ws_authenticated` で subscriber 確立後、alive な terminal PTY を registry から列挙し、各 `gateway.buffered_output(pty_id)` を `PtyOutput` として初期 replay 送信する。これで subscriber は必要なスクロールバックを正本から受け取る。

### F7. frontend remount / 復元（R1/R2, AC1/AC2）

- `TerminalTabPanel`: active tab の `TabsContent` のみ子（端末）をマウント。inactive tab は子を描画しない。
- `PaneLeafContainer`: active pane のみ `TerminalPanel` をマウント。inactive pane はプレースホルダ（クリックでアクティブ化）。
- アクティブ化 → `useTerminal` が `get_or_spawn_pty(sessionKey=...)` → `buffered_output` を `terminal.write` → 直近スクロールバック・カーソルを復元 → fit/`resize_pty` でサイズ追随。

---

## エラー処理

- **cap 到達かつ evict 対象なし**: `get_or_spawn` は `UsecaseError::CapReached` を返す。コマンド層は `Result<_, String>` でメッセージ化し、frontend は「上限に達したため新規端末を開けない」旨を表示（新規 pane を作らずロールバック）。
- **eviction 対象が既に exited/不在**: kill はべき等に扱い、不在なら no-op（既存 `kill` 同様）。
- **`pty-evicted` 受信時に該当 pane が既にない**: frontend はキャッシュ除去を no-op とする。
- **buffered_output の UTF-8 境界**: 既存 `process_pty_output` の pending buffer（`MAX_PENDING_BYTES`）で部分 UTF-8 を保持する仕組みを維持し、ring 取り出し時は `from_utf8_lossy` で安全化（既存踏襲）。
- **ロック poisoning**: 既存パターン `lock().unwrap_or_else(|e| e.into_inner())` を踏襲。
- **idle 解放と再アクセスの競合**: evict 後に同 session_key で `get_or_spawn` が来た場合は `is_new=true`（buffered_output 空）で新規生成され、frontend は新しいプロンプトを表示する（仕様上許容、A6）。

---

## テスト方針

### Rust（domain / usecase）

- `PtySessionRegistry`（純粋・clock 注入で決定論的）:
  - per-worktree cap / total cap の境界判定（cap-1, cap, cap+1）。
  - `select_evictable_for_worktree`: pinned 除外・idle 優先・最古 activity 選択・全 pinned 時は `None`。
  - `select_idle_timed_out`: idle timeout 直前/直後・pinned 除外。
  - active terminal（pinned）が eviction されないこと（R3/R8）。
- `spawn_usecase::get_or_spawn`（mock gateway）:
  - cap 未達 → 通常 spawn。
  - cap 到達かつ idle 有り → 1 件 evict して spawn 許可。
  - cap 到達かつ evict 対象なし → `CapReached`。
  - 既存 session_key ヒット → buffered_output 返却・activity 更新。
- `lifecycle_usecase::sweep_idle`: idle 超過のみ対象、pinned 非対象。
- output buffer 一本化: `buffered_output` が runtime ring から取得されること。ws_bridge に独立 buffer が存在しないこと（型・フィールド削除の回帰）。

### Rust（ws / gateway）

- subscriber 接続時に alive PTY の buffered_output が replay されること（R7）。
- `has_subscriber()` false 時に `try_send` が整形・送信をスキップすること（R7）。

### Frontend（Vitest, Tauri invoke は `vi.mock`）

- `useTerminal`: remount 時に `get_or_spawn_pty` の `buffered_output` を `terminal.write` で復元すること（R2）。`pty-evicted` リスナの登録。
- `useTerminalPanes`: `pty-evicted` 受信で該当 pane が `tabStateCache` から除去されること。active pane/tab の terminal mount に応じて `useTerminal` が active token を登録/解除すること。
- `TerminalTabPanel` / `PaneLeafContainer`: inactive tab/pane の xterm が DOM にマウントされないこと（R1, AC1）。active のみマウント。
- `react-resizable-panels` / `@tauri-apps/api` / Monaco は既存方針通り mock。

### 非テスト（CLAUDE.md 方針）

- 実 `git push` / 外部プロセス起動・実 PTY spawn の E2E は単体テスト対象外。idle sweeper の実時間待機はテストせず、判定ロジック（registry/usecase）を時刻注入でテストする。

---

## リスクと代替案

- **R-1: inactive xterm の unmount による再レイアウト/ちらつき。** 復元は 64KB replay で高速だが、巨大出力直後は描画コストが出得る。代替案: #858 の `display:none` 保持方式（DOM 残置）。本 Issue は R1（unmount 必須）に従い unmount を採用、#858 は非スコープ。
- **R-2: idle eviction による hidden 端末のプロセス kill。** 5 分 idle の hidden 端末はプロセスごと解放され、64KB を超える履歴は失われる。requirements A6/A7・R5 で明示的に許容済み。可視端末は pinned で保護。
- **R-3: 継続出力する hidden 端末は idle にならず evict されない。** cap 到達時に全 PTY が非 idle/pinned だと新規 spawn が `CapReached` になる。behavior R4 の「eviction または上限制御」に合致（上限制御を選ぶ）。`log()` 相当で UI に上限到達を明示する。
- **R-4: pinned 集合の同期漏れ。** frontend が active token の unregister/register を送り損ねると可視端末が evict され得る。緩和: `get_or_spawn` 直後に provisional pin、mount instance ごとの token 登録/解除、stale register を retired token で拒否、idle_timeout を十分長く（5 分）保つ。
- **R-5: activity 記録のロック競合。** 高頻度出力で registry ロックが競合し得る。緩和: activity 更新を 1 秒スロットルし、ring 書き込み（runtime 内ロック）とは別経路にする。
- **R-6: output buffer 一本化に伴う remote 回帰。** ws 独立バッファ削除で既存 remote 経路が壊れる懸念。緩和: F6 の replay と `has_subscriber()` ガードを ws テストで担保。

---

## 仮定

- A2/A5/A6/A7: requirements.md を引き継ぐ。初期値 — per-worktree cap = 32（4×8）、idle timeout = 300 秒、output buffer cap = 64KB、復元範囲 = 64KB ring 相当。
- A3: output buffer 正本は `PtyRuntime.output_buffer` に一本化し、`ws_bridge` 独立バッファを廃止する。
- A4: lightweight state = runtime output ring（直近スクロールバック）＋ pane サイズ/メタ。frontend は復元時に invoke 取得し xterm へ write。
- **D1（本設計の決定）**: lifecycle eviction の通知は `pty-exit`（プロセス終了）と別の `pty-evicted` イベントで行う。pane の扱い（残す/除去する）が異なるため。
- **D2（本設計の決定）**: active/可視端末の保護は frontend → Rust の `register_active_terminal` / `unregister_active_terminal`（active pin token）で行う。
- **D3（仮定・要確認の余地）**: 全体 max panes 初期値 = 64。A2 に明示がないため暫定値。`PtyLifecycleConfig` で変更可能。
- **D4（本設計の決定）**: idle の定義は「入力・出力・resize のいずれも idle_timeout の間なし」。activity 更新は 1 秒スロットル。

---

## Open Questions

なし（requirements.md / behavior.md ですべて解消済み。本設計で残る判断は D1〜D4 として仮定・決定で明示し、いずれもチューニング可能パラメータまたは内部設計に閉じるため、実装着手前のレビューで確認すれば足りる）。
