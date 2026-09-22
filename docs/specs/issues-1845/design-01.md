# Design 01

## 開始状態

初回。既存の `design-NN.md` は無い。開始状態の実装は `requirements.md` の Current Behavior を参照する。

- 差分の基準: base ブランチ `main`、派生点 `81ec380b feat(workflow): 終端事実で実行状態を確定する (#1836) (#1862)`。作業ブランチは `feat/issues/1845`。未コミットの変更は `docs/specs/issues-1845/` の追加だけで、コードは開始状態のままである
- この周までに解消・見送りとなった Thread: 無し（初回であり open Thread も無い）
- Current Behavior の補足。workspace 一覧が実際に描画するのは `list_branches_with_status_snapshot` が返す `worktree_display_groups.working_areas`（`src-tauri/src/usecase/repository_dto.rs` の `BranchCardDto`）であり、Current Behavior が挙げる `WorktreeEntryDto`（`list_worktrees`）ではない。`WorktreeEntryDto` は `src/App.tsx:171` の別経路が使う。どちらにも削除中の項目が無い点は Current Behavior のとおりである

## 変える部分

- 受理と削除処理の分離: `remove_worktree` の呼び出しが、削除処理の完了ではなく受理で返るようにする。根拠: R-001「worktree の削除を確定すると、削除処理の完了を待たずに削除ダイアログが閉じ、利用者は画面の操作を続けられる」/ B-001。ルート: 委任
- 受理の境界の位置: 削除前の検証（`validate_removal`）と実行木の Archive（`archive_worktree`）を受理より前に置き、これらの失敗は受理せず呼び出し元へ返す。根拠: R-002「削除前の検証と実行木の Archive は、削除の受理までに完了する。これらが失敗した場合、対象 worktree は削除されず、失敗は削除ダイアログ上で分かる」/ B-002 / B-003。ルート: 委任
- 削除中の domain 表現: 削除中であることを外から問い合わせられるようにする。開始状態では `src-tauri/src/domain/repository/worktree_operation.rs` の `WorktreeOperationState` が `deleting` を持つだけで問い合わせ操作が無く、識別子と状態の対応は `src-tauri/src/usecase/worktree_operation.rs` の `WorktreeOperations` が持ち、`docs/glossary/DOMAIN.md` に該当する語が無い。根拠: R-005 / R-006。ルート: 委任
- 削除中の一覧表出: 受理から削除処理が終わるまで、対象 worktree を削除中と分かる形で workspace 一覧に出す。開始状態では一覧を供給する read model に削除中の項目が無い。根拠: R-005「削除を受理してからその削除処理が終わるまで、対象 worktree は削除中と分かる形で workspace 一覧に残る」/ B-006。ルート: 委任
- 排他の寿命: 受理後の削除処理が終わるまで排他を保つ。開始状態では `WorktreeDeletionGuard` は `src-tauri/src/usecase/repository_usecase.rs:300` の `remove_worktree` の関数スコープで `_deletion` として保持され、関数から戻ると drop される。根拠: R-006「削除を受理してからその削除処理が終わるまで、対象 worktree に対する外部からの状態変更要求は受理されず、読み取りは引き続き行える」/ B-007。ルート: 委任

## 固定するルート

削除の方法を変えない。粒度は `src-tauri/src/adaptor/gateway/repository/worktree.rs:322` の `remove_worktree` の中身、すなわち `WorktreePruneOptions` に `valid` / `working_tree` と lock 時の `locked` を設定した `prune` の呼び出しと、残った場合の `std::fs::remove_dir_all` に手を入れないことである。理由は Request の指定「消し方は、今のままにする」。

## 変えないもの

なし。

## 未確定・リスク

- 削除中の識別子と一覧の行の対応。開始状態では `WorktreeOperations` のキーは `src-tauri/src/usecase/workflow/execution_archive.rs:198` が渡す `execution_archives.worktree_identity(path)` と `workspace-state:<name>` であり、workspace 一覧の行が持つ `BranchCardDto.worktree_path` と同じ文字列になるかは確認していない。一致しない場合、削除中を一覧の対象行へ結び付けられず R-005 / B-006 を満たせない
- 自動判断（Requirements・Behavior は修正していない）: Current Behavior が一覧の read model を `WorktreeEntryDto` と記す点は、workspace 一覧が実際に使う read model と一致しない。R-005 の内容は変わらないため Requirements は修正せず、正確な経路を「開始状態」に記した
- 未決のまま残した要求: なし
- `[DEFERRED]` で人間へ渡した件: なし
