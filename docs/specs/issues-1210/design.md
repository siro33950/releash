# Design

`RepositoryStateService`（#1210）の実装方針を定める。`requirements.md` / `behavior.md` を前提とし、本書では配置・型・処理フロー・エラー処理・テスト方針まで具体化する。

参照: `requirements.md`（仮定 A1〜A7）/ `behavior.md` / `docs/releash-performance-architecture-audit.md` M1。

## 概要

worktree（repo）ごとに分散している file watcher / Git dir watcher / git scan / read model 生成を、1 worktree につき 1 系統の `RepositoryStateService` に集約する。watcher の notify は invalidate（再走査要求）だけを発行し、debounce 後に background worker が 1 scan サイクル（read model 生成サイクル）を実行して versioned snapshot を生成する。status / diff stats / branch cards / worktree dirty count / diff file tree はこの単一 snapshot から導出する。ここでの「集約 scan」は全 linked worktree を物理的に 1 回の git2 走査へ統合する意味ではなく、current repo_path は 1 open の同一 `Repository` handle から status / diff stats / current dirty を導出し、別 linked worktree は workdir ごとに 1 dirty 走査を同じ scan サイクル内へ閉じる、という定義である。

本 Issue では新 command 名 `get_review_snapshot` は導入せず（A1）、既存 command（`get_git_status` / `get_status_diff_stats` / `build_diff_file_tree` / `list_branches_with_status`）を service の snapshot cache を読む経路へ寄せ、watcher 重複と走査重複を解消する。

## 変更対象

### 新規

- `src-tauri/src/usecase/repository_state/mod.rs` — モジュール公開
- `src-tauri/src/usecase/repository_state/service.rs` — `RepositoryStateService`（worktree レジストリ・ensure_watching・get_snapshot）
- `src-tauri/src/usecase/repository_state/worktree.rs` — `WorktreeState`（per-worktree の watcher / worker / snapshot 保持）
- `src-tauri/src/usecase/repository_state/snapshot.rs` — `RepositorySnapshot` / フラグ / version
- `src-tauri/src/usecase/repository_state/worker.rs` — invalidate → debounce → scan → supersede 判定
- `src-tauri/src/usecase/repository_state/scanner.rs` — 1 scan サイクルの集約（status gateway の集約 scan と branch card snapshot から snapshot を組む）

### 改修

- `src-tauri/src/watcher.rs` — `start_watching` / `start_git_dir_watching` を service への委譲に置換。watcher 起動と debouncer 所有を service へ移動。`resolve_git_watch_paths` / `classify_git_dir_events` / `canonicalize_event_path` は service から再利用（pub(crate) 化）。callback 内の `build_branch_list_sync`（同期 `list_branches_with_status`）を撤去し invalidate 発行のみにする。
- `src-tauri/src/lib.rs` — setup で `Arc<RepositoryStateService>` を構築し `app.manage`。既存 watcher への usecase 注入を service 構築に置換。
- `src-tauri/src/adaptor/controller/state.rs` — `AppState` に `repository_state: Arc<RepositoryStateService>` を追加。
- `src-tauri/src/adaptor/controller/command/repository/status.rs` — `get_git_status` / `get_status_diff_stats` を service snapshot 経由に変更。
- `src-tauri/src/adaptor/controller/command/code/diff.rs` — `build_diff_file_tree` の供給元を snapshot 由来に寄せる（後述の互換方針）。
- `src-tauri/src/adaptor/gateway/repository/status.rs` — `include_ignored(true)` を default で外し、ignored 取得を opt-in 引数化。

### 変更しない

- `behavior.md`
- frontend の command surface（#1211 で置換）。本 Issue では frontend は既存 invoke と既存 event listen のままで、Rust 側が単一 service を裏に持つ。

## アーキテクチャと責務分割

層の責務は `rust-first-logic` と既存 clean architecture（A4）に従う。

```
lib.rs (composition root)
  └─ Arc<RepositoryStateService>  ── app.manage / AppState に注入
        │  worktree レジストリ: HashMap<PathBuf(正規化), Arc<WorktreeState>>
        │
        ├─ subscribe(path)              : 冪等。無ければ WorktreeState 生成し watcher 起動
        ├─ get_snapshot(path, opts)     : 管理済みなら cache、未管理なら一時 scan の version 0 snapshot
        └─ request_ignored(path)        : ignored opt-in 取得

WorktreeState (per worktree, 1 系統)
  ├─ watchers: 単一の file watcher(recursive workdir) + Git dir watcher(refs/heads, HEAD, index)
  │     notify callback → invalidate チャンネルへ送信のみ（重い走査をしない）
  ├─ snapshot: parking_lot::RwLock<Arc<RepositorySnapshot>>
  ├─ version: AtomicU64 / requested_generation: AtomicU64（supersede 判定）
  └─ worker(runtime port): invalidate 受信 → debounce(300ms) → scanner 実行 → supersede 判定 → snapshot 確定 → event emit

scanner
  └─ RepositoryUsecase / CodeUsecase へ委譲して 1 snapshot を構築
       （status / diff stats / branch cards / dirty count / diff tree を 1 scan サイクル内で生成）
```

- **service**: worktree のライフサイクルと冪等な購読管理。複数 subscriber が同一 worktree を要求しても 1 つの `WorktreeState` を共有する（behavior「watcher 重複起動しない」）。read 経路だけでは `WorktreeState` / watcher を作らず、未管理 path は一時 scan 結果を返す。
- **WorktreeState**: 1 worktree の watcher・snapshot・version・worker を所有。`Drop` で watcher と worker を停止。
- **worker**: 非同期経路の中核。notify の同期実行禁止（behavior Rule「notify callback は invalidate だけ」）を担保。spawn / debounce sleep / blocking scan は `RepositoryStateWorkerRuntime` port で注入し、具体 Tokio 実装は gateway 側に置く。
- **scanner**: read model 集約。status gateway の集約 scan で current repo_path を 1 回だけ `client::open` し、同一 `Repository` handle から `statuses()` 1 回、`diff_tree_to_index` 1 回、`diff_index_to_workdir` 1 回、`Patch::from_diff` による FileDiffStat を導出する。branch cards は同じ scan サイクル内で生成し、current worktree の dirty count は集約 scan の status 結果を再利用する。別 linked worktree の dirty count は workdir が異なるため個別走査になるが、同一 scan サイクル内で worktree ごとに 1 回だけ算出して snapshot に格納する。これにより各 read model が「個別 command から独立に git2 走査を起動」しない（requirements 要求 2）。`releash-base` GC は worker が snapshot commit 採用後に確定 branch names で実行し、read command では実行しない。

scanner は usecase 層から既存 usecase（trait 経由の read model query）を呼ぶため、層の依存方向（usecase → gateway は trait 越し）を壊さない。

## データモデルまたは型

### RepositorySnapshot（snapshot.rs）

```rust
pub struct RepositorySnapshot {
    pub version: u64,
    pub flags: SnapshotFlags,
    pub status: Vec<FileStatusDto>,        // ignored を含まない（default）
    pub diff_stats: Vec<FileDiffStat>,
    pub branch_cards: Vec<BranchCardDto>,  // worktree dirty count は dirty_count フィールドに内包
    pub diff_file_tree: Vec<DiffTreeNodeDto>,
}

pub struct SnapshotFlags {
    pub stale: bool,    // 最新変更を未反映で background refresh 中
    pub loading: bool,  // 初回 / 再 scan 進行中
    pub limited: bool,  // threshold による打ち切り（本 Issue では常に false 既定。閾値は #1211 で確定 = A6）
}
```

- snapshot は `Arc` で共有し、worker が確定時に丸ごと差し替える（read 側はロック短時間）。
- 初期状態（scan 未完了）: `version = 0`, `loading = true`, 各 read model は空。

### 通知 event（A5）

Tauri event を version 付きで emit する。

```rust
#[derive(Clone, Serialize)]
pub struct RepositorySnapshotChangedEvent {
    pub worktree_path: String,  // 正規化前の subscriber 識別子（呼び出し元と一致）
    pub version: u64,
    pub stale: bool,
    pub loading: bool,
    pub limited: bool,
}
```

- event 名: `repository-snapshot-changed`。
- 既存の `file-change` / `git-status-changed` / `branch-list-sync` は後方互換のため当面残すが、内部発火源は service の worker に一本化する。frontend の listen はそのまま機能する。
- remote 向け `WsMessage::BranchListSync` は worker 確定時に従来同様 emit（ws bridge）。重い `list_branches_with_status` の同期実行を callback から worker へ移すだけで、外向き挙動は不変。

### opt-in 取得オプション

```rust
pub struct SnapshotQueryOptions {
    pub include_ignored: bool,  // default false
}
```

`include_ignored = true` の取得は default snapshot を汚さず、ignored を含む status を別途算出して返す（snapshot cache は non-ignored を保持）。`status.rs` の `include_ignored(true)` は引数で制御する形に変更する。

## 処理フロー

### 初回購読 / 初回 scan（behavior「loading を経て最初の snapshot」）

1. frontend が `start_watching` / `start_git_dir_watching` を invoke した場合だけ、service が `subscribe(path)` を呼ぶ。未登録なら `WorktreeState` を生成し watcher と worker を起動、`requested_generation` を 1 にして初回 invalidate を投入する。
2. read command が `get_snapshot(path)` を呼んだ時、管理済み worktree があれば cache を返す。未管理 path では `WorktreeState` / watcher を作らず、scanner を同期実行した version 0 の ready snapshot を一時的に返す。
3. 管理済み worktree の worker が初回 scan 完了で `version = 1` の snapshot を確定、`loading = false` で `repository-snapshot-changed` を emit。
4. frontend は event を受けて再 invoke し、確定 snapshot 由来の結果を表示する。

### 変更検知（behavior「invalidate → debounce → 1 回 scan」）

1. file / Git dir watcher の notify callback が発火。callback は `requested_generation += 1` し invalidate チャンネルへ送信するだけ（重い走査をしない）。
2. worker は invalidate を受け、300ms debounce（A7）でこの間の追加 invalidate を吸収（`requested_generation` が上がるだけ）。
3. debounce 満了で `start_gen = requested_generation` を読み、scanner を実行（git2 は同期のため `tokio::task::spawn_blocking`）。
4. scanner は current repo_path の status / diff stats / current dirty を status gateway の集約 scan から導出し、branch cards と diff tree を同じ scan サイクル内の snapshot parts として組み立てる。

### stale 即時返却（behavior「scan が長い場合」）

- background refresh 中（worker が新 scan 実行中で未確定）に `get_snapshot` が呼ばれたら、保持中の旧 snapshot を `stale = true` を立てたコピーとして即時返す。
- 確定後、新 version の snapshot を `stale = false` で通知する。

### cancel / supersede（behavior「古い scan を打ち切り、上書きしない」）

git2 走査は途中キャンセルできないため、**完了時 supersede** を採る。

1. worker は scan 後に `requested_generation == start_gen` を比較。
2. 一致しなければ（scan 中に新 invalidate が来た）結果を破棄し、最新 generation で再 scan する（古い結果で snapshot を上書きしない）。
3. 一致すれば `version += 1` で snapshot を確定し emit。
4. `version` は worker 単一直列のため単調増加が保証され逆行しない。

この方式は「進行中 scan の即時中断」ではなく「古い結果を採用せず最新で再走査」だが、外部観測（最終 snapshot が最新状態を反映・version 逆行なし）は behavior の Rule を満たす。中断不可である点は「仮定」に明記する。

### diff file tree の供給（互換方針）

現状 `build_diff_file_tree` は frontend が diff stats から組んだ entries を渡す pull 型。本 Issue では tree 構築を scanner 内（diff stats から `DiffTreeNodeDto` を組む既存ロジック流用）へ移し snapshot に含める。`build_diff_file_tree` command は当面 entries-only のシグネチャを維持し、統合 tree は `get_head_diff_file_tree_snapshot` の `combined_tree` から取得できるようにする（frontend 置換は #1211）。

## エラー処理

- **scan 失敗**（repo open 失敗 / git2 エラー）: worker は当該サイクルの snapshot 確定を行わず、直近の確定 snapshot を保持。`loading` を下げ、エラーは `tracing` でログ。frontend には version 更新を出さない（古い表示を維持）。リトライは次の invalidate / 再 scan に委ねる。
- **worktree 削除 / パス消失**: watcher エラーを契機に `WorktreeState` を service レジストリから除去し worker を停止。
- **command 経路**: `get_snapshot` は snapshot 未確定でもエラーにせず loading snapshot を返す（UI をブロックしない）。repo path 不正など回復不能な入力は従来どおり `AppError` を返す。
- **エラー型**: service 専用に `RepositoryStateError`（usecase 層）を定義し、既存 `UsecaseError` / `AppError` へ `From` 変換。各モジュール専用エラー型の規約に従う。
- **watcher 起動失敗**: `ensure_watching` はエラーを返すが、既存 `start_watching` の `Result<u64, String>` 互換を保つため文字列化して返す。

## テスト方針

Rust 単体テスト（`#[cfg(test)] mod tests`）。git2 は `git/mod.rs::test_helpers`（`create_test_repo` / `create_initial_commit` / `add_and_commit`）を再利用。tokio 非同期部は `#[tokio::test]`。

- **集約 scan の正準性**: status gateway の集約 scan が既存経路（`get_git_status` / `get_status_diff_stats`）の結果と一致し、scanner snapshot が既存 `list_branches_with_status` / diff tree と一致する（ignored 既定除外を除く）。behavior「非回帰」Rule に対応。
- **走査回数**: current repo_path の集約 scan が `statuses()` を 1 回だけ実行すること、branch cards 生成で current dirty を再走査せず、別 linked worktree dirty を worktree ごとに 1 回だけ走査することをカウンタで検証する。
- **version 単調増加 / 逆行なし**: 連続 invalidate で version が増加のみすることを検証。
- **supersede**: scan 中に generation を進めた場合、古い結果が採用されず再 scan され、最終 snapshot が最新 generation 由来であることを検証（scanner にフックを挟み generation を人工的に進める）。
- **stale 遷移**: refresh 中の `get_snapshot` が旧 version を `stale = true` で返し、確定後に `stale = false` の新 version へ遷移。
- **loading 遷移**: 初回購読で `loading = true` → 完了で `version >= 1`, `loading = false`。
- **debounce 集約**: debounce 期間内の複数 invalidate が 1 回の scan に集約されること（scan 実行回数をカウンタで検証）。
- **ignored opt-in**: default snapshot に ignored が含まれず、`include_ignored = true` で含まれる。
- **watcher 単一性**: 同一 worktree への複数 `ensure_watching` でレジストリ entry / watcher が 1 つ（重複起動しない）。
- **複数 worktree 独立**: W1 の invalidate が W2 の version に影響しない。

debounce / 非同期タイミングはテスト安定化のため、debounce 値と「scan 実行関数」を注入可能にする（テストで 0ms 相当・同期スタブ scanner を差し込む）。

## リスクと代替案

- **R1: 既存 command を snapshot 経由へ寄せる際の回帰**。`get_git_status` 等が即時計算から snapshot 参照に変わるため、初回は loading 空 snapshot を返しうる。緩和: command は未確定時に同期で初回 scan を待つ同期パス（短時間 block）も用意するか、空 + event 通知で frontend 再取得に委ねる。本設計は後者を既定とし、UI が空表示を一瞬挟む可能性を `limited`/`loading` で表現。代替: 初回のみ同期 scan を待つ（実装はやや複雑、初回レイテンシ増）。
- **R2: git2 scan の中断不可**。完了時 supersede のため、巨大 repo で無駄な完走が起きうる。緩和: debounce で invalidate を束ねる / 将来 scanner を段階分割。代替: scan を細粒度ステップ化しチェックポイントで generation 比較（#1211 の review file view と合わせて検討）。
- **R3: snapshot メモリ常駐**。worktree 数 × snapshot サイズ分のメモリを保持。緩和: 非アクティブ worktree の `WorktreeState` を LRU / close で解放。
- **R4: 二重通知**。新 `repository-snapshot-changed` と旧 event を併存させる間、frontend が二重 refresh しうる。緩和: 旧 event の発火も worker 一本化で重複を抑え、#1211 で旧 event を撤去。
- **代替アーキテクチャ**: actor（mpsc コマンド）型 vs 共有状態（RwLock + Notify）型。本設計は per-worktree に worker タスク 1 本 + invalidate チャンネル + RwLock snapshot の折衷とする（実装が単純で version 直列化を自然に担保）。完全 actor 化は将来 M2/M3 の session/streaming read model と統合する際に再検討。

## 仮定

`requirements.md` A1〜A7 を前提とする。本設計で追加・具体化した仮定:

- **D1: supersede は「完了時破棄＋再 scan」**。git2 走査の途中中断はしない。version 逆行なし・最終 snapshot 最新反映という外部挙動で behavior を満たす。
- **D2: 通知は Tauri event `repository-snapshot-changed`（version 付き）を主とし、remote 向け `WsMessage::BranchListSync` は worker 確定時に従来同様 emit する**。既存 `file-change` / `git-status-changed` / `branch-list-sync` は本 Issue では互換のため残し、発火源を worker に一本化する（撤去は #1211）。
- **D3: 既存 command は管理済み worktree では snapshot cache を読む。未管理 path では watcher を作らず一時 scan の version 0 ready snapshot を返す**。frontend は管理済み worktree の event で再取得する。
- **D4: service は `Arc<RepositoryStateService>` として `app.manage` し、`AppState` からも参照する**。watcher の debouncer 所有は service（`WorktreeState`）へ移す。worker の spawn / sleep / blocking scan と path normalize は usecase port とし、具体 Tokio / filesystem 実装は gateway 側に置く。
- **D5: scanner は `RepositoryUsecase` / `CodeUsecase` へ委譲して 1 snapshot を構築し、呼び出しを 1 scan サイクルに束ねる**。status gateway に current repo_path の集約 scan を置き、1 回の `client::open` で得た同一 `Repository` handle から `statuses()` / `diff_tree_to_index` / `diff_index_to_workdir` / `Patch::from_diff` を導出する。branch cards は scan サイクル内 snapshot として生成し、current worktree dirty は集約 scan の status 結果を再利用する。別 linked worktree dirty は別 workdir 走査が物理的に必要なため統合しないが、worktree ごとに 1 回だけ算出する。
- **D6: `limited` は本 Issue では常に false 既定**（打ち切り表現の器だけ用意）。具体閾値は #1211 で確定（A6）。
- **D7: worktree の同一性キーは `WorktreePathNormalizer` port で正規化した絶対パス**。subscriber へ返す `worktree_path` は呼び出し元と一致する文字列を保つ。

## Open Questions

なし。
