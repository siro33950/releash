# Design 03

## 開始状態

- 直前の Design: `docs/specs/issues-1845/design-02.md`
- 差分の基準: base ブランチ `main`、派生点 `81ec380b feat(workflow): 終端事実で実行状態を確定する (#1836) (#1862)`。作業ブランチは `feat/issues/1845`。design-01 / design-02 の変更は未コミットの working tree 変更として存在し、Spec 工程ではコードを変更していないため、この状態を今周の開始状態とする
- この周までに解消・見送りとなった Thread
  - `192f918b-dcbd-4ee4-a6dd-e52367d9ae5c`（`WorktreeDeletionGuard` の drop がプロセス間 lease を deletion 状態より先に解放する）は `[REJECTED]` で resolve 済み。guard の drop は背景タスクが `worktree.remove` と `releash-base` の後始末を終えた後にのみ起きるため、R-006 / B-007 が定める区間は drop 開始時点で既に終了している
  - design-02 が「変える部分」に挙げた `aa89346e-74d5-4c46-ab90-466ca4821864`（隔離 worktree の除外判定の表記の一致）と `3ad8fd74-85fa-43e3-8e7c-9ea4e1a96678`（削除中の保持先の一本化と区間終端の一致）は、いずれも開始状態の実装に取り込まれている
  - `[DEFERRED]` として人間へ渡した Thread は無い
- 今周で変える対象に関わる開始状態の要点
  - `src-tauri/src/adaptor/gateway/repository/state.rs:51-67` の `include_deleting_worktrees` は、usecase へ渡す前に全 `BranchCardDto.worktree_path` へ `worktree_operation::worktree_identity`（filesystem canonicalize）を適用する。この read model が workspace 一覧の行になる
  - `src-tauri/src/usecase/repository_usecase.rs:238-261` の `list_worktrees` は `WorktreeEntryDto.path` に `to_canonical_forward_slash`（区切り文字の変換のみ）を適用する。canonicalize は行わない
  - `src/App.tsx:135-139` の `selectedRootPath` は、開いている worktree tab の `rootPath` である。tab は `src/hooks/useWorkspaceNavigation.ts:26-45` の `openWorktreeTab` が受け取った `rootPath` をそのまま `id` と `rootPath` にする。`openWorktreeTab` へ渡る値は経路ごとに異なり、`src/App.tsx:172-183` は `list_worktrees` の `WorktreeEntryDto.path`、`src/App.tsx:202-210` の `handleSelectWorktree` は workspace 一覧の行が持つ `branch.worktree_path` を渡す
  - `src/components/workspace/WorkspaceList.tsx:640-641` は `branch.worktree_path === selectedRootPath` の文字列完全一致で選択を判定する
  - 削除中の行は `BranchCardDto.is_deleting` で表される。`src-tauri/src/usecase/repository_query_service.rs:79-110` の `include_deleting_worktrees` が `WorktreeOperations` の deletion target を走査し、`repository_root` の一致と、`target.path` と `card.worktree_path` の一致またはブランチ名の一致で対象行を決める。`WorktreeDeletionTarget.path` は `validate_removal`（`src-tauri/src/adaptor/gateway/repository/worktree.rs:272-320` の `removal_target` が `canonicalize` した path）に由来する
  - 隔離 worktree の除外判定は `src-tauri/src/usecase/repository_query_service.rs:41` の `classify_branch_cards` → `src-tauri/src/domain/workflow/value_objects/worktree_origin.rs:104` の `matches_isolated_identity_rule` にあり、repository root と `worktree_path` の文字列一致に依存する

## 変える部分

- 同一 worktree の path 表記の一致: workspace 一覧の行が持つ path と、選択状態の起点になる read model の path が、同じ worktree について別表記になる状態を解消する。開始状態では前者だけが filesystem canonicalize され、後者は区切り文字の変換だけであるため、symlink 等の別表記で到達した repository では両者が一致せず、一覧上の選択表示と選択 worktree 向けの処理が働かない。根拠: Thread `58f55ed4-4b2d-4681-acaa-96a0a467cab1`（`[FIX_POLICY]`）。受入条件は、同一 worktree について一覧の行の path と選択状態の起点になる read model の path が同じ表記で得られ、symlink 等の別表記で到達した repository でも一覧上の選択表示と選択 worktree 向けの処理が働き、かつ R-005 / B-006 の削除中の行の表示と隔離 worktree の除外判定が維持されること。ルート: 委任

## 固定するルート

今周に対する新規の指定は無い。design-01 で固定した「削除の方法を変えない」（`src-tauri/src/adaptor/gateway/repository/worktree.rs:322` の `remove_worktree` の中身、すなわち `WorktreePruneOptions` に `valid` / `working_tree` と lock 時の `locked` を設定した `prune` の呼び出しと、残った場合の `std::fs::remove_dir_all` に手を入れないこと）は今周も維持する。

## 変えないもの

なし。

## 未確定・リスク

- 削除中の行の判定に使う repository root の表記。`src-tauri/src/adaptor/gateway/repository/state.rs:43-49` の `main_repo_path` は `worktree_identity` で canonicalize した値を返し、これが `include_deleting_worktrees` の `repository_root` 引数になる。一方 `WorktreeDeletionTarget.repository_root` は `src-tauri/src/usecase/repository_usecase.rs:319` が `worktree.main_repo_path` から得た値で、`normalize_repo_path` を通るだけであり、git2 の `workdir()` が canonical な表記を返すかは確認していない。両者が一致しない場合、削除中の行が一覧に現れず R-005 / B-006 を満たせない。今周の変更で一覧側の path 表記を揃えるときに、この突き合わせの片辺だけを変えると同じずれが残る
- 自動判断: 今周は Requirements・Behavior を修正していない。R-001 / R-002 / R-005 / R-006 と B-001 / B-002 / B-003 / B-006 / B-007 の対応に誤り・不足・矛盾は無い
- 未決のまま残した要求: なし
- `[DEFERRED]` で人間へ渡した件: なし
