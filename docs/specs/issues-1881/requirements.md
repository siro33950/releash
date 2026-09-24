# Context

- 正本: [#1881 \[02\] store の読み込みの入口を async の 1 つにする](https://github.com/siro33950/releash/issues/1881)
- 所属: [milestone #99「02. UI と daemon の間の通信の仕組みを一本化する」](https://github.com/siro33950/releash/milestone/99) の Wave 02。milestone は現状として「保存データの読み込み口が、待ち方の違う 2 つに分かれていて、呼び出し元によっては daemon の処理用スレッドを塞いでいる」を挙げ、方針として確立した標準へ合わせること、同じことをする処理に別の実装を残さないことを置く。
- 従う標準は [tokio-rusqlite](https://docs.rs/tokio-rusqlite/latest/tokio_rusqlite/) の作り。接続ごとに専用のスレッドを立て、呼び出し側はクロージャを送って結果を受け取る。`Connection` の公開メソッドは `call` / `call_raw` / `call_unwrap` / `close` / `open*` がいずれも async で、同期の入口を持たない。待ち行列の深さ上限と期限は、この標準が定めない部分である。参照は [tokio `spawn_blocking`](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)。
- 依存は [#1880](https://github.com/siro33950/releash/issues/1880)。`42be41e0` で merge 済み。失敗の分類は `FailureKind`（`src-tauri/src/domain/failure.rs`）と `ClassifiedFailure` が持ち、gRPC のステータスコードへの対応付けは `src-tauri/src/adaptor/protocol/connect.rs` の `code()` 1 か所で行う。`Temporary` が `UNAVAILABLE`、`Expired` が `DEADLINE_EXCEEDED`、`StateRequired` が `FAILED_PRECONDITION`、`Corrupt` が `DATA_LOSS`、`InvalidInput` が `INVALID_ARGUMENT`、`Internal` が `INTERNAL` に対応する。
- 正本の本文が示す行番号は HEAD（`42be41e0`）と一致しない。対象はシンボル名で特定する。
- 同じ milestone の別 ISSUE が隣接する対象を持つ。書き込みの入口の統一は #1892、呼び出しの期限と取り消しは #1883、再試行ループの統合は #1889。
- 参照する既存実装: `src-tauri/src/adaptor/gateway/local_event_store/{reader,store,read_only,commit}.rs`、`src-tauri/src/adaptor/gateway/workflow/{fact_log,workflow_host,startup_repository,event_repository,execution_archive_repository,execution_projection_repository,worktree_context}.rs`、`src-tauri/src/adaptor/gateway/agent_session/{agent_session_repository,agent_session_query_service,session_facts}.rs`、`src-tauri/src/adaptor/gateway/workspace_tree/{repository,query_service}.rs`、`src-tauri/src/adaptor/controller/client/workspace_tree.rs`、`src-tauri/src/usecase/workflow/{startup,ports}.rs`、`src-tauri/src/domain/workflow/repository.rs`、`src-tauri/src/domain/failure.rs`、`src-tauri/src/adaptor/protocol/connect.rs`、`src-tauri/src/cli/{mod,review,file_direct}.rs`。

# Outcome

Releash の daemon と CLI を変更する開発者、および daemon の応答を受け取る利用者が対象である。

現在は、保存された事実を読む入口が async と「処理を止めて待つ」の 2 つに分かれており、どちらを使うかが呼び出し元ごとに違う。止めて待つ入口は、reader が応答するまで呼び出し元のスレッドを塞ぐ。async 関数から直接呼ばれている箇所では daemon の処理用スレッドが塞がれ、その間ほかの呼び出しが進まない。塞ぐのを避けるために `spawn_blocking` を被せるかどうかも呼び出し元ごとに違い、同じ読み込みに二通りの呼び方が残っている。

失敗の分類も経路ごとに割れている。読み込みの失敗が上位へ届く途中で文字列や別の値に置き換わり、store の混雑や期限切れが破損や内部エラーとして観測される。SQLite のエラーをどう分類するかにも複数の実装があり、同じエラーが通る経路によって違う分類で観測される。

変更後は、読み込みの入口が async の 1 つになり、daemon も CLI も同じ入口を通る。読み込みの結果待ちが処理用スレッドを塞がず、読み込みの失敗はその理由に対応した分類のまま呼び出し元へ届く。同じ失敗は、どの経路を通っても同じ分類で観測される。

# Current Behavior

2026-09-24 に `feat/issues/1881`（base `main` の `42be41e0`）で確認した。

## 読み込みの入口

- reader pool は入口を 2 つ持つ。`ReaderPool::submit`（`src-tauri/src/adaptor/gateway/local_event_store/reader.rs:365`）は oneshot を返す async 向けの入口で、`ReaderPool::submit_blocking`（同 `:404`）は `mpsc::sync_channel` の `recv()` で結果を待つ入口である。reader は専用スレッド 4 本（`READER_POOL_SIZE`、同 `:29`）、待ち行列の上限は 128（`READ_QUEUE_MAX_DEPTH`、同 `:30`）、読み込みの期限は 2 秒（`QUERY_DEADLINE_MS`、同 `:31`）で、いずれも 2 つの入口で共通である。
- async の入口を通るのは 5 つの読み込みだけである。`LocalEventStore::submit_query`（`store.rs:939`）経由の `resolve_commit_row` / `load_stream_page` / `run_query`（`store.rs:1065`・`:1074`・`:1082`）と、`LocalEventReadStore::read`（`read_only.rs:177`）経由の 2 つ（`read_only.rs:342`・`:350`）である。
- 止めて待つ入口は `LocalEventStore::submit_indexed_query_blocking`（`store.rs:1003`）と `LocalEventReadStore::submit_indexed_query_blocking`（`read_only.rs:214`）から使われる。非テストの呼び出しは `fact_log.rs:112`・`:546`・`:557`・`:700`・`:701`、`workflow_host.rs:288`、`provider_lifecycle_acceptance.rs:542`（debug ビルドの acceptance harness）。テストからの呼び出しは `fact_log_test.rs:327`、`local_event_store/store_test.rs:61`、`local_event_store/node_events_test.rs:315`・`:339`・`:383`・`:421`。
- fact log の読み込みは全て止めて待つ入口を通る。共通の通り道は `FactLogReadBackend::run_indexed`（`fact_log.rs:694`）で、`Live`（daemon の store）と `ReadOnly`（CLI の read-only store）のどちらも `submit_indexed_query_blocking` へ渡す。非テストの `run_indexed` 呼び出しは 23 か所（`fact_log.rs` 9、`execution_archive_repository.rs` 6、`session_facts.rs` 2、`event_repository.rs` 2、`worktree_context.rs` 2、`agent_session_query_service.rs` 1、`startup_repository.rs` 1）。

## 呼び出し側の形

- この読み込みを使う port は同期である。`WorkflowEventRepository`（`usecase/workflow/ports.rs:17`）、`WorkflowExecutionProjectionRepository`（同 `:39`）、`ExecutionTreeArchiveRepository`（`domain/workflow/repository.rs:35`）、`WorkflowStartupRepository`（同 `:119`）、`WorkspaceTreeRepository` と `WorkspaceQueryService`（`workspace_tree/repository.rs:147`・`workspace_tree/query_service.rs:103`）はいずれも同期の trait である。
- `spawn_blocking` を被せるかどうかは呼び出し元ごとに違う。workspace tree の client command は `spawn_blocking` で包む（`adaptor/controller/client/workspace_tree.rs:21`・`:36`・`:50`・`:66`・`:81`）。一方 `AgentSessionRepository` の実装は、async のメソッドから止めて待つ読み込みを直接呼ぶ（`agent_session_repository.rs:444`・`:490`・`:568`・`:585`・`:592`・`:607`・`:614`・`:657`・`:695`）。同じ実装の同期の補助メソッド（`agent_session_repository.rs:83` の `locate`、同 `:183` の `derive_session`）も async のメソッドから呼ばれており、同様に塞ぐ。起動時の再開処理も同じで、`WorkflowStartupUsecase::execute`（`usecase/workflow/startup.rs:29`）は async のまま `self.repository.list_tree_ids()`（`startup_repository.rs:42`）を直接呼ぶ。
- tokio の外で動く呼び出し元は CLI だけである。`cli/mod.rs:107` の `run` は同期で、runtime を作らない。`releash review` の `review_session_context`（`cli/review.rs:206`）が `LocalAgentSessionQueryService::get_blocking`（`agent_session_query_service.rs:66`）を、`review_workspace_worktree`（`cli/review.rs:227`）が `worktree_context` の読み込みを、`releash workflow status` / `workflow output get` の daemon 未起動時の経路（`cli/file_direct.rs:13`・`:28`）が `WorkflowExecutionProjectionRepository::get_execution` 経由で `fact_log::read_tree_records_from` を、いずれも同期のまま呼ぶ。daemon 側は `lib.rs:65` で runtime を作ってから動く。`std::thread::spawn` から store を読む箇所は無い。

## 失敗の分類

- reader pool の入口は混雑を `LocalEventQueryError::QueryBusy`、期限切れを `LocalEventQueryError::DeadlineExceeded` として返し、それぞれ `FailureKind::Temporary` と `FailureKind::Expired` に分類される（`domain/local_event/query.rs:81-95`）。正本が現状として挙げる「失敗は `Unavailable` にまとめられる」は、#1880（`42be41e0`）でこの形に変わっている。
- fact log の読み込み関数は、この値を文字列や別の値へ置き換える。`tree_id_for_node`（`fact_log.rs:716`）、`read_tree_records_from`（同 `:731`）、`list_tree_roots`（同 `:1202`）、`pending_rows_for_events`（同 `:550`・`:561`）が文字列にし、`reconcile_tree_pass`（同 `:1050`）が `WorkflowError::external` にする。文字列は `FactReadError::Corrupt`（`fact_log.rs:36-39`、`FailureKind::Corrupt`）、`WorkflowError::external`（`FailureKind::Internal`）、`LocalEventQueryError::IncompatibleStoredEvent`（`workspace_tree/repository.rs:277-289`、`FailureKind::StateRequired`）になる。
- fact log 以外にも、`LocalEventQueryError` を `WorkflowError::external`（`FailureKind::Internal`）へ置き換える箇所がある。`execution_archive_repository.rs:60`・`:82`・`:103`・`:117`・`:211`・`:249`、`startup_repository.rs:79`、`workspace_tree/query_service.rs:131`、`execution_projection_repository.rs:53`。
- その結果、store が混んで読み込みに失敗したとき、workspace tree の経路（`workspace_tree/repository.rs:57`・`:64`・`:81`）では `FAILED_PRECONDITION`、workflow execution の projection の経路（`execution_projection_repository.rs:53`）では `INTERNAL` が観測される。
- fact log の読み込みの失敗の返し方は 2 つの流儀に分かれている。`FactReadError`（`fact_log.rs:29`）は store 由来（`Query`）と codec 由来（`Corrupt`）を型で区別し、`ClassifiedFailure` と、`LocalEventQueryError` / `WorkflowError` への変換を持つ（同 `:40`・`:52`・`:61`）。これを返す関数は 7 つ（同 `:799`・`:820`・`:827`・`:848`・`:871`・`:1262`・`:1270`）で、`session_facts.rs:193`・`:202` が分岐して使う。残りは上記のとおり文字列を返す。
- 読み込みのクロージャの中で `rusqlite::Error` が出たときの分類にも 2 つの実装がある。`reader::sqlite_failure_kind`（`reader.rs:43`）と `reader::storage_unavailable`（同 `:62`）はエラーコードを見て `Temporary` / `Corrupt` / `StateRequired` / `Internal` に分ける。読み込み経路での使用は 21 か所（`fact_log.rs` 8、`reader.rs` 7、`worktree_context.rs` 3、`session_facts.rs` 2、`agent_session_query_service.rs` 1）。もう一方はエラーを捨てて `LocalEventQueryError::InvalidRequest`（`FailureKind::InvalidInput`）にする `map_err(|_| ...)` で、読み込み経路に 24 か所ある（`execution_archive_repository.rs` 8、`fact_log.rs` 6、`startup_repository.rs` 4、`reader.rs` 3、`event_repository.rs` 2、`workflow_host.rs` 1）。ほかに debug ビルドの acceptance harness に 3 か所。`reader.rs:141`・`:271` の `InvalidRequest` は引数の検証であり、この 24 か所には含まない。その結果、同じ `SQLITE_BUSY` が経路によって `UNAVAILABLE` にも `INVALID_ARGUMENT` にもなる。
- store を開くときの分類はさらに別の実装を持つ。`store::sqlite_error_is_storage_unavailable`（`store.rs:62`）が 13 のエラーコードを `StorageUnavailable` と判定し、`classify_sqlite_error`（同 `:84`）経由で 20 か所から使われる。`reader::sqlite_failure_kind` と対象のコードが食い違い、`SQLITE_IOERR` / `SQLITE_NOMEM` / `SQLITE_PROTOCOL` / `SQLITE_TOOBIG` / `SQLITE_NOLFS` / `SQLITE_INTERRUPT` / `SQLITE_AUTH` の 7 つは、open では `StorageUnavailable`、読み込みでは `Internal`（`INTERNAL`）になる。`store.rs:1134` のテストが、open 時に `StorageUnavailable` となる 10 のコードを固定している。書き込み経路の `commit::storage_unavailable`（`commit.rs:30`）は `reader::sqlite_failure_kind` を呼んでおり、独立の実装を持たない。
- `sqlite3_interrupt` の呼び出しと `set_authorizer` の設定は無く、`SQLITE_INTERRUPT` と `SQLITE_AUTH` は現状のコードでは発生しない。

# Scope / Non-goals

変更する。

- local event store の読み込みの入口を async の 1 つにすること。`ReaderPool::submit_blocking`、`LocalEventStore::submit_indexed_query_blocking`、`LocalEventReadStore::submit_indexed_query_blocking` を残さないことを含む。
- fact log の読み込み関数（`FactLogReadBackend::run_indexed` とそこを通る読み込み）と、それを使う側を async にすること。Current Behavior に挙げた同期の port（`WorkflowEventRepository`、`WorkflowExecutionProjectionRepository`、`ExecutionTreeArchiveRepository`、`WorkflowStartupRepository`、`WorkspaceTreeRepository`、`WorkspaceQueryService`）と、その呼び出し元（client command、CLI、debug ビルドの acceptance harness、テスト）を含む。
- tokio の外で動いている読み込みの呼び出し元（CLI）の処理を async の側へ移すこと。
- 読み込みの失敗が呼び出し元へ届くまで #1880 の分類を保つこと。混雑を `UNAVAILABLE`、期限切れを `DEADLINE_EXCEEDED` として観測できるようにすることを含む。fact log の読み込み関数が文字列や `WorkflowError::external` へ置き換える箇所と、`execution_archive_repository`・`startup_repository`・`workspace_tree/query_service`・`execution_projection_repository` で同じ置き換えをしている箇所を残さない。
- fact log の読み込みの失敗の返し方を `FactReadError` に揃え、文字列を返す流儀を残さないこと。
- 読み込みのクロージャの中で `rusqlite::Error` を分類する実装を `reader::sqlite_failure_kind` に一本化すること。読み込み経路の `map_err(|_| LocalEventQueryError::InvalidRequest)` 24 か所と、store を開く経路の `store::sqlite_error_is_storage_unavailable` を残さない。後者が対象にしていたエラーコードは `reader::sqlite_failure_kind` が引き取る。
- 読み込みを止めて待たなくなったことで不要になる `spawn_blocking` の被せ（`adaptor/controller/client/workspace_tree.rs` ほか、store の読み込みだけを包んでいるもの）の削除。

変更しない。

- `tokio-rusqlite` クレートの導入。従う標準は tokio-rusqlite の作りであり、クレートそのものではない。現行の `ReaderPool` は既にその作りを持ち、標準が定めない待ち行列の上限と期限を追加で持つ。
- 書き込みの入口。writer queue の同期の入口（`append_node_events_blocking`、`append_node_events_at_head_blocking` ほか）は #1892 が扱う。読み込みを含む関数が async になることに伴う波及は今回の対象だが、書き込みの入口そのものの形は変えない。
- 呼び出しごとの期限と取り消しの導入（#1883）。`QUERY_DEADLINE_MS`、`READ_QUEUE_MAX_DEPTH`、`READER_POOL_SIZE` の値と、期限の測り方も変えない。
- 再試行ループを 1 つの実装へまとめること（#1889）。
- store の読み込みを含まない同期処理（git 操作、ファイル I/O、process 操作など）と、それを包む `spawn_blocking`。
- SQL、読み込みの結果として返る値の内容、read model の形。
- Connect の contract（`proto/client.proto`）と HTTP local API の contract。
- `reader.rs:141`・`:271` の引数の検証による `InvalidRequest`。

# Requirements

- R-001: local event store の読み込みの入口は async の 1 つである。daemon と CLI を含む全ての読み込みがその入口を通り、処理を止めて待つ読み込みの入口は残らない。
- R-002: 読み込みの結果を待っている間、呼び出し元は daemon の処理用スレッドを占有しない。ある読み込みが滞っていても、その間に届いた他の呼び出しは進む。
- R-003: 読み込みを使う側は async である。tokio の外（runtime を持たない同期の呼び出し）から読み込む経路は残らない。
- R-004: 読み込みの失敗が呼び出し元へ届くとき、store の混雑による失敗は `UNAVAILABLE`、期限切れによる失敗は `DEADLINE_EXCEEDED` として観測される。
- R-005: 読み込みが SQLite のエラーで失敗したとき、失敗の分類はそのエラーの種類だけで決まり、読み込みの経路によって変わらない。データベースが他から使用中であることによる失敗は `UNAVAILABLE`、保存データの破損による失敗は `DATA_LOSS`、権限・容量・I/O・ロックなど実行環境に起因する失敗は `FAILED_PRECONDITION` として観測される。
- R-006: store を開くときに SQLite のエラーで失敗したとき、storage が使えないこととして観測される条件は変わらない。

# Assumptions / Open Questions

- 自動判断: R-005 が挙げる 3 つの観測のうち、保存データの破損による失敗が `DATA_LOSS` になることに対応する受入条件が無かったため、B-009 を追加した。R-005 が既に書いている範囲に収め、新しい観測可能な挙動と対象範囲を増やしていない。
- Open Question: なし。
