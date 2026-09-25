# Design 05

## 開始状態

- 差分の基準は base branch `main`、派生点は branch `feat/issues/1893` の HEAD `9194bd57`。開始状態の実装は、`9194bd57` に 1 周目から 4 周目までの未コミット変更を加えたものである。
- 直前の Design は `docs/specs/issues-1893/design-04.md`。Design 01 の「変える部分」14 項目、Design 02 の 4 項目、Design 03 の 7 項目、Design 04 の 3 項目は実装済みであり、本文では再掲しない。Design 04 の 3 項目は開始状態の実装で確認した（`worktree_gateway.rs` の `is_created` が `existing.validate()` を `git_operation::run` で包む、`repository/worktree.rs` の `recorded_main_repo_path` が `git_operation::optional` で `Result<Option<String>, OperationStopped>` を返す、同 file の `create_worktree` が `repo.worktree(...)` 失敗時に `GitOperationError::Stopped` を巻き戻し前に返し巻き戻しも `git_operation::optional` で停止を保つ、`repository/worktree_test.rs` に `test_旧worktreeの所属repo_停止を欠損やgitファイルの復元へ変えない` と `test_worktree作成失敗_巻き戻しの停止を元のgitエラーへ変えず後続へ進まない` の 2 件）。
- この周までに解消・見送りとなった Thread は無い。resolve 済みの Thread は 0 件、`[DEFERRED]` とした Thread は 0 件、`[REJECTED]` とした Thread は 0 件である。今周の入力は `[FIX_POLICY]` が付いた open Thread 2 件（`777c1243` / `d12fb304`）で、Requirements・Behavior の変更は無い。

## 変える部分

- `FileLease` の破棄時の後始末が、停止済みの context で取得できずに idle lock ファイルを残さないようにする: `adaptor/gateway/repository/worktree_operation.rs` の `FileLease::drop` は保持 lock を `fs2::FileExt::unlock` で解放したのち `remove_idle` を呼ぶ。`remove_idle` は `registry_lock` → `wait_lock` → `shared::file_lock::exclusive(file, &operation_context::current())` を通り、`file_lock::try_exclusive` は OS lock の取得前に `context.check(Instant::now())` を行う。`operation_context::current()` は task_local / thread_local の ambient な値のため、停止で巻き戻している同じ scope の Drop では必ず `LockError::Stopped` になり、Drop は警告を出して捨てる。`9194bd57` の `registry_lock` は `fs2::FileExt::lock_exclusive` で context に依らず取得しており、この掃除は成立していた。期限切れ・取り消しの後も、他に保持者のいない `{key}.admission` / `{key}.active` が残らない形にする。後始末のための registry lock の取得は引き継いだ期限を超えて待たない。根拠: Thread `777c1243-45b4-4940-b164-18f2747595bc`（`requirements.md` の Outcome「やめた操作の後始末が残る」を解消し「処理は止まって、確保していた外部プロセスと lock を残さない」状態にすること、R-001「lock の待ちへ引き継がれ、いずれもその期限を超えて続かない」、Design 01 の固定するルート 4）。ルート: 委任（停止済み context に縛られない取得、待たない取得など、R-001 と両立する形の選択を含む）。
- `remove_worktree` の現在ブランチの解決が停止を欠損へ変えないようにする: `usecase/repository_usecase.rs:318` の `let branch = self.branch.current(worktree_path).ok();` は、`adaptor/gateway/repository/branch.rs` の `get_current_branch`（`git_operation::run(|| client::open(repo_path))` と `git_operation::run(|| repo.head())`）が返す `RepositoryError::Stopped` を `None` へ変える。同じ関数はそのまま `archives.archive_worktree`、`self.worktree_terminals.kill_by_worktree`、`deletion.accept`、`self.notify_repository_changed`、`spawn_blocking` での実削除へ進み `Ok(())` を返す。この経路は `adaptor/controller/client/repository/worktree.rs` の `remove_worktree_shared` から呼ばれ、呼び出しの期限と取り消しが載る ambient な `OperationContext` の下で動く。停止した時点で次の操作へ進まず、分類（`Expired` / `Cancelled`）を保ったまま呼び出し元へ返る形にする。根拠: Thread `d12fb304-0a34-4636-9bfa-dd257879d044`（R-002「いずれも次の操作へ進まない」、R-003、B-009）。ルート: 委任（`branch` の解決失敗と停止の区別の付け方、戻り型の扱いを含む）。

## 固定するルート

今周で新しく固定する実装上の指定は無い。Design 01 で固定したルート 1〜8 は解除せず維持する。今周の変える部分に直接かかるのはルート 4（期限付きでファイル lock を待つ手続きを 1 か所へまとめる。諦める判断は Deadline が所有し、lock の取得自体は各 gateway が持つ）とルート 8（失敗の分類は `FailureKind` が所有し、期限切れを `Expired`、取り消しを `Cancelled` に写す。Connect コードへの対応付けは `adaptor/protocol/connect.rs` の 1 か所のままとする）である。

## 変えないもの

- `remove_idle` が他の保持者のいる lock ファイルを削除しない現在の挙動。`try_lock_exclusive` が `WouldBlock` のときに何もせず戻る形を維持する。
- `FileLease::drop` が保持していた lock を解放すること（R-005・B-005）。開始状態で満たされており変更しない。
- 停止以外の失敗で現在のブランチが解決できないときに、`remove_worktree` が `branch` を `None` として削除を続ける既存の許容。
- R-005 が定める後始末の対象（外部プロセスとファイルの lock）を広げない。Design 03 の「変えないもの」を維持する。
- `FailureKind` に新しい種類を追加しない。停止の分類は `Expired` / `Cancelled` の既存 2 種で表し、Connect コードへの対応付けは `adaptor/protocol/connect.rs` の 1 か所のままとする。
- Design 01・Design 02・Design 03・Design 04 の「変えないもの」の各項目も解除しない。処理の先が現在持つ期限の値、期限が引き継がれない経路に新しい期限を置かないこと、中断の口を持たない操作に口を作らないこと、store の入口の形と `execute` の受け取りの仕組み、reader プールのスレッド数と待ち行列の深さ、git2 由来のその他のエラーに対する既存の許容を含む。

## 未確定・リスク

- 今周で自動判断により Requirements・Behavior を修正した箇所は無い。「自動判断: 未決」として残した要求も無く、`[DEFERRED]` で人間へ渡した件も無い。Design 01 の周に行った自動判断（Current Behavior の git2 の module の列挙の補正）は `requirements.md` の Assumptions に記載済みである。
