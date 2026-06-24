# Design

Startup orphan cleanup を non-blocking service 化する実装設計。requirements.md / behavior.md（Issue #1216）を実装方針・責務分割・データ構造・エラー処理・テスト方針へ落とし込む。

正本: `docs/releash-performance-architecture-audit.md`（M3 / 項目 4）。

## 概要

- 現状、Tauri `setup` クロージャ（`src-tauri/src/lib.rs` 431-442）は `cleanup_orphan_processes` を別スレッドで `spawn` した直後に `.join()` しており、cleanup（PID 走査・SIGTERM・最大 2 秒 sleep・SIGKILL）の完了まで setup が同期ブロックされる。これが visible startup（first window ready）を遅延させうる。
- 本設計では `.join()` を廃し、cleanup を **バックグラウンドタスク**として走らせて setup を即座に先へ進める。
- cleanup と新規 spawn の順序保証は、setup のブロッキング join ではなく **Rust 側の明示的な完了ゲート（cleanup completion gate）** で担保する。新規 bridge プロセスの spawn は、cleanup 完了を待ってから OS に新しい pgid を確保させる。
- cleanup の実行状態（完了 / 失敗）・対象数（走査数・処理数・skip 数・失敗数）を、user data を含まない safe metadata として構造化ログと既存 telemetry 経路（`other::telemetry`）に公開する。
- 既存の孤児判定アルゴリズム（owner 同一性 / PID 再利用検出 / 保守的 skip / SIGTERM→SIGKILL 昇格）は変更しない（requirements 非スコープ）。

### 順序保証の本質（設計の前提となる事実）

実コードを確認した結果、`.join()` が守っている不変条件は「新規 spawn の保護」のうち **pgid 再利用レース** に限られることが分かった。

- `cleanup_orphan_processes`（`bridge_common.rs` 1066-1167）は各 `.pid` ファイルについて `owner_app_pid` が alive かつ `owner_start_time` が一致すれば「live instance 所有」として skip する。現インスタンスが新規 spawn 時に書く PID ファイルは `owner_app_pid = std::process::id()`（=自分）・`owner_start_time`（=自分の起動時刻）を持つため、**既存の owner 同一性判定で既に skip される**。よって behavior R2 の「自インスタンス起動以降に作成された PID は対象集合に含まれない」は、新規ファイルそのものについては既存ロジックで成立している。
- 一方、`.join()` が本質的に塞いでいるのは次のレースである:
  1. 前回クラッシュした旧インスタンスの stale ファイル `S`（owner=死亡 PID, pgid=`PG`）が残る。
  2. 旧プロセス群が既に終了し、OS が `PG` を解放する。
  3. 現インスタンスが新規 bridge を spawn し、OS が偶然 `PG` を再割当する。
  4. cleanup が `S` を処理 → owner 死亡なので孤児判定 → `killpg(PG, 0)` が alive（=新規プロセス群）→ SIGTERM/SIGKILL で**新規プロセス群を誤 kill**。
- `.join()` はこのレースを「新規 spawn が起きる前に stale ファイルを全て除去し切る」ことで塞いでいた。non-blocking 化すると cleanup と spawn が並走しうるため、この順序を別機構で保証する必要がある（requirements 要求 2）。

この事実から、本設計の順序機構は「新規 spawn 経路が cleanup 完了を待ってから pgid を確保する」**完了ゲート**を採用する（理由は「アーキテクチャと責務分割」§ ordering を参照）。

## 変更対象

| ファイル | 変更内容 |
|---|---|
| `src-tauri/src/lib.rs` | setup の cleanup spawn から `.join()` を除去。背景スレッドで cleanup を実行し、完了時に gate を開放 + telemetry/ログ記録。gate を managed state として登録。 |
| `src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs` | `cleanup_orphan_processes` が `OrphanCleanupReport` を返すよう変更（集計）。spawn 経路（`spawn_bridge_process` / `register_external_agent_process`）に gate 待ちを挿入。`CleanupGate` 型を追加。 |
| `src-tauri/src/infrastructure/agent_session/runtime/mod.rs` | `CleanupGate` / `OrphanCleanupReport` の re-export（必要に応じて）。 |
| `src-tauri/src/other/telemetry/mod.rs` | cleanup 観測用の記録関数（counter）を追加。 |
| `src-tauri/src/other/telemetry/attributes.rs` | cleanup 用の metric / attribute（status・count 種別）を追加。 |

フロントエンドは変更しない（観測は safe metadata の提供までで UI 表示は非スコープ。requirements 非スコープ / `.claude/rules/rust-first-logic.md`）。

## アーキテクチャと責務分割

### レイヤー配置

cleanup・gate・spawn 待ちは外部プロセス（OS シグナル / fork）を扱う低レベル処理であり、現状どおり `infrastructure/agent_session/runtime/`（`bridge_common.rs`）に置く。観測の公開は横断的関心事として `other/telemetry/` が担う。`setup`（composition root 相当）でのみ両者を配線する。本 Issue では `bridge_common.rs` の module 分割は行わない（requirements 非スコープ）。

### non-blocking 化（Rule R1）

`lib.rs` setup の `#[cfg(unix)]` ブロックを次のように変える:

- `std::thread::spawn(...).join()` の `.join()` を除去し、JoinHandle は drop する（detach）。cleanup は背景スレッドで実行され、setup は完了を待たずに `record_startup(AppStartup)` 以降へ進む。
- cleanup は引き続き起動時に 1 回だけ走る（behavior R1 Scenario 2）。
- cleanup は同期実装（`std::thread::sleep` を使う）なので、async task ではなく **専用 std スレッド**で走らせる（tokio worker を 2 秒間占有しないため）。

### ordering（Rule R2）— cleanup completion gate

新しい型 `CleanupGate` を導入し、Tauri の managed state（`app.manage(Arc<CleanupGate>)`）として配線する。

責務:
- 背景 cleanup スレッドは、cleanup 完了時（成功 / 失敗を問わず）に gate を「開放」する。
- 新規 bridge プロセスの spawn 経路は、**実際に子プロセスを `spawn()` する直前**（`bridge_common.rs` 3823 / 7xxx の `cmd.spawn()` 直前。`setsid` 後に新 pgid が確保される地点の手前）で gate の開放を待つ。これにより stale ファイルが残ったまま新 pgid が確保されることがなくなり、§「順序保証の本質」の pgid 再利用レースが構造的に消える。

待ち地点を spawn 直前にする理由: behavior R2 が要求するのは「cleanup 完了前に spawn された新規プロセスを誤 kill しない」こと。新 pgid の確保（`cmd.spawn()`）を cleanup 完了後に直列化すれば、cleanup が走査する stale ファイルの pgid 集合と、新規プロセスの pgid が時間的に交わらない。owner 同一性判定（既存）と合わせ、新規プロセスは二重に保護される。

待ち地点が visible startup の critical path に乗らない理由: `spawn_bridge_process` / `register_external_agent_process` は frontend からの `init_agent_sessions`（Tauri command）経由でのみ呼ばれ、これは first window ready 後にしか発火しない。よって gate 待ちは visible startup を遅延させない（Rule R1 と両立）。通常起動では孤児が無く cleanup は数 ms で完了するため、待ちは事実上ゼロ。孤児が多い場合のみ最初の session 起動が cleanup 完了（worst case 数秒）まで待つ。これは visible startup 外であり許容する（後述「仮定」）。

ゲート同期プリミティブ: `tokio::sync::watch::<bool>`(初期値 `false`) を採用する。

- `CleanupGate { tx: watch::Sender<bool>, rx: watch::Receiver<bool> }`（または `Sender` のみ保持し `subscribe()` で receiver を配る）。
- 背景スレッドは完了時に `tx.send(true)`（sync 文脈から呼べる）。
- spawn 経路（async）は `let mut rx = gate.subscribe(); if !*rx.borrow() { rx.wait_for(|v| *v).await; }` で待つ。
- `watch` を使う理由: 「ワンショット完了を複数 async waiter へブロードキャスト」「sync スレッドから signal」「await 前後の取りこぼし（notify の lost-wakeup）が無い」を同時に満たすため。`Notify` だと create-future-before-check の取りこぼし対策が要るので避ける。
- **開放保証**: cleanup スレッドは正常終了・panic 捕捉・スレッド起動失敗のいずれでも必ず `gate.open()` を呼ぶ。gate は cleanup 終了の事実だけで開き、時間経過では開かない。これにより cleanup が長時間かかる場合でも新規 pgid 確保と cleanup が並走しない。

非 unix（Rule R6）: cleanup は元から `#[cfg(unix)]`。非 unix では gate を「最初から開放済み（`true`）」で構築し、spawn 経路の待ちは即座に通過する。`#[cfg(not(unix))]` 経路では cleanup spawn 自体を行わない。

### observation（Rule R3 / R4）

`cleanup_orphan_processes` を「副作用のみ・戻り値なし」から「集計レポートを返す」関数へ変更する:

- 走査中に `OrphanCleanupReport` を組み立てて返す。判定・kill のロジック（owner 同一性 / PID 再利用 / SIGTERM→SIGKILL）は一切変えず、各分岐でカウンタを加算するだけにする。
- 背景スレッドは戻り値レポートを受け取り、(1) 構造化ログを 1 行出力、(2) `other::telemetry` の新規記録関数へ渡す。
- telemetry は既存 counter 方式（`is_performance_active()` ガード・`#[cfg(test)]` の `record_test_metric`）に倣う。観測値は **safe metadata（status と件数）のみ**。worktree path・session id・command body 等の user data は一切含めない。cleanup の individual ログ行も同じ境界に揃え、PID ファイルパス・session 派生値・PID/PGID 値を出さず、safe な reason と最終集計ログ（status / scanned / processed / skipped / failures）だけを出力する。

## データモデルまたは型

### `OrphanCleanupReport`（`bridge_common.rs`, `#[cfg(unix)]`）

```rust
#[cfg(unix)]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct OrphanCleanupReport {
    /// 走査した `.pid` ファイル数（拡張子 .pid のみ。.tmp 等は除外）。
    pub scanned: usize,
    /// 孤児として処理した数（SIGTERM/SIGKILL 送出、または PID ファイル除去）。
    pub processed: usize,
    /// live owner / 検証不能 / legacy format 等で保守的に skip した数（issue #1024）。
    pub skipped: usize,
    /// 読み取り失敗等で処理できなかった数。
    pub failures: usize,
}
```

- `scanned` `processed` `skipped` `failures` はいずれも件数のみで user data を含まない（Rule R4）。
- `processed` の定義は requirements 仮定どおり「SIGTERM/SIGKILL を送った、または PID ファイルを除去した数」。invalid pgid の除去・孤児の kill+除去はいずれも `processed` に含める。
- 状態（完了 / 失敗）は別途レポートとは独立に表現する: 関数が正常 return すれば「完了」、`failures > 0` でも関数自体は完走するので「完了（一部失敗あり）」とし、`failures` で失敗有無を区別する。スレッド自体が panic した場合は背景スレッド側で捕捉して「失敗」として記録する（後述エラー処理）。

### `CleanupGate`（`bridge_common.rs`）

```rust
pub(crate) struct CleanupGate {
    tx: tokio::sync::watch::Sender<bool>,
}

impl CleanupGate {
    /// unix: 未完了(false)で構築。非 unix: 開放済み(true)で構築。
    pub fn new(initially_open: bool) -> Self { /* watch::channel(initially_open) */ }
    /// cleanup 完了時に背景スレッドから呼ぶ（sync 文脈可）。
    pub fn open(&self) { let _ = self.tx.send(true); }
    /// spawn 直前に await。開放済みなら即 return。cleanup 終了通知まで待つ。
    pub async fn wait_until_open(&self) { /* subscribe → wait_for(|v| *v) */ }
}
```

managed state として `app.manage(Arc::new(CleanupGate::new(cfg!(unix))))`（unix は false、非 unix は true）。spawn 経路では `app.try_state::<Arc<CleanupGate>>()` で取得し、取得できなければ（テスト等で未配線）待たずに継続する。

### telemetry 追加（`attributes.rs` / `mod.rs`）

- `attributes.rs`: cleanup 件数の種別を表す attribute（例 `KEY_OPERATION` 値 `"startup.orphan_cleanup"`、status は既存 `OpStatus::{Success, Failure}` を再利用）。件数は counter の値として記録するか、種別ごとに `OpStatus` 的な軽量 enum を足す。詳細命名:
  - metric 名: `releash.startup.orphan_cleanup`（u64 counter）。
  - attribute: `status`（`success` / `failure`、`failures>0 || panic` で `failure`）、`outcome`（`scanned` / `processed` / `skipped` / `failures` の件数種別）。各 outcome 種別ごとに件数を `add(n, attrs)` する。
- `mod.rs`: `record_orphan_cleanup(report: &OrphanCleanupReport, failed: bool)` を追加。`is_performance_active()` ガード・`#[cfg(test)]` の `record_test_metric` 併記を既存関数（`record_hot_path_duration` 等）と同形で実装。

> 命名・粒度は実装時に既存 attribute 規約（allowlist テスト `usage_events_are_allowlisted` 等）と整合させる。outcome を attribute で分けず 4 つの専用 counter にする案もあるが、既存が「1 counter + operation/status attribute」方式なので踏襲する。

## 処理フロー

### 起動時（unix）

1. setup 開始時に `CleanupGate::new(false)` を `app.manage`。
2. window 準備完了で `record_startup(FirstWindowReady)`（既存 `lib.rs` 299）。**ここまで cleanup は関与しない**（Rule R1）。
3. `#[cfg(unix)]` ブロック: `data_dir` を clone し、専用 std スレッドを spawn（`.join()` しない）。
4. setup は即座に `record_startup(AppStartup)` 以降へ進み完了。
5. 背景スレッド内:
   a. `let report = cleanup_orphan_processes(&data_dir);`（panic は `catch_unwind` で捕捉、捕捉時 `failed=true`・空レポート扱い）。
   b. 構造化ログ 1 行（件数のみ）。
   c. `telemetry::record_orphan_cleanup(&report, failed)`。
   d. `gate.open()`（成功 / 失敗いずれでも必ず開放。失敗で開放しないと spawn が進まないため）。

### 新規 bridge spawn 時（unix）

1. `spawn_bridge_process` / `register_external_agent_process` が `cmd` を構築（env / `pre_exec(setsid)` 設定済み）。
2. `cmd.spawn()` の**直前**で `if let Some(gate) = app.try_state::<Arc<CleanupGate>>() { gate.wait_until_open().await; }`。
3. gate 開放後に `cmd.spawn()` → 新 pgid 確保 → `save_pgid`（既存どおり owner=自 pid を記録）。
4. 以降は既存フロー。

### 非 unix

- cleanup スレッドを spawn しない。gate は `true` 構築で spawn 経路の待ちは即通過（Rule R6）。

## エラー処理

- **cleanup 内部の I/O / parse 失敗**: 既存方針を維持（ファイル読めない→warn ログして continue、legacy/unknown format→保守的 skip、invalid pgid→除去）。個別ログには PID ファイルパス・session 派生値・PID/PGID 値を含めず、safe な reason のみを出す。これらは `failures` / `skipped` / `processed` に計上するのみで関数は完走する。
- **背景スレッドの panic**: `std::panic::catch_unwind` で捕捉。捕捉時は `failed=true` として telemetry に `status=failure` を記録し、**必ず `gate.open()` を呼ぶ**（spawn を無期限に塞がない）。
- **gate が開かない異常**: cleanup panic / スレッド起動失敗は捕捉して `gate.open()` する。`CleanupGate` の sender が閉じるなど本来起きない状態では warn ログのうえ spawn を継続する。
- **gate state 未配線（テスト/異常）**: `try_state` が `None` を返したら待たずに継続。
- **telemetry 無効時**: 既存どおり `is_performance_active()` が false なら no-op。

## テスト方針

`#[cfg(test)] mod tests`（`bridge_common.rs` / `telemetry/mod.rs`）に追加。CI は `cargo clippy -- -D warnings` / `cargo test`。

1. **Rule R7 / R2 — 順序保証（pgid 再利用レース）**:
   - 既存の実プロセスを使うテスト（`cleanup_orphan_processes_kills_alive_process_group` 19580 付近）の構造を流用。
   - 「現インスタンス所有」を表す PID ファイル（`owner_app_pid = std::process::id()`、`owner_start_time = get_process_start_time(自分)`）を置き、生きたプロセス群を指す状態で `cleanup_orphan_processes` を呼び、**そのプロセス群が kill されない**（owner 同一性 skip）ことを assert。これが「自インスタンスが spawn した新規プロセスは誤 kill されない」の核を検証する。
   - `CleanupGate` の単体テスト: `tokio` test で、未開放時 `wait_until_open()` が pending（短い timeout で確認）、`open()` 後に即 return、複数 waiter 全てが解放されること、managed state に閉じた gate を置いた production spawn 直前 helper が fake spawn closure を実行しないこと。
2. **Rule R3 / R4 — observation**:
   - `cleanup_orphan_processes` の戻り `OrphanCleanupReport` のカウントを検証（空 dir → 全 0、stale 除去 → `processed` 加算、legacy format → `skipped` 加算、invalid pgid → `processed` 加算）。既存の `cleanup_orphan_processes_*` テスト群を report 返り値の assert に拡張。
   - `telemetry::record_orphan_cleanup` を既存 test harness（`lock_test_telemetry` / `reset_test_metrics` / `test_metric_records`）で検証: `status=success/failure` と各 outcome 件数が記録されること、記録 attribute に user data キー（path/session 等）が**含まれない**こと（allowlist 的 assert）。
   - cleanup 個別ログが PID ファイルパスや process id を format しないことを、cleanup 関数ソースの regression test で検証する。
3. **Rule R5 — 孤児回収維持**:
   - 既存の「stale ファイル除去」「live owner skip」「PID 再利用で proceed」「保守的 skip」テスト群がそのまま green であること（アルゴリズム不変の回帰防止）。SIGTERM→sleep→SIGKILL 昇格手順を変えていないことを既存テストで担保。
4. **Rule R6 — 非 unix**:
   - `#[cfg(not(unix))]` で gate が即開放であること（コンパイル経路の確認）。cleanup spawn が無いことは構造で担保。
5. **Rule R1 — non-blocking**:
   - 厳密な実時間 assert は単体テストに馴染まないため、構造で担保（setup から `.join()` を除去・cleanup を detach スレッド化・gate 待ちを spawn 経路のみに限定）。gate の単体テストで「visible startup 経路は gate を待たない」設計を間接的に裏付ける。

## リスクと代替案

- **pgid 再利用レースの残余（gate を採らない場合）**: 「snapshot で起動時の `.pid` ファイル集合だけを cleanup 対象にする」案単独では、stale ファイルの pgid が新規プロセス群へ再割当されるケースを塞げない（§「順序保証の本質」step 4）。よって gate（spawn を cleanup 完了後へ直列化）を採用する。snapshot 案は behavior R2 の文面（「自インスタンス起動以降に作成された PID は対象外」）と一致するが、それは既存 owner 同一性判定で既に満たされており、追加の安全性を生まないため不採用。
- **最初の session 起動の遅延**: 孤児が多いと最初の `init_agent_sessions` 起因の spawn が cleanup 完了（worst case 数秒以上）まで待つ。visible startup 外なので requirements 上は許容。通常は孤児ゼロで待ち ≒ 0。cleanup 完了前に spawn を進める timeout は設けない。
- **gate 配線漏れ**: managed state 未登録だと spawn が待たず pgid 再利用レースに戻る。setup で必ず `manage` し、`try_state` 失敗時はログを残す。
- **telemetry 粒度**: outcome を attribute で分ける方式が既存 dashboards と噛み合うかは運用側に依存。既存 counter 方式を踏襲し、必要なら後続で粒度調整（本 Issue では最低限の件数を満たす）。
- **代替: init_agent_sessions 全体を gate で待つ**: command 先頭で待つ案もあるが、待ちを「実 spawn 直前」に絞るほうが、session メタ取得など spawn を伴わない経路を不必要に遅延させない。後者を採用。

## 仮定

- cleanup は従来どおり起動時 1 回・専用 std スレッドで実行し、setup は完了を待たない（requirements 仮定「cleanup の起動タイミング」）。
- 順序保証は `CleanupGate`（`tokio::sync::watch` ベースの完了ゲート）で担保し、新規 bridge の `cmd.spawn()` を cleanup 完了後へ直列化する。既存 owner 同一性判定と二重で新規プロセスを保護する。
- gate 待ちは spawn 経路（`init_agent_sessions` 起点、visible startup 外）でのみ発生し、first window ready を遅延させない。
- 観測は構造化ログ + 既存 telemetry 経路（`other::telemetry`、counter 方式）。frontend / 運用向け専用 read コマンドは追加しない（合意済み）。
- 対象数 = `scanned`（走査 `.pid` 数）/ `processed`（kill 送出 or ファイル除去数）/ `skipped`（保守的 skip 数）/ `failures`（処理失敗数）。
- 判定アルゴリズム（PID 再利用検出 / owner 同一性 / SIGTERM→最大 2 秒→SIGKILL 昇格 / 保守的 skip, issue #1024）は不変（requirements 非スコープ）。
- 対象は unix（`#[cfg(unix)]`）限定。非 unix では cleanup を行わず gate は即開放。

## Open Questions

なし（requirements.md / behavior.md で全て解消済み。本設計で残る選択は仮定・リスクに明記し、設計判断として確定した）。
