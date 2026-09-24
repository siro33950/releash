# Design 01

## 開始状態

差分の基準は `main` の `42be41e0`（`refactor(error): 失敗の分類をエラーの値に持たせる (#1880) (#1919)`）で、`feat/issues/1881` はこの commit から派生している。未コミットの変更は `docs/specs/issues-1881/` の追加だけで、コードの変更は無い。

`docs/specs/issues-1881/` に既存の Design は無いため初回とし、開始状態は `docs/specs/issues-1881/requirements.md` の Current Behavior を参照する。Review 由来ではないため、この周までに解消・見送りとなった Thread は無い。

## 変える部分

- 読み込みの入口を async の 1 つにする: `ReaderPool::submit_blocking`、`LocalEventStore::submit_indexed_query_blocking`、`LocalEventReadStore::submit_indexed_query_blocking` を削除し、全ての読み込みを `ReaderPool::submit` へ通す。根拠: R-001「読み込みの入口は async の 1 つである。（略）処理を止めて待つ読み込みの入口は残らない」、B-001。ルート: 固定（「読み込みの入口」）
- fact log の読み込みを async にする: `FactLogReadBackend::run_indexed` と、そこを通る非テストの読み込み 23 か所（`fact_log.rs` 9、`execution_archive_repository.rs` 6、`session_facts.rs` 2、`event_repository.rs` 2、`worktree_context.rs` 2、`agent_session_query_service.rs` 1、`startup_repository.rs` 1）を async にする。根拠: R-001、R-003「読み込みを使う側は async である」、B-001。ルート: 固定（「async 化の対象範囲」。方式は委任）
- 読み込みを使う同期 port を async にする: `WorkflowEventRepository`、`WorkflowExecutionProjectionRepository`、`ExecutionTreeArchiveRepository`、`WorkflowStartupRepository`、`WorkspaceTreeRepository`、`WorkspaceQueryService` と、その実装および呼び出し元（client command、CLI、debug ビルドの acceptance harness、テスト）を async にする。根拠: R-003、B-001、B-003。ルート: 固定（「async 化の対象範囲」。方式は委任）
- async のメソッドから止めて待つ読み込みを直接呼んでいる箇所を async の待ちへ移す: `AgentSessionRepository` の実装の 9 か所と同期の補助メソッド `locate` / `derive_session`、`WorkflowStartupUsecase::execute` から `list_tree_ids` への呼び出し。根拠: R-002「読み込みの結果を待っている間、呼び出し元は daemon の処理用スレッドを占有しない」、B-002。ルート: 委任
- CLI の読み込みを async の側へ移す: `cli/mod.rs` の `run` が tokio の外で動いているため、`releash review` の `review_session_context`（`LocalAgentSessionQueryService::get_blocking`）と `review_workspace_worktree`（`worktree_context` の読み込み）、`releash workflow status` / `workflow output get` の daemon 未起動時の経路（`cli/file_direct.rs`）を async の入口へ通す。根拠: R-003「tokio の外（runtime を持たない同期の呼び出し）から読み込む経路は残らない」、B-003。ルート: 委任
- 読み込みを止めて待たなくなって不要になる `spawn_blocking` の被せを削除する: `adaptor/controller/client/workspace_tree.rs` の 5 か所ほか、store の読み込みだけを包んでいるもの。根拠: R-002、B-002。ルート: 委任
- fact log の読み込みの失敗の返し方を `FactReadError` に揃える: 文字列を返す `tree_id_for_node`、`read_tree_records_from`、`list_tree_roots`、`pending_rows_for_events`（2 か所）と、`WorkflowError::external` へ直接置き換える `reconcile_tree_pass` を `FactReadError` へ寄せる。根拠: R-004「store の混雑による失敗は `UNAVAILABLE`、期限切れによる失敗は `DEADLINE_EXCEEDED` として観測される」、B-004、B-005。ルート: 固定（「規則の所有（fact log の読み込みの失敗の型）」。各関数のシグネチャは委任）
- fact log 以外で分類を捨てている変換を、分類を保つ形にする: `execution_archive_repository.rs` 6 か所、`startup_repository.rs` 1 か所、`workspace_tree/query_service.rs` 1 か所、`execution_projection_repository.rs` 1 か所の `LocalEventQueryError` → `WorkflowError::external`。根拠: R-004、B-004、B-005。ルート: 固定（「規則の所有（fact log の読み込みの失敗の型）」）
- 読み込み経路の `map_err(|_| LocalEventQueryError::InvalidRequest)` を `reader::storage_unavailable` 経由へ置き換える: 24 か所（`execution_archive_repository.rs` 8、`fact_log.rs` 6、`startup_repository.rs` 4、`reader.rs` 3、`event_repository.rs` 2、`workflow_host.rs` 1）と、debug ビルドの acceptance harness 3 か所。根拠: R-005「失敗の分類はそのエラーの種類だけで決まり、読み込みの経路によって変わらない」、B-006、B-007、B-009。ルート: 固定（「規則の所有（`rusqlite::Error` の分類）」。置き換えの手順は委任）
- SQLite エラーの分類を `reader::sqlite_failure_kind` へ一本化する: `store::sqlite_error_is_storage_unavailable` を廃止し、その 13 コードを `reader::sqlite_failure_kind` へ取り込み、`classify_sqlite_error` は `StateRequired` または `Temporary` を `StorageUnavailable` に対応づける。`store.rs` の open 経路 20 か所が対象に入る。根拠: R-005、R-006「store を開くときに（略）storage が使えないこととして観測される条件は変わらない」、B-006、B-007、B-008。ルート: 固定（「規則の所有（SQLite エラー分類の一本化先と分類表）」）

## 固定するルート

- 読み込みの入口: 既存の `ReaderPool::submit` を唯一の入口とし、`ReaderPool::submit_blocking`、`LocalEventStore::submit_indexed_query_blocking`、`LocalEventReadStore::submit_indexed_query_blocking` を削除する。`tokio-rusqlite` クレートは導入しない。範囲は local event store の読み込み経路すべて。粒度は削除対象のシンボル名までの詳細指定。理由は、tokio-rusqlite が定める標準は入口が async だけであることで、待ち行列上限と期限は標準が定めない部分であり、現行 `ReaderPool` は既にその作りを持つため。関係する要求は R-001、R-002、R-004 と B-001、B-002、B-004、B-005。
- 規則の所有（`rusqlite::Error` の分類）: `reader::sqlite_failure_kind` が所有する。読み込み経路の `map_err(|_| LocalEventQueryError::InvalidRequest)` 24 か所（`execution_archive_repository.rs` 8、`fact_log.rs` 6、`startup_repository.rs` 4、`reader.rs` 3、`event_repository.rs` 2、`workflow_host.rs` 1）を `reader::storage_unavailable` 経由へ置き換える。範囲は読み込み経路の 24 か所と、debug ビルドの acceptance harness 3 か所。`reader.rs` の引数検証（`:141`・`:271`）は対象外。粒度は所有者と対象箇所までの指定で、置き換えの手順は委任。理由は同じ `SQLITE_BUSY` が経路によって `UNAVAILABLE` と `INVALID_ARGUMENT` に割れているため。関係する要求は R-005 と B-006、B-007。
- 規則の所有（SQLite エラー分類の一本化先と分類表）: `store::sqlite_error_is_storage_unavailable` を廃止し、その 13 コードを `reader::sqlite_failure_kind` へ取り込む。取り込む 7 コード（`SQLITE_IOERR` / `SQLITE_NOMEM` / `SQLITE_PROTOCOL` / `SQLITE_TOOBIG` / `SQLITE_NOLFS` / `SQLITE_INTERRUPT` / `SQLITE_AUTH`）は一律 `FailureKind::StateRequired` とし、`Capacity` / `Cancelled` / `Permission` へ分けない。`classify_sqlite_error` は `StateRequired` または `Temporary` を `StorageUnavailable` に対応づけ、open 時の観測を維持する。範囲は `store.rs` の open 経路 20 か所を含む。粒度は分類先の `FailureKind` までの詳細指定。理由は、環境起因の失敗が読み込みで `INTERNAL` になる現状が誤りであること、発生しない 2 コードのために分類を細分化しても観測される場面がないこと。関係する要求は R-005、R-006 と B-006、B-007、B-008。
- 規則の所有（fact log の読み込みの失敗の型）: 既存の `FactReadError` が所有する。文字列を返す流儀（`tree_id_for_node`、`read_tree_records_from`、`list_tree_roots`、`pending_rows_for_events` の 2 か所）と `WorkflowError::external` へ直接置き換える箇所（`reconcile_tree_pass`）を `FactReadError` へ揃える。fact log 以外で `LocalEventQueryError` を `WorkflowError::external` へ置き換えている箇所（`execution_archive_repository.rs` 6 か所、`startup_repository.rs`、`workspace_tree/query_service.rs`、`execution_projection_repository.rs`）も分類を保つ形にする。範囲は fact log の読み込み経路とその下流。粒度は所有する型と対象箇所までの指定で、各関数のシグネチャは委任。理由は、store 由来と codec 由来を区別する型・変換・利用側の分岐が既にあり、`LocalEventQueryError` 一本化では codec 失敗のメッセージを失うため。関係する要求は R-004 と B-004、B-005。
- async 化の対象範囲: fact log の読み込み関数（`FactLogReadBackend::run_indexed` とそこを通る読み込み）と、それを使う同期 port（`WorkflowEventRepository`、`WorkflowExecutionProjectionRepository`、`ExecutionTreeArchiveRepository`、`WorkflowStartupRepository`、`WorkspaceTreeRepository`、`WorkspaceQueryService`）およびその呼び出し元（client command、CLI、debug ビルドの acceptance harness、テスト）。tokio の外で動く呼び出し元は CLI だけで、`std::thread::spawn` から store を読む箇所は無い。範囲は対象の列挙まで。粒度は対象範囲の指定で、方式（`async_trait`、runtime の作り方）は委任。理由は Request item の指定による。関係する要求は R-001、R-003 と B-001、B-003。

## 変えないもの

- `QUERY_DEADLINE_MS`（2 秒）、`READ_QUEUE_MAX_DEPTH`（128）、`READER_POOL_SIZE`（4）の値と、期限の測り方。呼び出しごとの期限と取り消しは #1883 が扱うため。
- store を開くときに storage が使えないこととして観測される条件（R-006）。`store.rs` の open 時の分類を固定するテストが対象であり、テストの期待値を実装に合わせて変えないため。
- 書き込みの入口そのものの形。writer queue の同期の入口は #1892 が扱う。読み込みを含む関数が async になることに伴う波及だけを今回の対象とする。
- 再試行ループを 1 つの実装へまとめること。#1889 が扱うため。
- `reader.rs:141` と `reader.rs:271` の引数検証による `LocalEventQueryError::InvalidRequest`。これは本来の用途であり、分類を捨てる `map_err` とは別であるため。
- `tokio-rusqlite` クレートの導入。従う標準は tokio-rusqlite の作りであり、クレートそのものではないため。
- SQL、読み込みの結果として返る値の内容、read model の形、Connect の contract（`proto/client.proto`）、HTTP local API の contract。

## 未確定・リスク

- 自動判断: R-005 が挙げる 3 つの観測のうち、保存データの破損による失敗が `DATA_LOSS` になることに対応する受入条件が無かったため、B-009 を追加し、対応表の R-005 の行を更新した。R-005 が既に書いている範囲に収めている。
- 未決のまま残した要求は無い。`[DEFERRED]` で人間へ渡した件も無い。
