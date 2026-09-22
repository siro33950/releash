# Context

- 正本: [#1845 \[workspace\] worktree の削除で、画面を待たせない](https://github.com/siro33950/releash/issues/1845)
- 所属マイルストーン: [01. Workflow 操作と状態の簡素化](https://github.com/siro33950/releash/milestone/90)。同マイルストーンの確定判断に「worktree の削除は、画面を待たせずに裏で行う。消し方は今のまま」がある
- 先行条件の [#1826](https://github.com/siro33950/releash/issues/1826) は `482dbdda feat(workflow): 実行木単位のArchiveを導入 (#1826)` で現行 `main` に取り込まれている。`docs/specs/issues-1826/requirements.md` の R-009 は「worktree に属する実行木を理由 `worktree_removed` で Git worktree とフォルダの削除より先に Archive し、実行中の Command プロセスをフォルダ削除より先に停止する」ことを要求する
- 同 R-012 は「Releash による worktree の削除が始まった後は、対象 worktree に対する外部からの状態変更要求は受理されず、読み取りは引き続き行える」ことを要求する
- #1826 の Context は「#1845 は worktree フォルダ削除を画面の待機対象から外す変更を扱う。ただし、削除前の実行木 Archive は本 Issue が先に同期的に完了させる前提である」と記しており、Archive を同期に保つ前提は #1826 側で確定している
- 削除の実体は libgit2 の `prune` 1 回の呼び出しであり、Releash はその途中に介入できない。Git 管理情報の削除とディレクトリ実体の削除は分けられない
- 現行の削除経路は次の 3 箇所で構成される。正本 Issue の「参照」に書かれた行番号は現在のコードと一致しないため、名前で特定した現在位置を記す
  - 画面: `src/components/workspace/WorkspaceList.tsx:1608` の `handleDeleteConfirm`、および `src/components/workspace/DeleteWorktreeDialog.tsx`
  - usecase: `src-tauri/src/usecase/repository_usecase.rs:300` の `remove_worktree`
  - gateway: `src-tauri/src/adaptor/gateway/repository/worktree.rs:322` の `remove_worktree`（prune と `remove_dir_all` は 328-340）
- worktree 削除の入口は Tauri command と loopback HTTP local API の双方から共有される `remove_worktree_shared`（`src-tauri/src/adaptor/controller/client/repository/worktree.rs:61`）である。`releash` CLI に worktree 削除のサブコマンドは無い
- 削除開始後の排他は `src-tauri/src/usecase/worktree_operation.rs` の `WorktreeDeletionGuard` が持ち、状態遷移の規則は `src-tauri/src/domain/repository/worktree_operation.rs` の `WorktreeOperationState` が所有する。削除中かを外から問い合わせる操作は無く、read model にも載っていない

# Outcome

対象者は、Releash で worktree を作って作業し、不要になった worktree を削除する利用者である。

現在は、削除を確定すると、削除が終わるまで削除ダイアログが開いたままになり、その間ダイアログを閉じることも他の操作を行うこともできない。worktree が大きいほど待ち時間が長い。

変更後は、削除を確定した時点でダイアログが閉じ、利用者は待たずに次の操作へ移れる。削除処理が終わるまで、対象 worktree は削除中と分かる形で一覧に残る。削除より前に行う実行木の Archive は、これまでどおり先に完了する。

# Current Behavior

## 画面が削除の完了を待つ

`src/components/workspace/WorkspaceList.tsx:1608` の `handleDeleteConfirm` は `invoke("remove_worktree")` を `await` し、その完了後に一覧の `refresh()` を `await` してから `setDeletingBranch(null)` でダイアログを閉じる。

`src/components/workspace/DeleteWorktreeDialog.tsx` は、この `onConfirm` の完了までを `deleting` 状態として扱い、その間は Cancel と Delete を disabled にし、Escape キーと外側クリックによる閉じる操作を無視し、ボタンを spinner 付きの "Deleting..." にする。失敗した場合は、ダイアログ内に `role="alert"` の赤字でメッセージを表示する。

最小の再現手順は、Workspace tree の worktree から Delete を選び、確認ダイアログで Delete を押すことである。削除が終わるまでダイアログは開いたままで、他の操作へ移れない。

## 削除の手順は 1 回の呼び出しで同期に完了する

`src-tauri/src/usecase/repository_usecase.rs:300` の `remove_worktree` は、1 回の呼び出しの中で次を順に行う。

1. `validate_removal`（未コミット変更数と force、lock の検査）
2. `begin_worktree_deletion`（`WorktreeDeletionGuard` の取得。以後この guard が drop されるまで対象 worktree への状態変更を拒む）
3. `validate_removal` の再実行
4. `archive_worktree`（#1826 による実行木の Archive）
5. `kill_by_worktree`（対象 worktree の terminal surface の停止）
6. `worktree.remove`（gateway の削除）
7. 返ってきたブランチ名に対する `releash-base` config の後始末（best-effort）

`src-tauri/src/adaptor/gateway/repository/worktree.rs:322` の `remove_worktree` は、`WorktreePruneOptions` に `valid(true)` と `working_tree(true)`、lock されていれば `locked(true)` を設定して `prune` を呼び、その後もフォルダが残っていれば `std::fs::remove_dir_all` で消す。

どの段階の失敗も `Result` として呼び出し元へ返り、最終的にダイアログ内へ表示される。`remove_worktree` の処理を別タスクへ逃がす経路は存在しない。

## 削除中は一覧の表示に現れない

削除中かどうかは `src-tauri/src/domain/repository/worktree_operation.rs` の `WorktreeOperationState` が `deleting` として持つが、外から問い合わせる操作は無い。一覧の read model（`src-tauri/src/usecase/repository_dto.rs:93` の `WorktreeEntryDto`）にも項目が無く、画面は削除中を表示しない。

## 待ち時間の実測

正本 Issue の 2026-09-20 の実測（`no9-monorepo.worktrees/feature/PJT-2312`）では、フォルダは 17GB、ファイルは 305,299 個で、そのうち Git が管理しているのは 6,916 個（約 2%）だった。全ファイルを 1 回たどるだけで約 9 秒かかる。削除自体の所要時間は測られていない。

# Scope / Non-goals

## 変更する対象

- worktree の削除を確定してから、削除ダイアログが閉じて画面の操作を続けられるようになるまでの待機
- 削除の受理から削除処理が終わるまでの間、workspace 一覧が対象 worktree をどう扱うか
- 削除の受理から削除処理が終わるまでの間、対象 worktree への状態変更を拒む範囲の維持

## 変更しない対象

- 削除の方法。libgit2 の `prune`（`working_tree(true)`）と、残った場合の `remove_dir_all` のままとする
- 受理後の削除処理の結果。成否を Releash は追跡せず、保持せず、利用者へ通知しない
- 受理した削除処理が終わる前にアプリが終了した場合の、残った worktree フォルダの扱い
- 削除前に行う実行木の Archive（#1826 R-009）を先に完了させること、およびその順序
- 削除前の検証（未コミット変更、force、lock）の内容と、その失敗を削除ダイアログ上に表示すること
- worktree を持たないブランチの削除（`delete_branch`）
- GC による、Git の worktree 一覧から消えた worktree の実行木の Archive（#1826 R-007、R-008）
- 削除の入口の構成（Tauri command と local API が同じ処理を共有すること）

# Requirements

- R-001: worktree の削除を確定すると、削除処理の完了を待たずに削除ダイアログが閉じ、利用者は画面の操作を続けられる
- R-002: 削除前の検証と実行木の Archive は、削除の受理までに完了する。これらが失敗した場合、対象 worktree は削除されず、失敗は削除ダイアログ上で分かる
- R-005: 削除を受理してからその削除処理が終わるまで、対象 worktree は削除中と分かる形で workspace 一覧に残る
- R-006: 削除を受理してからその削除処理が終わるまで、対象 worktree に対する外部からの状態変更要求は受理されず、読み取りは引き続き行える

# Assumptions / Open Questions

なし。
