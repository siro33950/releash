# Requirements

## Type

新機能 / リファクタリング。性能・メモリ効率改善（マイルストーン M1「Git / Diff hot path を Rust read model に寄せる」）の基盤として、`RepositoryStateService` を導入し、worktree ごとに分散している watcher / git scan / read model 生成を 1 系統に集約する。

関連: #1210（本 Issue）/ #1209（M0 計測, 先行）/ #1211 / #1212 / #767 / #866 / #894 / `docs/releash-performance-architecture-audit.md` M1（正本ドキュメント, commit `b0c5e4c2`）

## 背景と目的

現状、Git の read model 生成が component / hook ごとに分散している。

- `useGitStatus` が `start_watching` で file watcher を起動し、status を取得する。
- `useDiffFileTree` が status 後に `get_status_diff_stats` と `build_diff_file_tree` を別 command で呼ぶ。
- `useGitEventRefresh` は caller ごとに `start_watching` を呼び、ReviewPanel と useGitStatus などで watcher が重複する。
- `watcher.rs` の Git dir watcher は notify callback 内で `list_branches_with_status`（dirty count など重い read model 生成）を直接呼ぶ。

結果として、同一 worktree に対して watcher と git scan が複数走り、status / diff stats / branch cards / worktree dirty count / diff tree がそれぞれ別経路で git2 を起動して同じ走査を重複生成する。`docs/releash-performance-architecture-audit.md` の結論にあるとおり、問題は個別機能の遅さではなく「全量を読む / 全量を再計算する」設計の広がりにある。

本 Issue では、1 repo / worktree につき watcher と git scan を 1 系統へ集約する `RepositoryStateService` を Rust 側に追加し、status / diff stats / branch cards / worktree dirty count / diff tree を同一 cache / versioned snapshot から返す土台を作る。これは後続の #1211（ReviewSnapshot / ReviewFileView command）と #1212（hunk operation の id 化）が乗る基盤であり、M1 の最初に着手する位置づけ（Issue コメント「順序 4-1」）。

## スコープ

### RepositoryStateService の導入

- worktree（repo）ごとに 1 つの service instance が watcher と git scan を所有する。
- recursive file watcher と Git dir watcher を service へ集約し、worktree あたり 1 系統に統合する。
- watcher の notify callback では重い read model 生成（`list_branches_with_status` 等）を直接行わず、invalidate（再走査要求）だけを発行する。実際の snapshot 生成は debounce 後に service の worker が行う。
- 同一 worktree に対して、複数の subscriber（ReviewPanel / useGitStatus / diff tree など）が個別に watcher を起動しない。subscriber は service の snapshot / version 通知を購読する。

### 集約する read model（同一 snapshot から提供）

debounce 後に生成する versioned snapshot から、以下を同一 cache / snapshot 経由で提供する。

- Git status（review に必要な file 集合）
- diff stats（status と同一走査由来）
- branch cards（`list_branches_with_status` 相当）
- worktree dirty count
- diff file tree（`build_diff_file_tree` 相当の tree 化）

これらが、現状のように個別 command / 個別走査で都度生成されるのではなく、1 scan サイクル（debounce 後 worker の read model 生成サイクル）で作った versioned snapshot から導出される状態にする。ここでの「集約 scan」は「全 worktree を物理的に 1 回の git2 走査へ統合する」意味ではない。current repo_path は 1 回の `client::open` で得た同一 `Repository` handle から `statuses()` / `diff_tree_to_index` / `diff_index_to_workdir` / `Patch::from_diff` を導出し、別 linked worktree の dirty count は別 workdir 走査が必要なため worktree ごとに 1 回だけ scan サイクル内で算出する。

### versioned snapshot とフラグ

- snapshot は version（単調増加するシーケンス）を持ち、subscriber は version で新旧を判定できる。
- snapshot は次のフラグを含む:
  - `stale`: 現在の snapshot が最新の変更を反映していない（background refresh 中）。
  - `loading`: 初回 scan / 再 scan が進行中。
  - `limited`: threshold（後述）により内容が省略・打ち切りされている。
- 中規模 repo で scan に時間がかかる場合、まず既存（stale）snapshot を返しつつ、background で refresh して新しい version を通知できる。

### scan の cancel / supersede

- scan 実行中に次の変更（invalidate）が来たら、進行中の古い scan を cancel / supersede し、最新状態に対する scan を優先する。
- 古い scan の結果で新しい snapshot を上書きしない（version の逆行を防ぐ）。

### ignored files の扱い

- default の snapshot では ignored files を返さない（現状 `status.rs` の `include_ignored(true)` による無駄な転送・CPU を削減する）。
- ignored files が必要な UI のみ opt-in で取得できる経路を残す。

### 既存 watcher 駆動フローの置き換え

- `useGitEventRefresh` / `useGitStatus` / `useDiffFileTree` 等が個別に `start_watching` する経路を、service が提供する単一 watcher + snapshot 通知へ寄せる。
- `watcher.rs` の Git dir callback で行っている重い同期処理を、service の invalidate → debounce → background worker 経路へ移す。

## 非スコープ

- **`get_review_snapshot(worktree_path, base)` / `get_review_file_view(...)` という frontend 向け command surface の確定と、`useFileDiffContent` / `useImageDiff` の direct FS read 削除、image の blob ref / temp URL 化**。これは #1211 の範囲とする。本 Issue は service / cache / 単一 watcher / versioned snapshot という基盤と、既存の status / diff stats / branch / dirty count / diff tree 経路の集約に限定する（仮定 A1 参照。境界は Open Questions Q1）。
- **hunk operation の id 化（`stage_hunk_by_id` / `unstage_hunk_by_id`）と frontend patch 再生成の削除**。#1212 の範囲。
- **review file view（file open 時の hunk groups / line window / large-file fallback / tokenization status）**。#1211 の範囲。
- **session / streaming / terminal / runtime lifecycle の read model**（M2 / M3）。
- 計測・性能予算そのものの追加（#1209 / M0 で実施済み・別管理）。本 Issue は #1209 で入れた計測で改善効果を確認できる前提とする。

## 要求事項

1. worktree（repo）ごとに watcher と git scan が 1 系統に集約され、同一 worktree に対して watcher が重複起動しない。
2. status / diff stats / branch cards / worktree dirty count / diff file tree が、同一の scan サイクルに由来する versioned snapshot から提供される（個別 command がそれぞれ独立に git2 走査を起動しない）。current repo_path の status / diff stats / current dirty は 1 回の `client::open` で得た同一 handle から導出し、別 linked worktree dirty は worktree ごとに 1 回だけ同一 scan サイクル内で算出する。
3. snapshot は version と `stale` / `loading` / `limited` フラグを持ち、subscriber が新旧と状態を判定できる。
4. 中規模 repo で scan が長い場合、stale snapshot を即時に返しつつ background で refresh し、完了時に新しい version を通知できる。
5. scan 中に次の変更が来たら、古い scan を cancel / supersede し、古い結果で新しい snapshot を上書きしない。
6. ignored files は default snapshot で返さず、必要な UI だけが opt-in で取得できる。
7. 全ロジックは Rust（Tauri バックエンド）に実装する（プロジェクト方針 `rust-first-logic`）。frontend は service の snapshot / version 通知を購読して表示するだけで、Git orchestration（走査トリガ・tree 化・基準選択）を持たない。
8. watcher の notify callback 内で重い read model 生成（branch list / dirty count 等）を同期実行せず、invalidate → debounce → background worker の経路に分離する。
9. 既存の status / diff stats / branch / worktree dirty count / diff tree の表示挙動（ユーザーから見える結果）に、ignored 既定除外を除いて回帰がない。

## 受け入れ基準の概要

- ReviewPanel / useGitStatus / diff tree などが個別に watcher（`start_watching`）を開始せず、単一 watcher 由来の通知で更新される。
- snapshot に version / stale / loading / limited フラグが含まれ、subscriber が参照できる。
- 中規模 repo で、初回または変更後に stale snapshot を返しつつ background refresh が走り、完了後に最新 snapshot へ更新される。
- scan 実行中に追加変更が発生した場合、古い scan が打ち切られ、最終 snapshot が最新状態を反映する（version が逆行しない）。
- default snapshot に ignored files が含まれず、opt-in した経路でのみ取得できる。
- status / diff stats / branch cards / worktree dirty count / diff tree が同一 snapshot から導出され、current repo_path の status / diff stats / current dirty の二重 open や、同一 linked worktree dirty の重複走査が起きない。
- 既存の表示結果（status / diff stats / branch / diff tree）に回帰がない（ignored 既定除外を除く）。
- 新規ロジックに対する Rust 単体テスト（正常系・cancel/supersede・stale/version 遷移・ignored opt-in）がある。

## 仮定

以下は Issue とリポジトリ現状から置いた仮定。誤りがあれば指摘で修正する。

- **A1: 本 Issue と #1211 の境界（確定）**。本 Issue は `RepositoryStateService`（単一 watcher + 集約 scan + versioned snapshot cache + cancel/supersede + ignored opt-in）と、既存の status / diff stats / branch / dirty count / diff tree 経路の集約までを担う。本 Issue では新 command 名 `get_review_snapshot` を導入せず、既存 command を裏で単一 service 経由に寄せて watcher 重複を解消する。frontend 向けの `get_review_snapshot` / `get_review_file_view` command 名の確定と frontend 呼び出し置換・direct FS read 削除・image blob ref 化は #1211 とする（旧 Open Question Q1 の確定結果＝案 A）。
- **A2: Spec ディレクトリ名**は `docs/specs/issues-1210` とする（先行 M0 Issue #1209 が `docs/specs/issues-1209` を用いた命名慣行に合わせる）。
- **A3: 既存クレートの活用**。watcher は既存の `notify_debouncer_mini`、Git 操作は `git2`、非同期は `tokio`、共有状態は `parking_lot` / `Arc` を再利用し、新規の重い依存は追加しない。
- **A4: service の所有場所**。`RepositoryStateService` は usecase 層に state service として置き、`watcher.rs` / `AppState` から利用する（既存 clean architecture 移行方針 M4 と整合）。具体配置は design.md で確定する。
- **A5: subscriber 通知方式**。snapshot 更新は既存の Tauri event（`file-change` 相当の event emit）/ ws bridge と同様の event 通知で subscriber へ届ける。版管理のため event に version を含める。具体方式は design.md で確定する。
- **A6: threshold（`limited` 条件）**。large file / many files / rename detection / untracked content 等の threshold 値は本 Issue では「`limited` フラグで打ち切りを表現できること」を要求し、具体閾値は #1211 の review file view と合わせて design.md / 後続で確定する。
- **A7: debounce**。既存 `useGitEventRefresh` の 300ms debounce 相当を service 側へ移し、frontend 側の重複 debounce を不要にする。具体値は既存挙動を踏襲する。

## Open Questions

なし（Q1「本 Issue で `get_review_snapshot` command を導入するか」は確定: 案 A。本 Issue は service + 単一 watcher + 内部 snapshot/通知までとし、新 command 名 `get_review_snapshot` の導入と frontend 呼び出し置換は #1211 で行う。スコープ・非スコープ・A1 に反映済み）。
