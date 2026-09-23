# Design 04

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `feat/issues/1861` の `81ec380b`。Design 01〜03 の周の実装は未コミットの作業ツリーにあり、Spec 工程ではコードを変更していないため、この実装をこの周の開始状態とする。
- 直前の Design は `docs/specs/issues-1861/design-03.md`。
- この周までに解消・見送りとなった Thread はない。open Thread は2件で、2件とも `[FIX_POLICY]` が付き、`[REJECTED]`・`[DEFERRED]` はない。
- Requirements・Behavior はこの周で変更していない。R-001〜R-016 と B-001〜B-023 に欠番・重複がなく、対応表が全 Requirement ID を記載し各行に Behavior ID が対応していることを確認した。Assumptions / Open Questions は「なし」である。
- Design 03 の「変える部分」5件は開始状態で満たされている。`rescan_branches` は走査後に `requested_generation` を再照合し、失効していれば結果を公開せず再走査する（`src-tauri/src/usecase/repository_state/service.rs:107-121`）。`refresh_workspaces` の dispatch の行き先は `src-tauri/src/adaptor/controller/client/workspace_tree_shared_test.rs` が確認する。`useWorkspaceList` の `loading`・`pendingRef` は除かれている（`src/hooks/useWorkspaceList.ts:11-18`、`:120`）。`WorkspaceListUsecase` の構築は `wiring::build_workspace_list_usecase` に集約され、daemon と desktop の両入口がこれを呼ぶ（`src-tauri/src/adaptor/controller/wiring.rs:169-175`、`daemon.rs:257-267`、`client/workflow/mod.rs:1321-1330`）。登録 Repository 一覧の参照先は `refresh_workspaces` の snapshot へ一本化され、`useRepoList` は `addRepo`・`removeRepo`・`initFromCwd` だけになっている（`src/App.tsx:113-119`、`src/hooks/useRepoList.ts:10-30`）。
- Design 03 の「未確定・リスク」2件は開始状態で解消している。失効時は `rescan_branches` が結果を返さず再走査するため、失効が取得失敗として `WorkspaceListQueryService::branches` から返らない。restoration の完了判定は `repositoriesLoaded || repositoriesError` で、取得の成功と失敗のどちらでも確定する（`src/App.tsx:120-128`）。

## 変える部分

- 削除後の再読込を対象 Repository に閉じる: Worktree の削除および Branch の削除が成功した後の再読込で、対象 Repository 以外の Repository の走査を発生させない。開始状態では `WorkspaceList.tsx:1613-1631` の `handleDeleteConfirm` が親から受け取る `refresh` を呼び、`:1772` の `refresh={model.refresh}` がそれを与える。`useWorkspaceList.ts:47` の `refresh` は `worktreePath` なしの `request()` であり、`workspace_tree_shared.rs:25-27` の `None` 分岐から `list.rs:83-97` の全登録 Repository 走査へ入る。派生点 `81ec380b` では同じ処理が単一 `repoPath` の `useWorktreeList(repoPath).refresh` で対象 Repository に閉じていた。根拠: Thread 1bd38ae7-bd78-47bc-ab9d-2dcd8b110ece（blocking、`[FIX_POLICY]`）、`requirements.md` Scope / Non-goals「Repository の追加・削除、Worktree の作成・削除の操作」、R-014「正常な取得によって対象の削除が確認できた場合は、その結果を一覧へ反映する」、B-018、`AGENTS.md` レビュー観点「full-retention / full-recompute 経路を増やしていないか」。ルート: 委任
- 局所操作の通知を対象 Worktree の更新へ送る: Session の作成・削除・復元、Workflow 操作、Worktree メニュー・作成メニューの展開の後の再読込で、対象外の Repository の走査を発生させない。開始状態では `useWorkspaceList.ts:63-73` の `scheduleReload` が `worktreePath` を `timers` の key にしか使わず、期限時に引数なしの `refresh()` を呼ぶため、`:74-90` の `workspace-tree-refresh`・`workflow-execution-changed`・Agent Session 通知が渡す `worktreePath` が破棄され、`workspace_tree_shared.rs:25-27` の `None` 分岐から全体更新へ入る。根拠: Thread 1e36eb0c-e3a4-4098-ba93-56df061dfde8（blocking、`[FIX_POLICY]`）、R-005「手動更新と自動更新は、登録 Repository 一覧、各 Repository の Worktree 一覧、各 Worktree 配下の Session・Workflow の一覧情報を対象とする」、`AGENTS.md` レビュー観点「full-retention / full-recompute 経路を増やしていないか」。ルート: 委任

## 固定するルート

この周で新しく固定する実装上の指定はない。Design 01 で固定した次の4つを維持する。

- 全体の更新手順と結果の判定を Rust 側に置き、既存の `RepositoryStateService` の走査処理と Workspace の一覧取得処理を再利用する。Frontend は受け取った一覧・更新状態・エラーを表示するだけにする。（Design 01 固定ルート①）
- 一覧を保持する変更と、Session・Workflow の再取得経路の変更を併せて行う。（Design 01 固定ルート②）
- 更新対象は Workspaces の一覧情報に限り、表示中の Session や実行中の Workflow の継続を保つ。（Design 01 固定ルート③）
- 手動更新と自動更新で内部の経路を分けない。（Design 01 固定ルート④）

## 変えないもの

- Workspaces 行の手動更新と自動更新の対象範囲。削除後の再読込と局所操作後の再読込の範囲を狭めても、手動更新と自動更新は R-005 の3階層を対象とし続け、B-006・B-021 を満たす。理由: 人間が2件の Thread の受入条件にそう定めたため。
- 削除の一覧への反映。削除後の再読込を対象 Repository に閉じても、削除された Worktree・Branch は一覧から取り除かれ、R-014・B-018 を満たす。理由: 人間が Thread 1bd38ae7 の受入条件にそう定めたため。

## 未確定・リスク

- Repository 単位の再読込を表現する場合の generation の扱いが未確定である。`WorkspaceListRefresh::complete_branches` は `is_current(generation)`（`repositories` の generation との一致）を受理の条件にし、generation を進める操作は全体の `begin` と nodes だけを進める `begin_worktree` の2つしかない（`src-tauri/src/domain/workspace_tree/refresh.rs:67-148`）。branches の generation だけを進めると `complete_branches` が受理せず、全体の generation を進めると他 Repository の entry も失効して進行中の更新結果が捨てられる。どちらも R-013・B-017 の「より古い取得結果でより新しい一覧を上書きしない」と、R-008・B-009 の取得に成功した対象だけを更新する扱いに影響する。
- この周で自動判断した箇所はない。未決のまま残した要求はなく、`[DEFERRED]` で人間へ渡した件もない。
