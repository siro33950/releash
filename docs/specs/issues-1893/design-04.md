# Design 04

## 開始状態

- 差分の基準は base branch `main`、派生点は branch `feat/issues/1893` の HEAD `9194bd57`。開始状態の実装は、`9194bd57` に 1 周目・2 周目・3 周目の未コミット変更を加えたものである。
- 直前の Design は `docs/specs/issues-1893/design-03.md`。Design 01 の「変える部分」14 項目、Design 02 の 4 項目、Design 03 の 7 項目は実装済みであり、本文では再掲しない。Design 03 の 7 項目は開始状態の実装で確認した（`git_config.rs` の各 resolve が `git_operation::optional` で停止を保つ、`code/file_content.rs` の `discover_repo` が `GitOperationError::Stopped` を早期 return する、`canonicalize_managed_worktree_path_inner` の停止で次の repository へ進まない、`query_error` が分類を保つ、`watch` の孤立の解消、`ReviewError` の code と `failure_kind()` の一致、`SystemGhCommandRunner::output` と各 gateway module の停止伝播テスト）。
- この周までに解消・見送りとなった Thread は無い。resolve 済みの Thread は 0 件、`[DEFERRED]` とした Thread は 0 件、`[REJECTED]` とした Thread は 0 件である。今周の入力は `[FIX_POLICY]` が付いた open Thread 3 件（`6c45d488` / `d12fb304` / `93a50338`）で、Requirements・Behavior の変更は無い。

## 変える部分

- `is_created` の `existing.validate()` が停止で止まるようにする: `adaptor/gateway/workflow/worktree_gateway.rs` の `is_created` は `git2::Repository::open`・`repo.find_worktree`・`git2::Repository::open(path)`・`isolated.head()` を `git_operation::run` で包む一方、`existing.validate()` だけを直接 `?` で呼ぶ。この操作の実行中に停止しても、後続の `existing.path().canonicalize()` と `path.canonicalize()` を実行してから次の `git_operation::run` で初めて検出される。`existing.validate()` が終わった時点で止まり、後続のパス検証と次の git2 操作へ進まない形にする。停止は分類を保ったまま `isolated_worktree_error` 経由で `WorkflowError::Stopped` として呼び出し元へ返る。根拠: Thread `6c45d488-7006-4b1c-9eaf-c32f47372944`（R-002「いずれも次の操作へ進まない」、B-009）。ルート: 委任（包み方を含む）。
- `recorded_main_repo_path` と `create_worktree` 失敗時の巻き戻しが停止を捨てないようにする: `adaptor/gateway/repository/worktree.rs` の `recorded_main_repo_path` は `git_operation::run(|| client::open(path))` の `Stopped` を `if let Ok` で欠損へ変え、`.git` ファイルからの解決へ進む。戻り型 `Option<String>` は停止を表せない。同じ file の `create_worktree` は、`repo.worktree(...)` の失敗後の巻き戻しで `find_branch` と `delete` の `Stopped` を `let _ =` で捨て、元の git エラーを返す。いずれも停止した時点で次の操作へ進まず、分類（`Expired` / `Cancelled`）を保ったまま呼び出し元へ返る形にする。根拠: Thread `d12fb304-0a34-4636-9bfa-dd257879d044`（R-002、R-003、B-009、および Design 03 の未確定・リスクの自認）。ルート: 委任（`recorded_main_repo_path` の戻り型の変更と `adaptor/gateway/workflow/execution_archive_repository.rs` の `candidate_page` での受け方、巻き戻しでの停止の扱いを含む）。
- 上記 2 経路に停止伝播の必須 gateway テストを足す: `repository/worktree_test.rs` にある停止テストは `test_worktree列挙_途中の停止を欠損や成功に変えず返す`・`test_worktree一覧と掃除_各git操作の停止を成功に変えず後続へ進まない`・`test_worktree掃除_無効な登録のpruneでも停止を返す`・`test_worktree変更_既存branchの作成と削除は各操作で停止する` の 4 件で、最後の 1 件は既存 branch を使う成功経路であり、`recorded_main_repo_path` と `create_branch` が真のときの `create_worktree` 失敗時の巻き戻しはいずれも通らない。この 2 分岐について、停止した `OperationContext` の下で停止が欠損・元の git エラーへ変換されず次の操作へ進まないことを固定する。根拠: Thread `93a50338-465f-4f95-850a-3888dbe96e76`（R-002、B-009、`docs/architecture/TEST.md` の `adaptor/gateway/` 必須）。ルート: 委任（置き場所・粒度と対象 call site の洗い出しを含む）。

## 固定するルート

今周で新しく固定する実装上の指定は無い。Design 01 で固定したルート 1〜8 は解除せず維持する。今周の変える部分に直接かかるのはルート 8（失敗の分類は `FailureKind` が所有し、期限切れを `Expired`、取り消しを `Cancelled` に写す。Connect コードへの対応付けは `adaptor/protocol/connect.rs` の 1 か所のままとする）である。

## 変えないもの

- git2 由来のその他のエラーに対する既存の許容は変えない。`is_created` が `git2::ErrorCode::NotFound` を `Ok(false)` にすること、`recorded_main_repo_path` が `client::open` の停止以外の失敗で `.git` ファイルからの解決へ進むこと、`create_worktree` の巻き戻しの失敗そのものは元の作成エラーを返すことを含む。
- `FailureKind` に新しい種類を追加しない。停止の分類は `Expired` / `Cancelled` の既存 2 種で表し、Connect コードへの対応付けは `adaptor/protocol/connect.rs` の 1 か所のままとする。
- Design 01・Design 02・Design 03 の「変えないもの」の各項目も解除しない。処理の先が現在持つ期限の値、期限が引き継がれない経路に新しい期限を置かないこと、中断の口を持たない操作に口を作らないこと、store の入口の形と `execute` の受け取りの仕組み、reader プールのスレッド数と待ち行列の深さ、R-005 が定める後始末の対象を広げないことを含む。

## 未確定・リスク

- 今周で自動判断により Requirements・Behavior を修正した箇所は無い。「自動判断: 未決」として残した要求も無く、`[DEFERRED]` で人間へ渡した件も無い。Design 01 の周に行った自動判断（Current Behavior の git2 の module の列挙の補正）は `requirements.md` の Assumptions に記載済みである。
