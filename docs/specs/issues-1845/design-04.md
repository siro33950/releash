# Design 04

## 開始状態

- 直前の Design: `docs/specs/issues-1845/design-03.md`
- 差分の基準: base ブランチ `main`、派生点 `81ec380b feat(workflow): 終端事実で実行状態を確定する (#1836) (#1862)`。作業ブランチは `feat/issues/1845`。design-01 から design-03 までの変更は未コミットの working tree 変更として存在し、Spec 工程ではコードを変更していないため、この状態を今周の開始状態とする
- この周までに解消・見送りとなった Thread
  - `58f55ed4-4b2d-4681-acaa-96a0a467cab1`（同一 worktree の path 表記の一致。design-03 が「変える部分」に挙げた件）は resolved。開始状態の実装に取り込まれている
  - `aa89346e-74d5-4c46-ab90-466ca4821864`（隔離 worktree の除外判定の表記の一致）と `3ad8fd74-85fa-43e3-8e7c-9ea4e1a96678`（削除中の保持先の一本化と区間終端の一致）は resolved
  - `192f918b-dcbd-4ee4-a6dd-e52367d9ae5c`（`WorktreeDeletionGuard` の drop と lease 解放の順序）は `[REJECTED]` で resolve 済み
  - `[DEFERRED]` として人間へ渡した Thread は無い
- 今周で変える対象に関わる開始状態の要点
  - `src/hooks/useWorktreeList.ts:46-73` の `refresh` は、`list_branches_with_status_snapshot` の結果に対し 56 行目で `enrichWithPrStatus` を await し、その完了後の 61 行目で `setBranches` する。取得した snapshot を PR 情報より先に画面へ反映する経路は無い。この経路は初回取得、`branch-list-sync`、`branch-list-refresh`、polling のいずれの契機でも共通である
  - `enrichWithPrStatus`（同 17-44）は `get_cached_pr_status` を呼ぶ。`src-tauri/src/usecase/git_host/git_host_usecase.rs:29-36` は cache miss のとき `provider.fetch_pr_status` を同期実行し、GitHub gateway の待ち時間の上限は `src-tauri/src/adaptor/gateway/git_host/github.rs:16` の `GH_TIMEOUT`（10 秒）である
  - `src/hooks/useWorktreeList.ts:102-104` の polling 間隔は、既に `setBranches` 済みの `branches` に `is_deleting` を持つ行がある場合にだけ 1 秒になり、それ以外は 120 秒である
  - `src/components/workspace/WorkspaceList.tsx:1616-1637` の `handleDeleteConfirm` は、`remove_worktree` の受理後に削除ダイアログを閉じ、`refresh({ silent: true })` を開始する
  - 受理後の削除処理は `src-tauri/src/usecase/repository_usecase.rs:338-347` の `spawn_blocking` で走り、排他 Guard を保持したまま `worktree.remove` と `releash-base` の後始末を行う。削除中であることは `BranchCardDto.is_deleting` として snapshot に載る

## 変える部分

- 削除中の行の表示が PR 情報取得に従属する状態の解消: workspace 一覧の表示反映が PR 情報取得の完了を待つため、受理から削除処理が終わるまでの間に `is_deleting` が画面へ一度も現れない経路が成立する状態を解消する。根拠: Thread `ac7d58db-a24a-48c4-946b-2c8e92a0cd75`（`[FIX_POLICY]`）、R-005「削除を受理してからその削除処理が終わるまで、対象 worktree は削除中と分かる形で workspace 一覧に残る」/ B-006。受入条件は、削除の受理から削除処理が終わるまでの間、PR 情報取得の成否および所要時間に依存せず対象 worktree が削除中と分かる形で workspace 一覧に現れ、削除処理が終わるまで残ること。ルート: 委任

## 固定するルート

今周に対する新規の指定は無い。design-01 で固定した「削除の方法を変えない」（`src-tauri/src/adaptor/gateway/repository/worktree.rs` の `remove_worktree` の中身、すなわち `WorktreePruneOptions` に `valid` / `working_tree` と lock 時の `locked` を設定した `prune` の呼び出しと、残った場合の `std::fs::remove_dir_all` に手を入れないこと）は今周も維持する。

## 変えないもの

なし。

## 未確定・リスク

- 自動判断: 今周は Requirements・Behavior を修正していない。R-001 / R-002 / R-005 / R-006 と B-001 / B-002 / B-003 / B-006 / B-007 の対応に誤り・不足・矛盾は無い
- 未決のまま残した要求: なし
- `[DEFERRED]` で人間へ渡した件: なし
