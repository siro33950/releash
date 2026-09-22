# Design 02

## 開始状態

- 直前の Design: `docs/specs/issues-1845/design-01.md`
- 差分の基準: base ブランチ `main`、派生点 `81ec380b feat(workflow): 終端事実で実行状態を確定する (#1836) (#1862)`。作業ブランチは `feat/issues/1845`。design-01 の変更は未コミットの working tree 変更として存在し、Spec 工程ではコードを変更していないため、この状態を今周の開始状態とする
- この周までに解消・見送りとなった Thread: 無し。`[DEFERRED]` として人間へ渡した Thread、`[REJECTED]` として resolve した Thread はいずれも無い
- 今周で変える対象に関わる開始状態の要点
  - `src-tauri/src/adaptor/gateway/repository/state.rs:43-48` の `main_repo_path` は `get_main_repo_path` の結果へ `worktree_operation::worktree_identity`（filesystem canonicalize）を適用して返す。一方、この値と突き合わせる `BranchCardDto.worktree_path` は `src-tauri/src/adaptor/gateway/repository/branch_card.rs:23-24,100-126` で `normalize_repo_path`（区切り文字と末尾のみ）を適用した `repo.workdir()` / `wt.path()` 表記のままである。隔離 worktree の除外判定（`src-tauri/src/usecase/repository_query_service.rs:44-52` の `classify_branch_cards` → `src-tauri/src/domain/workflow/value_objects/worktree_origin.rs:104-114` の `matches_isolated_identity_rule`）は、この 2 つの文字列完全一致に依存する
  - 削除中であることは `src-tauri/src/usecase/repository_query_service.rs:66-142` の `RepositoryQueryService` が `deleting_worktrees`（path をキーとする Map と `Weak<WorktreeDeletionGuard>`）として保持し、`src-tauri/src/usecase/repository_usecase.rs:322-344` の背景タスクが `worktree.remove` と `releash-base` の後始末の後、`_deletion`（排他 Guard）が drop される前に `forget_deleting_worktree` を呼ぶ。排他の正本は `src-tauri/src/domain/repository/worktree_operation.rs` の `WorktreeOperationState.deleting` と、それを識別子ごとに保持する `src-tauri/src/usecase/worktree_operation.rs` の `WorktreeOperations` である
  - `WorktreeOperations` のキーは `src-tauri/src/usecase/workflow/execution_archive.rs:197-210` が渡す `execution_archives.worktree_identity(path)`（canonicalize 済み）と `workspace-state:<name>` である

## 変える部分

- 隔離 worktree の除外判定における表記の一致: 除外判定の片辺（repository root）だけが canonicalize され、もう片辺（`BranchCardDto.worktree_path`）が非 canonical のままであるため、symlink 等の別表記で到達した repository 配下の隔離 worktree が workspace 一覧へ露出する状態を解消する。根拠: Thread `aa89346e-74d5-4c46-ab90-466ca4821864`（`[FIX_POLICY]`）。受入条件は、repository へ到達した表記にかかわらず隔離 worktree が workspace 一覧から除外され続け、同時に R-005 / B-006 の削除中の行の表示が維持されること。ルート: 委任
- 削除中の保持先の一本化と区間終端の一致: 削除中であることの保持先が排他の正本（`WorktreeOperationState.deleting` / `WorktreeOperations`）と `RepositoryQueryService.deleting_worktrees` に二重化しており、一覧から削除中の行が消える時点が排他の解放より前にずれる状態を解消する。根拠: Thread `3ad8fd74-85fa-43e3-8e7c-9ea4e1a96678`（`[FIX_POLICY]`）。R-005「削除を受理してからその削除処理が終わるまで、対象 worktree は削除中と分かる形で workspace 一覧に残る」と R-006「削除を受理してからその削除処理が終わるまで、対象 worktree に対する外部からの状態変更要求は受理されず、読み取りは引き続き行える」は同一の区間を指す。受入条件は、受理から削除処理が終わるまでの任意の時点で一覧を取得すると対象 worktree が削除中として現れ、一覧から消える時点と排他が解かれる時点が一致し、削除中であることの保持先が排他の正本と二重化していないこと。B-007 の状態変更拒否と読み取り継続は維持される。ルート: 委任

## 固定するルート

今周に対する新規の指定は無い。design-01 で固定した「削除の方法を変えない」（`src-tauri/src/adaptor/gateway/repository/worktree.rs:322` の `remove_worktree` の中身、すなわち `WorktreePruneOptions` に `valid` / `working_tree` と lock 時の `locked` を設定した `prune` の呼び出しと、残った場合の `std::fs::remove_dir_all` に手を入れないこと）は今周も維持する。

## 変えないもの

なし。

## 未確定・リスク

- 削除中の識別子と一覧の行の対応。design-01 の「未確定・リスク」に挙げた論点は今周も残る。開始状態では `RepositoryQueryService.deleting_worktrees` のキーが canonicalize 済みの worktree path で、一覧の行が持つ `BranchCardDto.worktree_path` は非 canonical 表記であり、path が一致しない場合はブランチ名での突き合わせに依存している。今周の 2 件目の変更で削除中の保持先を排他の正本へ寄せると、その正本のキー（`execution_archives.worktree_identity(path)` と `workspace-state:<name>`）と一覧の行の表記を突き合わせる必要が同じ形で生じる。1 件目の変更（除外判定の表記の一致）と同じ原因であり、両者を個別に揃えると再びずれる余地が残る
- 自動判断: 今周は Requirements・Behavior を修正していない。R-001 / R-002 / R-005 / R-006 と B-001 / B-002 / B-003 / B-006 / B-007 の対応に誤り・不足・矛盾は無い
- 未決のまま残した要求: なし
- `[DEFERRED]` で人間へ渡した件: なし
