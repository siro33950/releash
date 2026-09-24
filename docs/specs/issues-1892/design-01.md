# Design 01

## 開始状態

初回。直前の Design は無い。開始状態の実装は `docs/specs/issues-1892/requirements.md` の Current Behavior を参照する。

差分の基準は base `main` の `d2610dcb`、派生点は `feat/issues/1892` の作成時点。依存する #1880（`42be41e0`）と #1881（`b1fba8eb`）はどちらもこの基準に含まれる。

この周までに解消・見送りとなった Thread は無い（open Thread なし）。Spec 工程でコードを変更していないため、上記の基準がそのまま開始状態である。

## 変える部分

- 止めて待つ書き込みの入口の削除: `LocalEventStore::append_node_event_blocking` / `append_node_events_blocking` / `append_node_events_at_head_blocking`（`adaptor/gateway/local_event_store/store.rs:936`・`:945`・`:952`）と、`std::sync::mpsc::sync_channel(1)` による返信（同 `:957`）を残さない。根拠: R-001「local event store の書き込みの入口は async の 1 つである。全ての書き込みがその入口を通り、処理を止めて待つ書き込みの入口は残らない」、B-001。ルート: 「固定するルート」の writer の入口の形。
- writer の入口の形の置き換え: `WriteRequest` enum（`writer.rs:100`）と、writer thread 側の入口ごとの分岐（`store.rs:713` から始まる分岐）を残さず、writer 接続を受け取るクロージャを提出する形にする。車線の判定に使う critical と大きさの bytes はクロージャの外のフィールドに持つ。根拠: R-001、B-001。ルート: 「固定するルート」の writer の入口の形。
- 止めて待つ書き込みを呼ぶ関数と呼び出し元の async 化: 同期の `fact_log::append_pending_rows_blocking`（`workflow/fact_log.rs:575`）と `fact_log::append_single_fact`（同 `:995`）、および `fact_log::reconcile_tree_pass`（同 `:1171` の呼び出し）、`StoredWorkflowStartupRepository::append`（`workflow/startup_repository.rs:145` の呼び出し）、`WorkflowRuntimeHost::append_events_at_head`（`workflow/workflow_host.rs:355` の呼び出し）、`ExecutionTreeArchiveFactRepository::append`（`workflow/execution_archive_repository.rs:89`）、`event_log_writer::append_required_events_for_app`（`workflow/event_log_writer.rs:22`）、debug desktop ビルドの `test_support::seed_workflow_session_facts`、およびテストからの呼び出しを、async の入口を通す形にする。根拠: R-001、R-002「書き込みの結果を待っている間、呼び出し元は daemon の処理用スレッドを占有しない。ある書き込みが滞っていても、その間に届いた他の呼び出しは進む」、B-001、B-002。ルート: 委任
- 待ち行列が溢れた失敗の分類の統一: `AdmitRejection::Capacity` に由来する書き込みの失敗を、commit の入口（`store.rs:906`・`:1008` の `CommitBatchError::CapacityExceeded` → `FailureKind::Capacity`）と node event の追記（同 `:966` の `NodeEventWriteError::StorageUnavailable` → `FailureKind::Temporary`）で食い違わせず、どちらも `FailureKind::Temporary`（`UNAVAILABLE`）にする。根拠: R-003「writer の待ち行列が溢れたことによる書き込みの失敗は、どの書き込みの入口から観測しても `UNAVAILABLE` になる」、B-003。ルート: 委任
- store への書き込みの失敗の型の統合: `NodeEventWriteError`（`writer.rs:66`）を残さず、`CommitBatchError`（`domain/local_event/batch.rs:86`）が書き込みの失敗とその分類を所有する。`From<NodeEventWriteError> for WorkflowError`（`writer.rs:90`）と、`Conflict` だけを別扱いする呼び出し元 3 か所（`fact_log.rs:1184-1192`、`startup_repository.rs:167-173`、`workflow_host.rs:373-381`）の書き換えを含む。統合後も、SQLite のエラーによる失敗の分類が `reader::sqlite_failure_kind` の結果だけで決まり、書き込みの経路で変わらない状態を保つ。根拠: R-003、R-004「書き込みの失敗が呼び出し元へ届くとき、失敗の分類はその失敗が起きた理由に対応する。SQLite のエラーによる書き込みの失敗の分類は、そのエラーの種類だけで決まり、書き込みの経路によって変わらない」、R-005、B-003、B-004、B-005。ルート: 「固定するルート」の store への書き込みの失敗の分類の所有。
- batch の大きさ上限の検証の集約と分類: `store.rs` の 4 か所（`:818` の events / state_mutations の件数、`:865` の decoded_bytes、`:884` の node_events の件数、`:894` の decoded_bytes）に分かれた判定を 1 か所へ寄せ、上限の超過による失敗を `FailureKind::Capacity`（`RESOURCE_EXHAUSTED`）として、待ち行列が溢れた失敗と別の分類で観測できるようにする。上限値は変えない。根拠: R-005「batch の大きさが上限を超えたことによる書き込みの失敗は `RESOURCE_EXHAUSTED` として観測される」、B-005。ルート: 「固定するルート」の batch の大きさ上限の検証。

## 固定するルート

- 規則の所有（store への書き込みの失敗の分類）: `CommitBatchError`（`domain/local_event/batch.rs`）が書き込みの失敗と、その分類を所有する。`NodeEventWriteError`（`adaptor/gateway/local_event_store/writer.rs`）を残さず、node event の追記も `CommitBatchError` を返す。範囲は `NodeEventWriteError` の全 variant、`From<NodeEventWriteError> for WorkflowError`、および `Conflict` だけを別扱いする呼び出し元 3 か所（`fact_log.rs`、`startup_repository.rs`、`workflow_host.rs`）。粒度は所有者と対象箇所までの指定で、variant の構成と置き換えの手順は委任。理由は、今回直す食い違いが同じ概念を 2 つの型で表現していることから生じており、型を 2 つ残すと分類が domain（`batch.rs`）と gateway（`writer.rs`）の 2 か所で決まり続けて同じ食い違いが再発するため。`docs/architecture/DOMAIN.md`「規則は domain が所有する。形は概念による」「判断・計算・分類・検証・方針は domain にある」に従う。
- writer の入口の形: tokio-rusqlite のクロージャ形に従う。`WriteRequest` enum（Commit / NodeEventAppend）を残さず、writer 接続を受け取るクロージャと、車線の判定に使う critical および大きさの bytes をクロージャの外のフィールドとして持つ形にする。返信は各クロージャの中の `tokio::sync::oneshot`。writer thread は渡された関数を実行するだけとし、入口ごとの分岐（`store.rs:713` から始まる分岐）を残さない。範囲は `WriteQueue` への提出と writer thread の実行部分。粒度は形の指定までで、型名・関数名・型引数の取り方は委任。理由は、標準が定める形だから。tokio-rusqlite は `CallFn = Box<dyn FnOnce(&mut rusqlite::Connection) + Send>`（`lib.rs:158`）と event_loop の `Message::Execute(f) => f(&mut conn)`（同 `:412-415`）でこの形を定め、公開メソッドをすべて async にして同期の入口を持たない。#1881 は読み込みで同じ形（`ReaderPool::submit` と `ReadJob`、`reader.rs:374-417`）を採り、標準が定めない期限（`deadline_ms`）と待ち行列の上限（`READ_QUEUE_MAX_DEPTH`）をクロージャとは別のフィールドに持たせた。書き込みの車線と大きさも同じやり方で持てるため、車線の分け方と上限値は変わらない。
- batch の大きさ上限の検証: gateway（`adaptor/gateway/local_event_store`）が持つまま、`store.rs` の 4 か所（`:818` の events / state_mutations の件数、`:865` の decoded_bytes、`:884` の node_events の件数、`:894` の decoded_bytes）に分かれた判定を 1 か所へ寄せる。上限値は変えない。範囲はこの 4 か所と `append_node_events_at_head`。node 事実の追記も共通の検証を通し、4096 件または 16 MiB を超える要求をキュー投入前に `CapacityExceeded` として拒否する。粒度は所有する層と対象箇所までの指定で、寄せ先の関数の形は委任。理由は、待ち行列が溢れた失敗と分けるために 4 か所すべての分類を書き換えるため、同じ判定（decoded_bytes の比較が `:865` と `:894` の 2 回、`MAX_BATCH_EVENTS` の比較が `:818` と `:884` の 2 回）を残す理由がないこと。domain へ移す案を採らないのは、decoded_bytes が registry の encode 結果に依存する永続化の関心であり、閾値だけが domain へ移って規則が結局 2 か所に分かれるため。

上記 3 点以外の実装方法は委任。型・関数・モジュールの具体的な配置と命名、クロージャの型引数の取り方、async 化に伴う呼び出し元の書き換え手順、テストの構成を含む。

## 変えないもの

- 待ち行列の車線（normal / critical）の分け方と上限値。node event の追記が normal 車線に入ることを含む。理由は同時実行枠の優先度が #1894 の対象であるため。入口をクロージャ形にしても、車線の critical と大きさの bytes はクロージャの外のフィールドとして持つので、分け方も上限値も変わらない。
- batch の大きさ上限の値（`MAX_BATCH_EVENTS` 4096 / `MAX_BATCH_STATE_MUTATIONS` 8192 / `MAX_BATCH_DECODED_BYTES` 16 MiB）。4 か所に分かれた判定を 1 か所へ寄せ、node 事実の追記にも同じ上限を適用する。
- どの事実を原子的に追記するかの規則と、その所有者。`fact_log::append_pending_rows_blocking` が `event_type == "execution_completed"` を含むかで分岐する形はそのままとし、この関数については async 化だけを行う。理由は、正本 #1892 が挙げていない対象であり、事実の種類をドメインの値オブジェクトにすると `event_type` を文字列で扱う範囲全体へ波及して、この ISSUE のレビュー対象の境界が曖昧になるため。別途 ISSUE として報告する。
- 読み込みの入口（#1881）、書き込みの期限（#1893）、書き込みの経路の再試行と起動時の再開処理の失敗の扱い（#1890）。`startup_repository.rs` については書き込みの async 化だけを行う。
- `expected_tree_head` による conflict 検出の条件、writer thread が実行する SQL、writer thread の本数、store を開くときに writer thread の外で行う書き込み。Connect の contract と HTTP local API の contract。

## 未確定・リスク

- B-002 の「別の呼び出し」に別の書き込みが含まれる場合の判定範囲。writer thread の本数を 1 本のまま変えないため、ある書き込みが writer thread 上で滞っている間、後続の書き込みは writer thread の直列化により待つ。B-002 を満たすと判定する範囲を「呼び出し元が daemon の処理用スレッドを占有しないこと」（R-002 の第 1 文）と読み、writer thread 上の直列化を含まないものとして実装する。この読みが外れる場合、writer thread の本数が Non-goals にあるため、この周では B-002 を満たせない。
