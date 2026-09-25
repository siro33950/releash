# Design 03

## 開始状態

- 差分の基準は base branch `main`、派生点は branch `feat/issues/1893` の HEAD `9194bd57`。開始状態の実装は、`9194bd57` に 1 周目と 2 周目の未コミット変更を加えたものである。
- 直前の Design は `docs/specs/issues-1893/design-02.md`。Design 01 の「変える部分」14 項目と Design 02 の「変える部分」4 項目は実装済みであり、本文では再掲しない。Design 02 の 4 項目は開始状態の実装で確認した（`each_worktree` が `git_operation::optional` で停止を返す、`isolated_worktree_error` が `RepositoryError::Stopped` を `WorkflowError::Stopped` へ写す、`notion/service_impl_test.rs` に期限切れの 2 件、`repository/worktree_operation_test.rs` に `registry` と `active` 待ちの期限・取消の 1 件）。
- この周までに解消・見送りとなった Thread は無い。resolve 済みの Thread は 0 件、`[DEFERRED]` とした Thread は 0 件、`[REJECTED]` とした Thread は 0 件である。今周の入力は `[FIX_POLICY]` が付いた open Thread 7 件（`d12fb304` / `1be6249c` / `28293c1e` / `5d9efb13` / `fce310ab` / `d4547481` / `93a50338`）で、Requirements・Behavior の変更は無い。

## 変える部分

- git2 の停止を通常値へ変換する call site を直す: `git_config.rs` の `get_branch_base`（`repo.config()` に `.ok()`）・`get_releash_base`（`.ok().and_then(... .ok())`）・`prune_stale_branch_bases`（`if let Ok(mut gc_cfg)`、削除ループの `let _ =`）、`code/file_content.rs` の `discover_repo`（`Err` を記録して親ディレクトリの discover を続行）、`repository/status.rs` の `count_patch_lines`（`_ => return (0, 0)`、`Err(_) => continue`、`if let Ok(line)`）、`repository/worktree.rs` の `Repository::open(wt.path())` に付いた `.ok()`、`usecase/repository_usecase.rs` の `list_worktrees`（`dirty_count` の `.unwrap_or(0)`、`get_branch_base` の `.unwrap_or(None)`）が、`GitOperationError::Stopped` / `RepositoryError::Stopped` を未設定・既定値・欠損・continue へ変換して後続の git2 操作へ進む。停止した時点で次の操作へ進まず、分類を保って呼び出し元へ返す形にする。根拠: Thread `d12fb304-0a34-4636-9bfa-dd257879d044`（R-002「いずれも次の操作へ進まない」、B-009、R-003）。ルート: 委任（既存の `git_operation::optional` を使うかを含む）。
- managed worktree の解決が停止で次の repository へ進まないようにする: `canonicalize_managed_worktree_path_inner` の `let Ok(worktrees) = usecase.list_worktrees(&repo_path_str) else { continue; };` が `RepositoryError::Stopped` を含む全エラーを捨てて次の repository を試し、ループを抜けた `Err("worktree_path is not a configured git worktree")` が `ManagedWorktreeGateway::resolve` の `.map_err(WorkflowError::external)` で `FailureKind::Internal` になる。停止では探索を止め、分類を保ったまま `WorkflowError` へ返す形にする。根拠: Thread `1be6249c-44ee-4e64-8ee7-e207e0805136`（R-002、B-009、R-003、Design 01 の固定するルート 8）。ルート: 委任。
- `query_error` が store の取り消しの分類を保つようにする: `LocalEventQueryError::Stopped` が `workspace_tree/query_service.rs` の包括節 `error => WorkflowError::external(error.to_string())` に落ち、`domain/workflow/error.rs` の `Self::External(_) => F::Internal` で分類が失われる。取り消しが `Cancelled`、期限切れが `Expired` を保ったまま呼び出し元へ返る形にする。根拠: Thread `28293c1e-3ae0-425c-94f4-0ed22b05729a`（R-003、B-004、Design 01 の固定するルート 8）。ルート: 委任（`WorkflowError::Stopped` の利用を含む）。
- `watch` の登録後検査が孤立した watcher を残さないようにする: `adaptor/controller/api/client.rs` の `watch` は `spawn_blocking` の `watcher.watch(...)` が完了して `result` を得た後に `context.check(Instant::now())` を置き、その `Err` が `let id = result.map_err(watch_error)?` より前に早期 return する。`usecase/watcher.rs` の `watch` は `reserve` → 実行 → `complete` で登録を確定してから id を返すため、この検査で失敗すると登録済みの watcher に対する id が呼び出し元へ届かない。id が届かない watcher が残らない形にする。根拠: Thread `5d9efb13-1962-4162-ae83-9e3f0629a93d`（今周の実装が入れた検査が生む、要求にない副作用。R-005 の後始末の対象は広げない）。ルート: 委任（検査を登録前へ移すか、失敗時に登録を解除するかを含む）。
- `ReviewError::Stopped` の詳細 code が停止の分類と食い違わないようにする: `domain/comment/mod.rs` の `ClassifiedFailure for ReviewError` は `Self::Stopped(error)` を `Expired` / `Cancelled` へ写す一方、同じ型の `code()` は `Self::Stopped(_) => ReviewErrorCode::Io` を返す。停止の分類という 1 つの概念が 2 か所で食い違う値になっている。詳細 payload の code を `failure_kind()` と食い違わない形にする。根拠: Thread `fce310ab-ea2a-4e7e-bb34-df523c3b0baa`（R-003、`AGENTS.md` のレビュー観点「同じ概念が二つの場所で表現されていないか」）。ルート: 委任（`FailureKind` に新しい種類は追加しない）。
- 実 `SystemGhCommandRunner::output` を通る gateway テストを足す: `output` は `with_timeout(GH_TIMEOUT)` と共有 process 実行を組み合わせ、`ProcessError::Stopped(Expired)` / その他の `Stopped` / `Io` を `GhCommandOutput` の `Timeout` / `Stopped` / `SpawnFailed` へ分岐するが、`github.rs` の `#[test]` は全て `with_runner` の `FakeGhRunner` を使いこの実装を通らない。引き継いだ期限と `GH_TIMEOUT` の合成、および `ProcessError` の変換を固定する。根拠: Thread `d4547481-fc1e-4d3f-a765-55c809ca06bb`（R-004、B-006、R-003、`docs/architecture/TEST.md` の `adaptor/gateway/` 必須）。ルート: 委任（`gh` の差し替え方を含む）。
- git2 の各同期経路に停止伝播の必須 gateway テストを足す: `adaptor/gateway/` 配下で `OperationContext` / `OperationStopped` を使うテストは `repository/{worktree,worktree_operation,branch_card}_test.rs`・`shared/{git_operation,process,file_lock}_test.rs`・`notion/service_impl_test.rs`・`workflow/worktree_gateway_test.rs`・`local_event_store/*` に限られ、同じく `git_operation::run` 化を受けた `repository/git_config.rs`・`repository/util.rs`・`repository/branch.rs`・`code/file_content.rs`・`code/branch_diff.rs`・`code/mod.rs`・`git_host/discovery.rs` と worktree の mutation 経路には無い。停止が通常値へ変換されず次の git2 操作へ進まないことを、これらの module で固定する。根拠: Thread `93a50338-465f-4f95-850a-3888dbe96e76`（R-002、B-009、`docs/architecture/TEST.md` の `adaptor/gateway/` 必須）。ルート: 委任（置き場所・粒度と対象 call site の洗い出しを含む）。

## 固定するルート

今周で新しく固定する実装上の指定は無い。Design 01 で固定したルート 1〜8 は解除せず維持する。今周の変える部分に直接かかるのはルート 8（失敗の分類は `FailureKind` が所有し、期限切れを `Expired`、取り消しを `Cancelled` に写す。Connect コードへの対応付けは `adaptor/protocol/connect.rs` の 1 か所のままとする）である。

## 変えないもの

- `FailureKind` に新しい種類を追加しない。停止の分類は `Expired` / `Cancelled` の既存 2 種で表し、Connect コードへの対応付けは `adaptor/protocol/connect.rs` の 1 か所のままとする。
- R-005 が定める後始末の対象（外部プロセスとファイルの lock）を広げない。`watch` の孤立は今周の実装が入れた検査の副作用として直し、watcher を後始末の対象へ加える判断はしない。
- git2 由来のその他のエラーに対する既存の許容は変えない。設定や対象が無いときに `None` を返すこと、`canonicalize_managed_worktree_path_inner` が停止以外のエラーで次の repository を試すことを含む。
- Design 01 と Design 02 の「変えないもの」の各項目も解除しない。処理の先が現在持つ期限の値、期限が引き継がれない経路に新しい期限を置かないこと、中断の口を持たない操作に口を作らないこと、store の入口の形と `execute` の受け取りの仕組み、reader プールのスレッド数と待ち行列の深さを含む。

## 未確定・リスク

- Thread `d12fb304` の受入条件が名指ししていない call site で、`git_operation::run` の結果を落とす非テストの経路が開始状態に残る。`adaptor/gateway/repository/git_config.rs` の `resolve_current_base_branch`（`repo.config()` に `.ok()`）・`resolve_base_ref_oid`（`revparse_single` と `peel_to_commit` に `.ok()`）・`resolve_effective_base_branch`（`client::open` / `repo.head()` / `repo.config()` の `Err(_) => return Ok(None)` と `merge_base` の `.is_err()`）、`adaptor/gateway/repository/worktree.rs` の `recorded_main_repo_path`（`client::open` に `if let Ok`）と、worktree 作成失敗時の巻き戻しで `find_branch` と `delete` に付いた `let _ =` が該当する。Thread `93a50338` のテストが module 単位で停止伝播を固定するため検出はされ得るが、是正そのものは今周の変える部分に入っていない。これらの経路が残る間、その経路では R-002 と B-009 を満たさない。
- 今周で自動判断により Requirements・Behavior を修正した箇所は無い。「自動判断: 未決」として残した要求も無く、`[DEFERRED]` で人間へ渡した件も無い。Design 01 の周に行った自動判断（Current Behavior の git2 の module の列挙の補正）は `requirements.md` の Assumptions に記載済みである。
