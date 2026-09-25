# Design 02

## 開始状態

- 差分の基準は base branch `main`、派生点は branch `feat/issues/1893` の HEAD `9194bd57`。開始状態の実装は、`9194bd57` に 1 周目の実装の未コミット変更（`src-tauri/` の 89 ファイル、`domain/operation_context.rs`・`other/operation_context.rs`・`adaptor/gateway/shared/{git_operation,file_lock,process}.rs` の新設を含む）を加えたものである。
- 直前の Design は `docs/specs/issues-1893/design-01.md`。Design 01 の「変える部分」に挙げた 14 項目は 1 周目で実装済みであり、本文では再掲しない。
- この周までに解消・見送りとなった Thread は無い。resolve 済みの Thread は 0 件、`[DEFERRED]` とした Thread は 0 件である。今周の入力は `[FIX_POLICY]` が付いた open Thread 4 件（`2f2742ad` / `bec3dda8` / `ce8811b4` / `01a64db8`）で、Requirements・Behavior の変更は無い。

## 変える部分

- `each_worktree` の列挙が停止を握り潰さないようにする: `git_operation::run(|| repo.find_worktree(&name)).ok()?` が `GitOperationError::Stopped` を要素の欠損へ変換し、`filter_map` が次の index へ進む。停止を呼び出し元へ返し、欠けた一覧を成功として返さない形にする。呼び出し元は `repository/worktree.rs` の 4 箇所（`prune_invalid_worktrees`、`list_worktrees`、`removal_target`、`remove_worktree`）と `repository/branch_card.rs` の 1 箇所（`build_worktree_map`）の計 5 箇所。根拠: Thread `ce8811b4-bea6-4ac8-83f2-49fcd3818b20`（R-002「いずれも次の操作へ進まない」、B-009、R-003）。ルート: 委任。
- isolated worktree gateway が停止の分類を保つようにする: `RepositoryIsolatedWorktreeGateway` の `repository_root`・`is_created`・`create` が `RepositoryError` / `Box<dyn std::error::Error>` を `WorkflowError::external(error.to_string())` へ文字列化し、`domain/workflow/error.rs` の `Self::External(_) => F::Internal` で `Expired` / `Cancelled` が失われる。停止を分類を保ったまま `WorkflowError` へ写す。根拠: Thread `01a64db8-3911-4cd2-97bb-02e97633adcd`（R-003、Design 01 の固定するルート 8）。ルート: 委任。
- Notion gateway に期限切れの必須テストを足す: `send` は呼び出し context と `REQUEST_TIMEOUT`（10 秒）を合成して HTTP 応答を待ち、`send_with_retry` は同じ context で `Retry-After` を待つが、`service_impl_test.rs` の 2 件はいずれも `OperationStopped::Cancelled` だけを検証する。引き継いだ期限が `REQUEST_TIMEOUT` より早いときに両方の待ちが `Expired` で終わることを固定する。根拠: Thread `2f2742ad-df74-456d-b345-bd1cbf19d666`（R-004、B-006、R-003、`docs/architecture/TEST.md` の `adaptor/gateway/` 必須）。ルート: 委任。
- worktree 削除の待ちに期限切れの必須テストを足す: `usecase/worktree_operation.rs` の `delete_many` が `ready_to_delete` を `operation_context::wait` で待つ経路について、`worktree_operation_test.rs` の `test_worktree削除待ち_取り消しで予約を解放する` は `Cancelled` と予約解放だけを検証する。Deadline を設定した `OperationContext` の下で `Expired` により待ちが終わり、`begin_deletion` の削除予約が解放されることを固定する。根拠: Thread `bec3dda8-074d-44ff-b43f-ce4413db6ffb`（R-001、B-003、R-005、B-005、R-003、`docs/architecture/TEST.md` の `usecase/` 必須）。ルート: 委任。

## 固定するルート

今周で新しく固定する実装上の指定は無い。Design 01 で固定したルート 1〜8 は解除せず維持する。今周の変える部分に直接かかるのはルート 8（失敗の分類は `FailureKind` が所有し、期限切れを `Expired`、取り消しを `Cancelled` に写す。Connect コードへの対応付けは `adaptor/protocol/connect.rs` の 1 か所のままとする）である。

## 変えないもの

- `FailureKind` に新しい種類を追加しない。停止の分類は `Expired` / `Cancelled` の既存 2 種で表し、Connect コードへの対応付けは `adaptor/protocol/connect.rs` の 1 か所のままとする。Design 01 の「変えないもの」を維持する。
- Design 01 の「変えないもの」のその他の項目（処理の先が現在持つ期限の値、期限が引き継がれない経路に新しい期限を置かないこと、中断の口を持たない操作に口を作らないこと、store の入口の形と `execute` の受け取りの仕組み、reader プールのスレッド数と待ち行列の深さ）も解除しない。

## 未確定・リスク

- `git_operation::run` の結果から停止を握り潰す call site が、今周の変える部分に挙げた `worktree.rs:120` 以外にも残る。`9194bd57` + 1 周目の実装で `.ok()` / `.ok()?` / `.is_err()` / `let _ =` / `unwrap_or_else` により結果を落とす非テストの call site は 21 箇所あり、`repository/git_config.rs`・`repository/branch_card.rs`・`repository/worktree.rs`・`code/file_content.rs` に分布する。Thread が指摘したのはそのうち 1 箇所であり、残る箇所で `Stopped` が握り潰されていれば、その経路は R-002 と B-009 を満たさない。
- 自動判断（Design 01 の周に行い、`requirements.md` の Assumptions に記載済み）: Current Behavior の git2 の module の列挙を `9194bd57` の実装に合わせて補正した。正本の記載に無い `repository/git_config`、`repository/watch`、`code/diff_compute`、`git_host/discovery`、`workflow/worktree_gateway` が非テストの経路で `git2::` を使う。
- 今周で自動判断により Requirements・Behavior を修正した箇所は無い。「自動判断: 未決」として残した要求も無く、`[DEFERRED]` で人間へ渡した件も無い。
