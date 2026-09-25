# Context

- 正本: [#1893 `[04] 期限を処理の先（store・外部プロセス）へ引き継ぐ`](https://github.com/siro33950/releash/issues/1893)
- 所属: [milestone #99「02. UI と daemon の間の通信の仕組みを一本化する」](https://github.com/siro33950/releash/milestone/99) の Wave 04。milestone は個別の問題として「daemon 側に、呼び出しごとの制限時間も、UI が呼び出しをやめたときの取り消しも無い。終わらない処理が同時実行の枠を握り続ける」を挙げ、通信の土台になる仕組みを 1 か所にまとめることを方針に置く。
- 従う標準は [gRPC Deadlines](https://grpc.io/docs/guides/deadlines/) の deadline propagation。呼び出しに付いた期限は絶対期限として処理の先へ引き継がれ、期限を超えた処理は続けず `DEADLINE_EXCEEDED`、呼び出し側の中断は `CANCELLED` で終わる。
- 標準が定める期限の合成は置き換えではなく、先に来る方を採る形である。Go の `context` は「If the parent's deadline is already earlier than d, `WithDeadline(parent, d)` is semantically equivalent to parent」とし、grpc-java の [`Deadline`](https://grpc.github.io/grpc-java/javadoc/io/grpc/Deadline.html) は `minimum(Deadline other)` を持つ。資源側が持つ期限（HTTP client の timeout 等）は削除されず、引き継いだ期限と併存する。
- 標準が定める取り消しは協調的である。[Cancellation](https://grpc.io/docs/guides/cancellation/) は「the gRPC library in general does not have a mechanism to interrupt the application-provided server handler, so the server handler must coordinate with the gRPC library to ensure that local processing of the request ceases」「If an RPC is long-lived, its server handler must periodically check if the RPC it is servicing has been cancelled and if it has, cease processing」とする。
- 標準は取り消しの前に行われた変更を戻さない。[Core Concepts](https://grpc.io/docs/what-is-grpc/core-concepts/) が「Changes made before a cancellation are not rolled back」とする。
- 標準は呼び出しから始まらない処理に既定の期限を置かない。Go の `context.Background()` は「never canceled, has no values, and has no deadline」であり、背景処理は自らの期限を設定する。
- 最初の周の調査基準は branch `feat/issues/1893` の HEAD `9194bd57`。正本の「今の作り」の表は `42be41e0` 時点の記載であり、以後 #1881（`b1fba8eb`）・#1883（`d2610dcb`）・#1892（`952c8df2`）・#1885（`9194bd57`）が merge されている。表の各項目は `9194bd57` でも成立することを確認した（Current Behavior に記載）。正本の示す行番号は HEAD と一致しないため、対象はシンボル名で特定する。
- アーキテクチャの正本は `AGENTS.md`。アプリケーションロジックは daemon（サーバ）が所有する。永続化は event store であり、full-retention / full-recompute の経路を増やさない。domain 層の規約は `docs/architecture/DOMAIN.md`。domain 配下で `tokio` 等を `use` せず、外側の能力は port として宣言する。
- 依存する #1880（`42be41e0`）により、失敗の分類は `FailureKind`（`src-tauri/src/domain/failure.rs`）が持ち、Connect のステータスコードへの対応付けは `src-tauri/src/adaptor/protocol/connect.rs` の 1 か所で行う。`FailureKind::Expired` が `DEADLINE_EXCEEDED`、`FailureKind::Cancelled` が `Canceled` に対応する。
- 依存する #1883（`d2610dcb`）により、単発の呼び出しには daemon 既定 120 秒の期限が当たり、期限切れ・中断で呼び出しが終わると同時実行の枠が解放される。取り消しは `ClientApiDeps::execute` の中の `tokio_util::sync::CancellationToken` と `drop_guard` が持つ。#1883 は「既に同期処理として動き始めた処理の内側で取り消しを受け取れるようにすること」と「期限を store と外部プロセスへ引き継ぐこと」を本 ISSUE へ残している。
- 依存する #1881（`b1fba8eb`）・#1892（`952c8df2`）により、store の読み込みの入口は `ReaderPool::submit`、書き込みの入口は async の 1 つに揃っている。本 ISSUE は入口の形を変えない。
- 同じ milestone の隣接 ISSUE。daemon の中の繰り返し処理が使う期限と再試行は [#1890](https://github.com/siro33950/releash/issues/1890)、同時実行の枠の優先度は [#1894](https://github.com/siro33950/releash/issues/1894) が担う。
- 各資源が持つ中断の口。外部プロセスはプロセスの終了、SQLite の実行中の文は [`sqlite3_interrupt()`](https://sqlite.org/c3ref/interrupt.html)（rusqlite 0.40 の `InterruptHandle`）、git2 の checkout は `CheckoutBuilder::notify`、git2 の fetch は `RemoteCallbacks::transfer_progress`、ファイル lock の待ちは `try_lock_exclusive` の繰り返し。git2 の diff / status は口を持たない（libgit2 の統一 cancellation は [libgit2#3334](https://github.com/libgit2/libgit2/issues/3334) で未実装）。
- 参照する既存実装: `src-tauri/src/adaptor/controller/api/client.rs`、`src-tauri/src/adaptor/gateway/local_event_store/{reader,connection,store}.rs`、`src-tauri/src/adaptor/gateway/git_host/github.rs`、`src-tauri/src/adaptor/gateway/notion/service_impl.rs`、`src-tauri/src/adaptor/gateway/code/staging.rs`、`src-tauri/src/adaptor/gateway/comment/mod.rs`、`src-tauri/src/adaptor/gateway/repository/worktree_operation.rs`、`src-tauri/src/usecase/worktree_operation.rs`、`src-tauri/src/domain/repository/worktree_operation.rs`、`src-tauri/src/domain/git_host/value_objects/cache.rs`。

# Outcome

対象者は、Releash の UI を使う利用者と、daemon を実装・保守する開発者である。

現在、呼び出しに付いた期限は呼び出しの入口までしか効かない。処理の先にある store の問い合わせ、外部プロセスの実行、lock の待ちは、それぞれ無関係な独自の期限を持つか、期限を持たない。期限の無い待ちは、相手が応答しない限り終わらない。呼び出しが期限切れや中断で終わっても、`spawn_blocking` の上で始まった同期処理は取り消しを受け取らず、最後まで走り切る。利用者から見ると、応答が返らない操作があり、やめた操作の後始末が残る。

変更後は、呼び出しに付いた期限が処理の先まで引き継がれる。store の問い合わせ、外部プロセスの実行、lock の待ちは、いずれも呼び出しの期限を超えて続かない。処理の先が自分の期限を持つ場合は、先に来る方でその処理が終わる。呼び出しの期限切れと中断は同期処理の内側まで届き、処理は止まって、確保していた外部プロセスと lock を残さない。

# Current Behavior

`9194bd57` のコードで確認した挙動である。

## 呼び出しの期限は入口までしか効かない

- 単発の呼び出しのハンドラは build script が生成し（`src-tauri/build.rs`）、手書き分は `src-tauri/src/adaptor/controller/api/client_service.rs` にある。いずれも `connectrpc::RequestContext` を `_ctx` として受け取り、使わずに捨てている。期限の値を読む口はあるが、読んでいない。
- 期限は router の `DeadlinePolicy`（既定 120 秒、`client.rs` の `router`）が持ち、connectrpc が handler の future ごと打ち切る形でだけ効く。期限の値は処理の先へ渡らない。
- `ClientApiDeps::execute` は `CancellationToken` を作り、`drop_guard` を握ったうえで、処理を `tokio::spawn` で切り離して `run_command` に包む。`run_command` は `run_until_cancelled` で future を包むだけで、token は `dispatch_admitted` が返す future の先へ渡らない。
- command の実処理の多くは `spawn_blocking` の上の同期呼び出しである（`src-tauri/src/adaptor/controller/client/` の各 module）。future が drop されても、`spawn_blocking` の task は走り続ける。

## 処理の先は独自の期限を持つか、期限を持たない

- store の読み込み: `ReaderPool::submit` が `QUERY_DEADLINE_MS`（2 秒）から絶対期限を作って job に持たせる（`local_event_store/reader.rs`）。判定は `run_worker` が job を取り出した時点の 1 回だけで、走り始めた問い合わせと、`receiver.await` で待つ呼び出し側には期限が無い。reader は専用スレッド 4 本（`READER_POOL_SIZE`）、待ち行列は 128 件（`READ_QUEUE_MAX_DEPTH`）で、溢れると `QueryBusy` になる。
- SQLite の busy_timeout: 接続の設定で 2 秒（`local_event_store/connection.rs` の `configure_common`）。
- store の書き込みの返事の待ち: `LocalEventStore` の書き込みは `queue.admit(job)` で受け付けを確定させたあと `receiver.await` で `oneshot` を待つ（`local_event_store/store.rs`）。期限は無い。呼び出し側が await をやめても job は待ち行列に残り、writer スレッドは実行する。`drain_and_close` の規約は「Stop new writes, persist every request already admitted to the writer, then join all store workers」であり、admit 済みは永続化される。
- `gh`: `GH_TIMEOUT`（10 秒）。`try_wait` を 100 ms 間隔で繰り返し、超過で `kill` する（`git_host/github.rs`）。
- Notion API: `REQUEST_TIMEOUT`（10 秒）を `reqwest::blocking::Client` に設定。429 のときは `Retry-After`（上限 60 秒）の間 `std::thread::sleep` で待ち、`MAX_RETRIES`（2 回）まで繰り返す（`notion/service_impl.rs` の `send_with_retry`）。
- `git apply --cached` / `--reverse`: `Command::spawn` のあと `wait_with_output` で待つ（`code/staging.rs` の `git_stage_hunk` / `git_unstage_hunk`）。期限は無い。
- git2（libgit2）の各操作: 期限は無い。`git2::` を使う非テストの module は `adaptor/gateway/repository/`（`util`、`branch`、`status`、`branch_card`、`worktree`、`git_config`、`watch`）、`adaptor/gateway/code/`（`staging`、`branch_diff`、`mod`、`file_content`、`diff_compute`）、`adaptor/gateway/git_host/`（`discovery`）、`adaptor/gateway/workflow/`（`worktree_gateway`）、`infrastructure/git/`（`helpers`、`client`）にある。
- review comment のファイル lock: `try_lock_exclusive` を 10 ms 間隔で繰り返し、10 秒で諦める（`comment/mod.rs`）。
- worktree の registry のファイル lock: `FileWorktreeOperationLocks::registry_lock` が `fs2::FileExt::lock_exclusive` で止めて待つ。期限は無い（`repository/worktree_operation.rs`）。
- worktree の削除が、変更の操作の終わりを待つ: `FileWorktreeOperationLocks::deletion` が `try_lock_exclusive` を 10 ms 間隔で繰り返し、`WorktreeOperations::delete_many` が `ready_to_delete` になるまで `notified()` を待つ（`usecase/worktree_operation.rs`）。いずれも期限は無い。状態遷移そのものは `domain/repository/worktree_operation.rs` の `WorktreeOperationState` が所有する。

## 期限の表現が 5 通りに分かれている

`reader.rs` は `deadline_ms: i64`（`StoreClock::now_ms()` 基準）、`github.rs` は `GH_TIMEOUT: Duration` と `start.elapsed()`（`Instant` 基準）、`notion/service_impl.rs` は `REQUEST_TIMEOUT` を `reqwest` へ委ね、`comment/mod.rs` は `Duration::from_secs(10)` をループ条件に直書きし、`client.rs` は connectrpc の `DeadlinePolicy` を使う。期限の合成と期限切れの判定を所有する型は無い。

## 同じ処理の先を、呼び出し以外の経路も使う

- workflow の runtime、daemon の中の繰り返し処理、CLI / hook 用の HTTP local API（`adaptor/controller/api/workflow.rs`、`provider_lifecycle.rs`）が、同じ store・外部プロセス・lock を使う。これらの経路には呼び出しの期限が無く、上記の独自の期限と期限の無い待ちが、そのまま唯一の上限になっている。

# Scope / Non-goals

今回変更する対象。

- 呼び出しに付いた期限の、処理の先への引き継ぎ。対象は Current Behavior に挙げた各所（store の読み込み、SQLite の busy_timeout、store の書き込みの返事の待ち、`gh`、Notion API、`git apply --cached` / `--reverse`、git2 の各操作、review comment のファイル lock、worktree の registry のファイル lock、worktree の削除が変更の操作の終わりを待つ処理）。
- 呼び出しの取り消し（期限切れ・client の中断）の、処理の先への引き継ぎ。`spawn_blocking` の上で始まった同期処理が止まれるようにすることを含む。
- 引き継いだ期限と、処理の先が持つ期限の合成。
- 期限切れ・中断で止まった処理が確保していた外部プロセスとファイルの lock の後始末。
- 引き継いだ期限の超過と取り消しの、失敗の分類。
- 期限の合成と期限切れ・残り時間の判定を所有する、期限の表現。
- 期限付きでファイル lock を待つ処理の共通化。対象は `comment/mod.rs`、`repository/worktree_operation.rs` の `registry_lock` と `deletion` の 3 箇所。

今回変更しない対象。

- 期限と取り消しを `execute` で受け取る仕組み（#1883）。既定の期限の値（120 秒）と、client が指定した期限の扱いを含む。
- store の入口の形（#1881・#1892）。
- 処理の先が現在持つ期限の値（store の読み込みの 2 秒、SQLite の busy_timeout の 2 秒、`gh` の 10 秒、Notion API の 10 秒、review comment のファイル lock の 10 秒）。
- 呼び出しの期限が引き継がれない経路に、新しい期限を置くこと。
- daemon の中の繰り返し処理が使う期限（#1890）。
- workflow の Command Node と agent の実行の期限（`adaptor/gateway/workflow/failure_policy_config.rs` の stale timeout など）。
- 再試行（#1890）。Notion API の `Retry-After` による再試行の回数と条件を含む。
- 同時実行の枠の数と優先度（#1894）。reader プールのスレッド数と待ち行列の深さを含む。
- 購読の stream を開いた後の期限（#1883 で対象外と決定済み）。
- CLI / hook 用の HTTP local API が呼び出しの期限を受け取る仕組み（#1883 で対象外と決定済み）。
- 中断の口を持たない操作に、口を作ること。git2 の diff / status を別プロセスへ切り出すことを含む。

# Requirements

- R-001: 呼び出しに付いた期限は、その呼び出しから始まる store の問い合わせ、外部プロセスの実行、lock の待ちへ引き継がれ、いずれもその期限を超えて続かない。
- R-002: 呼び出しの期限切れと client の中断は、その呼び出しから始まった同期処理の内側まで届く。その処理は、実行中の操作を中断できる場合は中断して止まり、中断できない場合はその操作が終わった時点で止まる。いずれも次の操作へ進まない。
- R-003: 引き継いだ期限の超過で止まった処理は期限切れ、取り消しで止まった処理は中断として分類され、呼び出しはそれぞれ `DEADLINE_EXCEEDED` / `CANCELLED` で終わる。
- R-004: 呼び出しから始まる処理は、引き継いだ期限と、その処理の先が持つ期限のうち、先に来る方で終わる。
- R-005: 引き継いだ期限の超過または取り消しで止まった処理は、そのとき確保していた外部プロセスとファイルの lock を残さない。
- R-006: 呼び出しの期限が引き継がれない経路から store・外部プロセス・lock を使うとき、その処理の先が持つ期限がそのまま上限になる。
- R-007: 既に store の待ち行列へ受け付けられた書き込みは、その呼び出しが期限切れまたは中断で終わっても保存される。

# Assumptions / Open Questions

- 期限の合成、期限切れの判定、残り時間の計算は、domain が所有する期限の表現に置く。
- 期限切れと取り消しのどちらで止まるかの判断は、期限と取り消しを組にした 1 つの値が所有し、処理の先へはその値を渡す。取り消しは domain の port として宣言し、adaptor が実装する。
- 失敗の分類は #1880 の `FailureKind` が引き続き所有し、新しい分類を追加しない。
- 自動判断: Current Behavior の git2 の module の列挙を `9194bd57` の実装に合わせて補正した。正本の記載に無い `repository/git_config`、`repository/watch`、`code/diff_compute`、`git_host/discovery`、`workflow/worktree_gateway` が非テストの経路で `git2::` を使う。
