# Design

## The actual design

### Architecture

#### 事実行 append の同期ポートを writer thread への blocking 委譲で満たす

事実行 append は呼び出し側から見て同期の port である。usecase の `PreparedWorkflowTransaction::persist` は同期の `FnOnce(&[WorkflowEvent]) -> Result<(), E>` を受け取り（`src-tauri/src/usecase/workflow/runtime_driver.rs:163-169`）、domain の `IsolatedWorktreeLedgerRepository::append` も同期 trait method である（`src-tauri/src/domain/workflow/repository.rs:21-26`）。この 2 つの契約が同期であることが、gateway 側に「tokio runtime を要さない同期 append」を用意する理由になる。

決定は次のとおり。

- `LocalEventStore` に node_events 単一行 append の blocking 入口を置く。writer thread への request 投入は現在と同じ `WriteQueue::admit` で行い、応答の受け取りだけを同期化する。
- `fact_log::append_pending_rows_blocking` は OS thread の spawn と tokio runtime の構築をやめ、呼び出し元の thread でこの blocking 入口を行ごとに呼ぶ。

この形が成立する根拠は既存実装にある。writer は tokio task ではなく独立した OS thread であり（`src-tauri/src/adaptor/gateway/local_event_store/store.rs:735-745` の `local-event-store-writer`）、呼び出し元が block しても応答側は前進する。読み出し側は同じ store に対して既に同一の形を採っており（`store.rs:1005` の `submit_indexed_query_blocking` → `local_event_store/reader.rs:1899`）、`append_facts_for_events` は同じ関数の中で runtime を作らずにその読み出しを呼んでいる（`adaptor/gateway/workflow/fact_log.rs:389-424`）。append 経路だけが runtime を必要としていたのは、reply channel が async 専用だったことによる。

#### reply channel を std の同期チャネルにする

`NodeEventAppendRequest.reply` は現在 `tokio::sync::oneshot::Sender` であり（`local_event_store/writer.rs:62`）、その blocking 受信は tokio runtime context の内側で panic する。事実行 append の async 呼び出し元はまさに runtime worker 上にある（`adaptor/gateway/workflow/event_log_writer.rs:39-45` の provider Stop 受理、`adaptor/gateway/workflow/workflow_host.rs:2119-2160` の commit）ため、oneshot のまま blocking 受信へ切り替えると R-003 を満たせない。

reply を reader pool と同じ std の同期チャネル（容量 1）へ変更する。std の受信は runtime context を判定せず panic しないため、同期文脈と async 文脈のどちらからでも同じ 1 つの入口が使える。送信側が drop された場合の受信失敗は、現在の oneshot 受信失敗と同じく `NodeEventWriteError::OutcomeUnknown` へ写す。

`WriteRequest::Commit` 側の reply は変更しない。`commit_batch_with_node_events` は async の production 呼び出し元を持つため（`adaptor/gateway/agent_session/agent_session_repository.rs:260`）、そのまま async として残す。

#### append の入口を blocking の 1 つに統一する

`LocalEventStore::append_node_event`（async、`store.rs:978`）の production 呼び出し元は `append_pending_rows_blocking` だけである。blocking 入口を追加したうえで async 版を残すと、同じ意味の append が 2 通りになる。async 版は削除し、既存の呼び出し（いずれもテスト）を blocking 入口へ寄せる。

#### panic 境界を再構築しない

現在は `std::thread::scope` の `join()` が append worker の panic を `node fact append worker panicked` という失敗へ畳んでいる。thread を作らなくなるとこの境界は消える。唯一の panic 源は store の write queue / reader pool 内部の mutex poisoning であり、同じ関数が呼ぶ読み出し経路には元から境界がない。`catch_unwind` を新設せず、読み出し経路と同じ扱いに揃える。

### Interface

- `LocalEventStore` の node_events append: 同期 method 1 つ。入出力は既存 async 版と同型（`NewNodeEventRow` と発生時刻の `Option<i64>` を受け、`Result<i64, NodeEventWriteError>` を返す）。async 版は削除する。
- `writer::NodeEventAppendRequest` は `reply` field の型だけを変更する。他の field と `WriteRequest` の variant 構成は変えない。
- `fact_log::append_facts_for_events` / `append_single_fact` / `append_pending_rows_blocking` のシグネチャは変えない。したがって呼び出し元（`workflow_host` の commit 経路、`event_log_writer`、`worktree_ledger_repository`、起動時 reconciliation）に変更は要らない。
- Tauri command、local API、CLI の外部契約は変更しない。

### Data Model

該当なし。`NewNodeEventRow` の field、`seq` と `timestamp_ms` の与え方、記録される事実の種類は変えない。

### Database

該当なし。同一 writer connection 上の同一の単一行 INSERT を実行する。access path の追加も schema 変更もない。

### UI/UX

該当なし。

### Algorithm

維持する不変条件と、それが本設計で成り立つ根拠だけを示す。

- 同一 node の事実行の並び（R-004 / B-007）: rows は 1 行ずつ順に投入し、応答を得てから次を投入する。writer は単一 thread で request を直列に処理するため、helper thread を挟まなくなっても同一 node の到着順は変わらない。
- 失敗時の記録単位（R-005 / B-008）: 途中の行が失敗した時点で残りを投入せず、呼び出し元へ失敗を返す。原子性は 1 行を超えない（`store.rs:973-977`）。
- 失敗分類: append 失敗は `NodeEventWriteError` のまま gateway 境界で文字列へ写し、`workflow_host` 側の `append_error_context` 合成（`workflow_host.rs:1497` ほか）と `settle_runtime_failure_for_node` の扱いは変えない。runtime 構築起因の文言（`failed to create fact append runtime: ...`）と worker panic 文言は発生源が消えるため無くなる。B-003 の「その文言が発生しない」はこの消滅で満たす。

### Infra

該当なし。プロセス起動時の fd soft limit は Non-goals であり、追加・変更・撤去する構成要素はない。

## Alternatives Considered

- 既存の tokio runtime handle を共有して `Handle::block_on` する（Issue が挙げた対応案の 1 つ）。`Handle::block_on` は runtime worker thread 上で panic する。事実行 append の async 呼び出し元（`event_log_writer.rs:45`、`workflow_host.rs:2157`）はまさにその位置にあるため R-003 を満たせない。`workflow_host.rs:1719` に同種の呼び出しがあるが、あれは `spawn_blocking` の内側で blocking region に入っており条件が異なる。
- append 経路全体を async にする。usecase の `persist` が同期 `FnOnce`、domain の `IsolatedWorktreeLedgerRepository::append` が同期 trait method であるため、usecase と domain の契約変更を伴う。さらに `workflow_host.rs:2119` は「同期 append を跨いで executions mutex を保持する」ことを明示した設計であり、その境界も作り直しになる。Requirements にない変更範囲へ広がる。
- reply を tokio oneshot のまま保ち、外部 executor の `block_on` で待つ。`futures-executor` を直接依存へ追加することになり、同じ store の読み出し経路（`adaptor/gateway/local_event_store/reader.rs:1904` の `mpsc::sync_channel(1)`）と別パターンになる。得られるのは reply field の型を変えずに済むことだけで、割に合わない。

## Cross-cutting concerns

- 性能: append 1 回あたりの OS thread spawn と runtime 構築が無くなる。呼び出し元が block する区間は writer の応答待ちだけになり、block する thread の本数は変わらない（現行も呼び出し元 thread が `join()` で block していた）。
- 可観測性: append 失敗の文言が `node fact append failed: {error}` に収束する。writer 側の `node event append failed [{correlation}]` ログ（`store.rs:771`）は変えない。
- 検証: B-001 と B-002 の「fd が増えない」は、隔離した子プロセス内で writer を INSERT 直前に停止し、呼び出し元が応答待ちに入った append 実行中の open fd 数を追記前と比較して確認する。単発は writer の停止位置への到達通知、並行はその到達通知と write queue に残り N-1 本が滞留したことの双方により、全 append が in-flight であることを確定する。完了後の open fd 数も追記前と比較する。数え方は `/dev/fd` の列挙で、macOS と CI の Linux（`/proc/self/fd` への symlink）の双方で同じ経路が使える。B-003 は同じ子プロセス隔離下で store を open して warm-up append を済ませ、現在の open fd 数に 2 個の余裕だけを残す値まで `RLIMIT_NOFILE` の soft limit を下げて確認する。limit を下げた後は `/dev/fd` を列挙せず、`session_attached` の append が成功して既存 reader から同じ事実行を読めることを確認する。rlimit の変更は親プロセスおよび他テストへ波及しない。B-005 と B-006 は、async runtime 上（`#[tokio::test]`）から append を呼んで panic せず結果が返ることで確認する。

## Risks

- blocking 受信が前進する根拠は「writer が独立した OS thread であり、その応答が tokio task の進行に依存しない」ことに尽きる。実装で node_events append の応答経路に tokio task を挟むと、current-thread runtime や worker を塞いだ状態で deadlock し、B-004 から B-006 を満たせなくなる。
- fd 数の観測を行う子プロセスには対象テストだけを指定し、テスト thread 数も 1 に固定する。writer の停止中に到達通知と queue 滞留数で in-flight を確定してから計数するため、他テストの fd 増減や append 完了タイミングには依存しない。
