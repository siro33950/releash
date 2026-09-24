# Context

- 正本: [#1892 \[03\] store の書き込みの入口を async の 1 つにする](https://github.com/siro33950/releash/issues/1892)
- 所属: [milestone #99「02. UI と daemon の間の通信の仕組みを一本化する」](https://github.com/siro33950/releash/milestone/99) の Wave 03。milestone は現状として「保存データの読み込み口が、待ち方の違う 2 つに分かれていて、呼び出し元によっては daemon の処理用スレッドを塞いでいる」「失敗が一時的か、何度やっても失敗するものかを、呼び出し元ごとに別々に判定している」を挙げ、方針として確立した標準へ合わせること、同じことをする処理に別の実装を残さないことを置く。
- 従う標準は [tokio-rusqlite](https://docs.rs/tokio-rusqlite/latest/tokio_rusqlite/) の作り。接続ごとに専用のスレッドを立て、呼び出し側はクロージャ（`CallFn = Box<dyn FnOnce(&mut rusqlite::Connection) + Send>`）を送り、結果を `tokio::sync::oneshot` で受け取る。スレッド側は渡された関数を実行するだけで、要求の種類による分岐を持たない。`Connection` の公開メソッドは `call` / `call_raw` / `call_unwrap` / `close` / `open*` がいずれも async で、同期の入口を持たない。チャネルは境界を持たないため、待ち行列の深さ上限、車線、期限は、この標準が定めない部分である。#1881 は読み込みでこの形を採り、標準が定めない期限と上限を `ReaderPool` の job のうちクロージャとは別のフィールドに持たせた。
- 依存は [#1880](https://github.com/siro33950/releash/issues/1880)（`42be41e0` で merge 済み）と [#1881](https://github.com/siro33950/releash/issues/1881)（PR #1921、`b1fba8eb` で merge 済み）。
- #1880 により、失敗の分類は `FailureKind`（`src-tauri/src/domain/failure.rs`）と `ClassifiedFailure` が持ち、gRPC のステータスコードへの対応付けは `src-tauri/src/adaptor/protocol/connect.rs` の `code()` 1 か所で行う。`Temporary` が `UNAVAILABLE`、`Capacity` が `RESOURCE_EXHAUSTED`、`RestartRequired` が `ABORTED`、`StateRequired` が `FAILED_PRECONDITION`、`Corrupt` が `DATA_LOSS`、`Expired` が `DEADLINE_EXCEEDED` に対応する。
- 失敗をどのコードで表すかの標準は gRPC と Connect が定める。gRPC の `doc/statuscodes.md` は「Sent or received message was larger than configured limit」を `RESOURCE_EXHAUSTED` とし、`FAILED_PRECONDITION` / `ABORTED` / `UNAVAILABLE` の使い分けを「(a) Use `UNAVAILABLE` if the client can retry just the failing call. (b) Use `ABORTED` if the client should retry at a higher level. (c) Use `FAILED_PRECONDITION` if the client should not retry until the system state has been explicitly fixed.」と定める。Connect プロトコルの仕様は `resource_exhausted` を「Operation can't be completed because some resource is exhausted. Use unavailable if the server is temporarily overloaded and the caller should retry later.」、`unavailable` を「The service is currently unavailable, usually transiently. Clients should back off and retry idempotent operations.」と定める。待ち行列が溢れて書き込みを受け付けられないのは、同じ呼び出しをそのまま再試行すれば通る一時的な過負荷であり、batch の大きさ上限の超過は、同じ呼び出しを再試行しても通らない資源の枯渇である。
- #1881 により、読み込みの入口は `ReaderPool::submit` の 1 つになり、store の混雑による読み込みの失敗を `UNAVAILABLE`、期限切れを `DEADLINE_EXCEEDED` として観測することが固定されている（#1881 の R-004）。
- 正本の本文が示す行番号は HEAD（`d2610dcb`）と一致しない。対象はシンボル名で特定する。正本が非テストの呼び出し元として挙げる `workflow_host.rs` の 2 か所は、実際にはテストである（Current Behavior に記載）。
- 同じ milestone の別 ISSUE が隣接する対象を持つ。書き込みの経路の再試行と起動時の再開処理の失敗の扱いは [#1890](https://github.com/siro33950/releash/issues/1890)、呼び出しの期限の引き継ぎは [#1893](https://github.com/siro33950/releash/issues/1893)、同時実行枠の優先度は [#1894](https://github.com/siro33950/releash/issues/1894)。
- 参照する既存実装: `src-tauri/src/adaptor/gateway/local_event_store/{store,writer,commit,reader,node_events}.rs`、`src-tauri/src/adaptor/gateway/workflow/{fact_log,workflow_host,startup_repository,event_log_writer,execution_archive_repository,test_support}.rs`、`src-tauri/src/domain/local_event/{batch,query,repository}.rs`、`src-tauri/src/domain/workflow/repository.rs`、`src-tauri/src/usecase/workflow/ports.rs`、`src-tauri/src/domain/failure.rs`、`src-tauri/src/adaptor/protocol/connect.rs`。

# Outcome

Releash の daemon を変更する開発者、および daemon の応答を受け取る利用者が対象である。

現在は、保存データへ書き込む入口が async と「処理を止めて待つ」の 2 つに分かれている。止めて待つ入口は、writer が応答するまで呼び出し元のスレッドを塞ぐ。この入口は tokio の async の経路から `spawn_blocking` なしで呼ばれており、daemon の処理用スレッドが塞がれ、その間ほかの呼び出しが進まない。入口を挟んで同期の関数と async の関数が混在しており、同じ書き込みに二通りの呼び方が残っている。

失敗の分類も入口ごとに割れている。writer の待ち行列が溢れたという同じ原因の失敗が、commit の入口では容量の超過、node event の追記では storage が一時的に使えないこととして観測される。

変更後は、書き込みの入口が async の 1 つになり、全ての書き込みがその入口を通る。書き込みの結果待ちが daemon の処理用スレッドを塞がず、待ち行列が溢れたことによる失敗は、どの書き込みからも同じ分類で観測される。

# Current Behavior

2026-09-24 に `feat/issues/1892`（base `main` の `d2610dcb`）で確認した。

## 書き込みの入口

- writer は専用スレッド 1 本（`local-event-store-writer`、`src-tauri/src/adaptor/gateway/local_event_store/store.rs:696`）で、writer 接続への rusqlite の呼び出しをすべて所有する。待ち行列は `WriteQueue`（`writer.rs:152`）で、normal（`NORMAL_LANE_MAX_REQUESTS` 1024 件 / `NORMAL_LANE_MAX_BYTES` 64 MiB、`writer.rs:18-19`）と critical（128 件 / 8 MiB、同 `:20-21`）の 2 車線を持つ。車線への受け入れは `WriteQueue::admit`（`writer.rs:167`）1 か所で、上限の判定もここで行う。
- 入口は 2 種類ある。async の入口は `LocalEventStore::commit_batch`（`LocalEventTransactionRepository` の実装、`store.rs:994`）と `commit_batch_with_node_events`（同 `:879`）で、`tokio::sync::oneshot` で結果を待つ。処理を止めて待つ入口は `append_node_event_blocking`（同 `:936`）、`append_node_events_blocking`（同 `:945`）、`append_node_events_at_head_blocking`（同 `:952`）で、`std::sync::mpsc::sync_channel(1)`（同 `:957`）の `recv()` で結果を待つ。期限は無い。
- node event の追記は常に normal 車線に入る。`WriteRequest::critical`（`writer.rs:115-120`）が `NodeEventAppend` に対して false を返す。commit は `CommitOperationKind::is_critical()` の結果で車線が決まる。
- writer thread 側の処理は入口ごとに分かれる。commit は `execute_commit`（`store.rs:701`）、node event の追記は `store.rs:713` から始まる分岐のクロージャで、`expected_tree_head` の照合（同 `:718` からの分岐）と `node_events::append_node_event` の繰り返しを 1 つの transaction で行う。

## 止めて待つ入口の呼び出し元

- 非テストの呼び出しは 4 か所である。`fact_log::append_pending_rows_blocking`（`workflow/fact_log.rs:575`、同期）が `:587` と `:597`、`fact_log::reconcile_tree_pass`（同 `:1044`、async）が `:1171`、`StoredWorkflowStartupRepository::append`（`workflow/startup_repository.rs:132`、async）が `:145`、`WorkflowRuntimeHost::append_events_at_head`（`workflow/workflow_host.rs:337`、async）が `:355`。いずれも tokio の async の経路から `spawn_blocking` なしで呼ばれる。
- `append_pending_rows_blocking` は同期の関数で、`fact_log::append_facts_for_events`（`fact_log.rs:525`、async）と `fact_log::append_single_fact`（同 `:995`、同期）から呼ばれる。完了事実（`execution_completed`）を含む行列は 1 回の追記で原子的に、それ以外は 1 行ずつ追記する。
- `append_single_fact` の非テストの呼び出しは、`ExecutionTreeArchiveFactRepository::append`（`workflow/execution_archive_repository.rs:63`、async）の `:89` と、debug desktop ビルド（`cfg(any(test, all(debug_assertions, feature = "desktop")))`）の `test_support::seed_workflow_session_facts`（`workflow/test_support.rs:21`、async）の `:87`・`:111`・`:124`。
- `append_facts_for_events` の非テストの呼び出しは `event_log_writer::append_required_events_for_app`（`workflow/event_log_writer.rs:14`、async）の `:22` の 1 か所。
- テストからの呼び出しは、3 つの止めて待つ入口が 22 か所（`local_event_store/node_events_test.rs` 7、`local_event_store/store_test.rs` 4、`workflow/startup_repository_test.rs` 3、`workflow/fact_log_test.rs` 2、`workflow/workflow_host.rs` の `:2196` から始まる `#[cfg(test)] mod` 2、`workflow/test_support.rs` の `#[cfg(test)]` 部分 1、`workflow/workflow_host_test.rs` 1、`workflow/runtime_command_gateway_test.rs` 1、`workflow/execution_projection_repository_test.rs` 1）、`append_pending_rows_blocking` が 11 か所、`append_single_fact` が 6 か所、`append_facts_for_events` が 7 か所。
- 正本が非テストの呼び出し元として挙げる `workflow_host.rs:4703`・`:4795`（#1881 の後は `:4758`・`:4855`）は、`workflow_host.rs:2196` から始まる `#[cfg(test)] mod` の中にある。`workflow_host.rs` の非テストの呼び出しは `append_events_at_head` の 1 か所だけである。正本が車線の判定として挙げる `writer.rs:888-893` は、現在の `writer.rs`（352 行）には無い。該当する判定は `writer.rs:115-120`。

## 呼び出し側の形

- 書き込みを使う port は既に async である。`WorkflowStartupRepository::append`（`domain/workflow/repository.rs:128`）、`ExecutionTreeArchiveRepository::archive` / `restore`（同 `:65`・`:71`）。`WorkflowEventRepository::append`（`usecase/workflow/ports.rs:19`）は `#[cfg(test)]` の同期メソッドで、canonical な実装は常に失敗を返す（`workflow/event_repository.rs:114`）。
- tokio の外から store へ書き込む経路は無い。CLI（`src-tauri/src/cli/`）から store へ書き込むのは `#[cfg(test)]` の test_support（`cli/common.rs:215`・`:262`）だけで、`releash workflow` / `review` / `hook` の本体は local API 経由で daemon へ渡す。`std::thread::spawn` から store へ書き込む箇所は無い。

## 失敗の分類

- 待ち行列が溢れたとき（`AdmitRejection::Capacity`）の分類が入口で違う。commit の入口は `CommitBatchError::CapacityExceeded`（`store.rs:906`・`:1008`）を返し、`FailureKind::Capacity`（`domain/local_event/batch.rs:147`）を経て `RESOURCE_EXHAUSTED`（`adaptor/protocol/connect.rs:38`）として観測される。node event の追記は `NodeEventWriteError::StorageUnavailable`（`store.rs:966`）を返し、`FailureKind::Temporary`（`writer.rs:82-86`）を経て `UNAVAILABLE`（`connect.rs:30`）として観測される。
- 読み込み側は混雑を `LocalEventQueryError::QueryBusy`（`local_event_store/reader.rs:393`）とし、`FailureKind::Temporary` を経て `UNAVAILABLE` として観測される。#1881 の R-004 がこれを固定している。
- commit の入口では、待ち行列が溢れた失敗と、batch が大きすぎる失敗（`MAX_BATCH_EVENTS` / `MAX_BATCH_STATE_MUTATIONS` / `MAX_BATCH_DECODED_BYTES` の超過、`store.rs:821`・`:866`・`:885`・`:895`）が同じ `CapacityExceeded` になる。node event の追記には大きさの検査が無く、待ち行列の byte 計算による受け入れ拒否だけがある。
- それ以外の書き込みの失敗の分類は入口で一致している。待ち行列が閉じている場合と返信が失われた場合は、どちらも `RestartRequired`（`CommitBatchError::OutcomeUnknown` は `batch.rs:144`、`NodeEventWriteError::OutcomeUnknown` は `writer.rs:82-86`）を経て `ABORTED` になる。writer thread での SQLite のエラーは、どちらも `reader::sqlite_failure_kind` で分類する（commit は `commit::storage_unavailable`、node event の追記は `store::node_append_error`、`store.rs:52`）。`expected_tree_head` の不一致は `NodeEventWriteError::Conflict`（`store.rs:724`）を経て `ABORTED` になる。
- `NodeEventWriteError` から上位のエラーへの変換は分類を保つ。`From<NodeEventWriteError> for WorkflowError`（`writer.rs:90-98`）が `WorkflowError::StorageUnavailable { kind }` にし、呼び出し元 3 か所（`fact_log.rs:1184-1192`、`startup_repository.rs:167-173`、`workflow_host.rs:373-381`）は `Conflict` だけを別扱いにして、残りは分類を持たせたまま返す。

# Scope / Non-goals

変更する。

- local event store の書き込みの入口を async の 1 つにすること。`append_node_event_blocking`、`append_node_events_blocking`、`append_node_events_at_head_blocking` と、`NodeEventAppendRequest` の `std::sync::mpsc` による返信を残さないことを含む。
- 止めて待つ書き込みを呼ぶ関数と、その呼び出し元を async にすること。同期の `fact_log::append_pending_rows_blocking` と `fact_log::append_single_fact`、`fact_log::reconcile_tree_pass` / `StoredWorkflowStartupRepository::append` / `WorkflowRuntimeHost::append_events_at_head` の内側の呼び出し、`ExecutionTreeArchiveFactRepository::append`、`event_log_writer::append_required_events_for_app`、debug desktop ビルドの `test_support::seed_workflow_session_facts`、およびテストからの呼び出しを含む。
- 待ち行列が溢れたことによる書き込みの失敗の分類を、入口によらず同じにすること。commit の入口の `CommitBatchError::CapacityExceeded` と node event の追記の `NodeEventWriteError::StorageUnavailable` の食い違いを残さない。
- 待ち行列が溢れたことによる失敗と、batch の大きさ上限を超えたことによる失敗を、別の分類として観測できるようにすること。現在この 2 つは commit の入口で同じ `CommitBatchError::CapacityExceeded` になる。
- store への書き込みの失敗を表す型を 1 つにすること。`NodeEventWriteError` を残さず、`CommitBatchError` が書き込みの失敗と、その分類を所有する。`From<NodeEventWriteError> for WorkflowError` と、`Conflict` だけを別扱いする呼び出し元 3 か所（`fact_log.rs`、`startup_repository.rs`、`workflow_host.rs`）の書き換えを含む。
- batch の大きさ上限の検証を 1 か所にすること。`store.rs` の 4 か所（`:818`・`:865`・`:884`・`:894`）に分かれた判定を残さない。上限値は変えない。
- 書き込みの失敗が呼び出し元へ届くまで #1880 の分類を保つこと。

変更しない。

- 読み込みの入口（#1881）。この変更は #1881 が async にした関数の上で書き込みを async にする。
- 書き込みの経路にある再試行（`Conflict` による再試行、結果が不明なときの `fact_log::resolve_unknown_append` による確認）と、起動時の再開処理の失敗の扱い（#1890）。`startup_repository.rs` については書き込みの async 化だけを行う。
- 書き込みの期限（#1893）。`WriteQueue` に期限を持たせない。
- 待ち行列の車線（normal / critical）の分け方と上限値、同時実行枠の優先度（#1894）。node event の追記が normal 車線に入ることを含む。
- `tokio-rusqlite` クレートの導入。従う標準は tokio-rusqlite の作りであり、クレートそのものではない。
- 書き込みの原子性の単位（どの行をひとまとめに追記するか）と、その単位を決める規則の所有。`fact_log::append_pending_rows_blocking` が `event_type == "execution_completed"` を含むかで分岐する形は、規則も所有者も変えない。この関数については async 化だけを行う。
- `expected_tree_head` による conflict 検出の条件、writer thread が実行する SQL、writer thread の本数。
- store を開くときに writer thread の外で行う書き込み（schema の初期化と evolve、起動時の maintenance、WAL の checkpoint）。writer の待ち行列を通らず、writer thread が動き出す前に実行される。
- Connect の contract（`proto/client.proto`）と HTTP local API の contract。

# Requirements

- R-001: local event store の書き込みの入口は async の 1 つである。全ての書き込みがその入口を通り、処理を止めて待つ書き込みの入口は残らない。
- R-002: 書き込みの結果を待っている間、呼び出し元は daemon の処理用スレッドを占有しない。ある書き込みが滞っていても、その間に届いた他の呼び出しは進む。
- R-003: writer の待ち行列が溢れたことによる書き込みの失敗は、どの書き込みの入口から観測しても `UNAVAILABLE` になる。
- R-004: 書き込みの失敗が呼び出し元へ届くとき、失敗の分類はその失敗が起きた理由に対応する。SQLite のエラーによる書き込みの失敗の分類は、そのエラーの種類だけで決まり、書き込みの経路によって変わらない。
- R-005: batch の大きさが上限を超えたことによる書き込みの失敗は `RESOURCE_EXHAUSTED` として観測される。

# Assumptions / Open Questions

- Assumption: 人間が受け入れた仮定は無い。
