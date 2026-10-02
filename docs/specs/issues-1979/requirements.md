# Context

- 入力文書: https://github.com/siro33950/releash/issues/1979
- 補助資料: https://github.com/siro33950/releash/issues/1978、`docs/architecture/README.md`、`docs/architecture/CONTROLLER.md`、`docs/architecture/USECASE.md`
- 規約は、きっかけを生む駆動部（タイマー・OS の通知）を infrastructure に置き、controller がきっかけを Usecase の引数に変えて Usecase を呼ぶと定めている（`docs/architecture/README.md` の部品の一覧、`docs/architecture/CONTROLLER.md:6`）。usecase は時刻と task を持ち込まない。許されるのは、Usecase 自身が所有する非同期の排他・通知・task の協調に使う `tokio::sync` 等の実行制御 primitive までである（`docs/architecture/USECASE.md:7`）。
- 規約どおりの形の例: provider の session の題名の取り込み。Main が `infrastructure::timer::ticks` の stream を作って controller の `run` に渡し（`src-tauri/src/adaptor/controller/daemon.rs:226-232`）、controller が stream を待って Usecase を呼ぶ。item ごとの `tokio::spawn` も controller が持つ（`src-tauri/src/adaptor/controller/provider_session_title.rs:9-53`）。
- ISSUE の本文の位置は main `506f9a98` のものである。その後 #1978（`46b24a96`）が入り、terminal の読み直しの置き場所が移った。以下の位置は main `941f9a89` で確認したもの。

# Outcome

- 対象者: daemon の開発者。
- 現在の問題: daemon の中の繰り返し処理のうち 5 つで、usecase または controller が自分で task を起こし、`loop` と時刻（`sleep`・interval）を持っている。そのため、繰り返し処理の起動と停止が usecase と controller に散らばり、止め方と失敗の扱いが処理ごとに違う。usecase が時刻を扱うので、テストで時刻を差し替えにくい。
- 変更後の状態: 5 つの繰り返し処理はどれも、時刻を infrastructure の駆動部だけが作り、controller がきっかけを待って Usecase の 1 回分の操作を呼ぶ形になる。Usecase は task・`loop`・時刻を持たない。各処理の外から見た振る舞いは変わらない。

# Current Behavior

調査は main `941f9a89` のコードを読んで行った。

1. 購読の worker（`src-tauri/src/usecase/state_subscription.rs:218-297`）
   - `start_read` の中で、対象ごとに `tokio::spawn`（:226）と `loop`（:229）を起こす。
   - きっかけは 3 つ。
     - 変化の知らせ（`changes.recv()`、:235）
     - `notify_and_wait` からの待ち合わせ（`waiting_changes.recv()`、:244）
     - 外部の情報の定期の取り直し（`timer.interval(CacheTtl::EXTERNAL_INFORMATION)`、:227-228, :248）
   - 溜まった知らせは、読む前に `try_recv` で 1 回にまとめる（:252-266）。
   - 対象を読んで配信し、待っている `notify_and_wait` に終わりを知らせる（:267-294）。
   - 購読が無くなった対象の task は `reconcile_watches` が `abort` する（:368-379）。
   - タイマーは usecase の trait `SubscriptionTimer`（:14-19）を通して、gateway の `TokioSubscriptionTimer`（`src-tauri/src/adaptor/gateway/subscription_timer.rs:4-11`）から渡される（`src-tauri/src/adaptor/controller/daemon.rs:92-95`）。
2. terminal の読み直し（`src-tauri/src/usecase/terminal_surface/subscription.rs:281-321`、`schedule_terminal_refresh`）
   - #1978 で `usecase/state_subscription.rs` から移った処理。
   - 送り待ちが溢れると、controller（`src-tauri/src/adaptor/controller/api/client.rs:218`）がこれを呼ぶ。
   - 対象ごとに `tokio::spawn`（:301）と `loop`（:302）を起こし、作り直しの task を対象ごとに 1 つに保つ（:294-297）。
   - 読み直しに失敗したら失敗を配信して抜ける（:304-309）。作り直しの記録（`terminal_resets`）が空になっても抜ける（:310-318）。
   - 購読が無くなった対象の task は `stop_inactive_workers` が `abort` する（:88-104）。
3. launch の記録の保持期限（`src-tauri/src/usecase/agent_session/agent_session_launch.rs:550-554`）
   - `activate_workflow_node` が記録を `Activated` にした後、`tokio::spawn` と `tokio::time::sleep(ACTIVATED_WORKFLOW_LAUNCH_RETENTION)` で 300 秒待つ（:224-225）。その後、`activated_workflow_launches` から記録を消す。
4. terminal の checkpoint の定期の書き出し（`src-tauri/src/adaptor/controller/terminal_checkpoint.rs:31-79`）
   - controller の `run` が、dirty の知らせを受けるたびに、session ごとに `tokio::spawn`（:56）と `loop` を 1 つ起こす。
   - `tokio::time::sleep(CHECKPOINT_PERSIST_INTERVAL)`（250ms、:10, :58）を待ってから flush する。待っている間に dirty が来ていれば繰り返し、来ていなければ終わる（:59-75）。
   - 組み立ては `src-tauri/src/adaptor/controller/terminal_surface_runtime.rs:76-99`。
5. repository の scan の worker（`src-tauri/src/adaptor/controller/repository_scan.rs:15-97`）
   - usecase（`src-tauri/src/usecase/repository_state/worktree.rs:94`）が、trait `RepositoryStateWorkerRuntime`（`src-tauri/src/usecase/repository_state/runtime.rs:29-52`）の `spawn_worker` を呼ぶ。controller の実装（`repository_scan.rs:146-150`）が `tokio::spawn` する。
   - `run_worker` は `loop`（:33）の中で、`runtime.sleep(debounce)`（:85）を待ってから scan する。`sleep` の実装は controller の `tokio::time::sleep`（:152-154）。debounce は 300ms（`src-tauri/src/usecase/repository_state/service.rs:17`）。
   - 溜まった理由は `collect_pending_reasons` でまとめる（:90-96）。shutdown になったら止まる（:28-35）。
   - #1979 の本文の 4 つには入っていない。利用者の判断で対象に含めた。

# Scope / Non-goals

## Scope

- Current Behavior の 1〜5 の繰り返し処理の駆動。
- 1 に使う trait `SubscriptionTimer` と `src-tauri/src/adaptor/gateway/subscription_timer.rs`（駆動を移すと要らなくなるので削除する）。
- 5 に使う trait `RepositoryStateWorkerRuntime` の `sleep` と `spawn_worker`。
- 上の駆動を組み立てる Main（`src-tauri/src/adaptor/controller/daemon.rs`、`src-tauri/src/adaptor/controller/terminal_surface_runtime.rs` など）。

## Non-goals

- `src-tauri/src/usecase/daemon_supervision.rs` の監督の `loop`（:106 の `tokio::spawn`、`run` の `loop` と `wait_for_poll`）。#1904 で `daemon_supervision` ごと削除するので扱わない。
- `RepositoryStateWorkerRuntime` に残る `invalidation_channel`・`scan`・`scan_worktrees` の形。部品の表に無い種類の trait であることはマイルストーン #97 で扱う。
- `src-tauri/src/usecase/agent_session/agent_session_launch.rs:369`（`launch_standalone_idempotent`）と `:1111`（`record_hook_launch`）の `tokio::spawn`。どちらも 1 回だけの task で、繰り返し処理ではない。
- terminal の出力の流量制御の規則（#1888）。
- ISSUE #1979 の本文の編集。

# Requirements

- R-001: 1〜5 の繰り返し処理で、Usecase は繰り返しを回す task を起こして持ち続けず、きっかけを待つ `loop` を持たず、時刻（`sleep`・interval・遅延）を扱わない。1 回の操作の中で起こしてその場で待ち終える処理（`spawn_blocking` 等）は含まない。
- R-002: 1〜5 の繰り返し処理で、時刻を作るのは infrastructure の駆動部だけである。task の起動と、きっかけを待つ `loop` は controller が持ち、controller はきっかけごとに Usecase を呼ぶ。
- R-003: 1〜5 の繰り返し処理で、業務の判断は今どおり usecase と domain が答える。判断は次のとおり。
  - どの対象がどの知らせで影響を受けるか
  - 外部の情報を取り直すか
  - 作り直しを続けるか
  - 保持期限が来た記録を消すか
  - scan を続けるか

  controller は判断を持たない。どの対象が今購読されているかの正は usecase に 1 つだけ置き、controller に二重に持たせない。
- R-004: 購読の worker の振る舞いを保つ。
  - 対象ごとに、変化の知らせ・`notify_and_wait`・外部の情報の取り直しの間隔（`CacheTtl::EXTERNAL_INFORMATION`）で読み直して配信する。
  - 購読を始めた直後（最初の値を作っている間を含む）に届いた変化も、取りこぼさずに読み直して配信する。
  - 溜まった知らせは 1 回の読み取りにまとめる。
  - `notify_and_wait` は、その対象の読み取りの終わりを待てる。
  - 購読が無くなった対象の読み直しは止まる。
- R-005: terminal の読み直しの振る舞いを保つ。
  - 対象ごとに作り直しの処理は 1 つだけ動く。
  - 失敗したら失敗を配信して終わる。
  - 作り直しの記録が空になったら終わる。
  - 購読が無くなった対象の作り直しは止まる。
- R-006: launch の記録の保持期限の振る舞いを保つ。`Activated` になった記録は 300 秒後に消える。
- R-007: terminal の checkpoint の書き出しの振る舞いを保つ。dirty になった session は 250ms 後に flush される。flush している間に再び dirty になっていれば、もう一度 250ms 待って flush する。250ms 待っている間に届いた dirty は、その後の flush に含まれる。
- R-008: repository の scan の worker の振る舞いを保つ。
  - 知らせを受けたら debounce（300ms）を待ち、溜まった理由をまとめて scan する。
  - shutdown になったら止まる。
- R-009: 1〜5 の繰り返し処理は、テストで時刻を差し替えられる。駆動部が作る時刻のきっかけを、テストが作った偽のきっかけに置き換えて動かせる。
- R-010: `SubscriptionTimer` と `adaptor/gateway/subscription_timer.rs` は存在しない。`RepositoryStateWorkerRuntime` は `sleep` と `spawn_worker` を持たない。

# Assumptions

なし
