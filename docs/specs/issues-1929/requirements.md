# Context

- 正本: [#1929 `[05] 失敗を業務の失敗と技術的な失敗に分ける`](https://github.com/siro33950/releash/issues/1929)
- 所属: [milestone #99「02. UI と daemon の間の通信の仕組みを一本化する」](https://github.com/siro33950/releash/milestone/99) の Wave 05。milestone の「層の整理（規約 `docs/architecture/` に合わせる。対象はサーバのコードだけ）」の 4 件（#1928・#1929・#1930・#1931）の 2 番目である。
- 規約の正本は `docs/architecture/`。この変更が根拠にする規定は次のとおり。
  - `DOMAIN.md`「失敗」: 「domain が表す失敗は業務の意味があるものだけである。不変条件と検証の違反、今の状態では受け付けない操作、対象が無い、集約の版の競合などがこれに当たる」。「trait の失敗は、業務の結果と、中身を見ない技術的な失敗の変種 1 つで表す。技術的な失敗が一時的か、やり直してよいかは、失敗を生んだ外側が扱う。domain はそれを判断に使わない」。「transport の語彙（gRPC のステータスコード、HTTP status 等）も、それと 1 対 1 に対応する別名の語彙も、domain に持ち込まない」。
  - `DOMAIN.md`「モデルは実行経路にある」: 移行の途中で一時的に規則から外れる場合、いつ・何によって解消するかを明示する。
  - `USECASE.md`「失敗」: 「業務の手順をやり直すか（集約の版が競合したら読み直して再実行する等）は Usecase が決める。判断に使うのは domain の失敗が持つ業務の意味である」。「技術的な失敗は、中身で分類し直さずにそのまま出す。失敗を文字列へ変換して落とさない」。
  - `PRESENTER.md`「失敗」: 「Usecase が出した失敗を、転送の失敗（Connect のステータスコード、HTTP status）へ対応付けるのは presenter の 1 か所だけである。業務の失敗と技術的な失敗を区別して対応付ける」。「転送の失敗をその外で組み立てない。失敗を文字列にして転送の失敗にしない」。
  - `PRESENTER.md`: Output Data を転送の形に変えるのは presenter である。
- 依存: #1928。`635ee8d6` で merge 済みである。#1928 が残した前提は次のとおり（`docs/specs/issues-1928/requirements.md`、同 `behavior.md`）。
  - 期限切れ・取り消しを表す `Stopped` の変種は domain と usecase の失敗の型から無くなり、出ていく側の自前の期限切れは gateway が技術的な失敗の変種に `FailureKind::Expired` を持たせて返す。
  - 出ていく側の自前の期限切れを最終的にどのステータスコードで返すかは #1928 の対象外であり、この変更が決める。
  - `FailureKind::retry_action()` を業務の失敗（版の競合）へ置き換えることは #1928 の対象外であり、この変更が扱う。
  - `AppError` は中身（`FailureKind` と `Serialize` でのフロントへの表現）を変えずに `src-tauri/src/adaptor/presenter/error.rs` へ移った。
- 基準は `main` の `177fdf5d`（#1932 の実装 PR #1934）である。作業ブランチ `feat/issues/1929` の派生元 `635ee8d6` には、#1932 が足した delegate の注入済みの記録のやり直し（`src-tauri/src/usecase/workflow/delegate.rs:69`）がまだ無い。Current Behavior は `177fdf5d` で確認した。`src-tauri/src/other/` は既に無く、`AppError` は `src-tauri/src/adaptor/presenter/error.rs` にある。
- AIP-193（https://google.aip.dev/193）は、`google.rpc.Status` と `google.rpc.Code` の正規コードを使うこと、`ErrorInfo` の `reason` / `domain` による分類を定める。依存先の期限切れにどのステータスコードを使うかは定めていない。

# Outcome

対象者は、daemon を実装・保守する開発者である。Releash の画面と CLI を使う利用者は、この変更の前後で同じステータスコードを見る。

現在、domain に、gRPC のステータスコードと 1 対 1 に対応する 16 値の失敗の分類と、それを返す trait がある。転送のステータスコードは、この分類を失敗に付ける各所で実質的に決まっており、adaptor の 26 ファイル 104 行（うち controller 24 ファイル）が分類を付けている。やり直すかどうかも同じ分類から導かれ、「やり直しで直る失敗は Node の失敗にしない」という同じ規則が 2 か所で別の形に書かれ、store 由来の失敗で結果が食い違う。store の失敗の意味も、store 自身の変種とこの分類の 2 か所で表されている。「対象が今『要対応』か」も、失敗の記録と対象ごとの状態の 2 か所で保持され、答えがずれる。分類はそのまま画面の表示文字列になっている。

変更後は、domain の失敗が業務の意味だけを表し、trait の失敗は業務の結果と、中身を見ない技術的な失敗の変種 1 つで表される。転送のステータスコードへの対応付けは presenter の 1 か所にある。業務の手順をやり直すかは集約の版の競合という業務の失敗で決まり、一時的な失敗をやり直すかは技術的な失敗が持つ性質で決まる。「やり直しで直る失敗は Node の失敗にしない」の判定は 1 つになる。store の失敗の意味は store 自身の変種だけが持つ。「要対応」の判定は失敗の記録だけから導かれ、二つ目の表現は無くなる。画面へ出る分類の文字列は presenter が決める。画面と CLI が受け取るステータスコードは変わらない。

# Current Behavior

`177fdf5d` のコードで確認した挙動である。

## 転送のステータスコードと 1 対 1 の分類が domain にある

- `src-tauri/src/domain/failure.rs` に `FailureKind`（`Temporary` / `RestartRequired` / `StateRequired` / `InvalidInput` / `Expired` / `Missing` / `AlreadyPresent` / `Permission` / `Capacity` / `Unsupported` / `Internal` / `Corrupt` / `Cancelled` / `Unknown` / `OutsideRange` / `AuthenticationRequired` の 16 値）、それを返す trait `ClassifiedFailure`、`RetryAction`（`Stop` / `Retry` / `Restart`）、`retry_action()`、`requires_attention()`、`TargetFailures`、`BackgroundFailures`、`TechnicalFailure`（`kind: FailureKind` と `message`）がある。
- 参照ファイル数は、`*_test.rs` と `test_helpers*.rs` を除いて数えると、`FailureKind` が domain 35・usecase 28・adaptor 41、`ClassifiedFailure` が domain 21・usecase 32・adaptor 22 である。`ClassifiedFailure` を実装する型を持つファイルは domain 21・usecase 23・adaptor 4 である。
- `src-tauri/src/adaptor/protocol/connect.rs:26` の `code()` が `FailureKind` の 16 値を Connect のステータスコードへ 1 対 1 で対応付ける。同ファイル `:84` は `ConnectError` から `FailureKind` への逆変換も持つ。
- `src-tauri/src/adaptor/presenter/error.rs` の `AppError` が `FailureKind` を持ち、`ClassifiedFailure` を実装し、フロントへの serialize 表現（通常はメッセージのプレーン文字列、`Coded` は `code` と `message` の object）を決める。`AppError` に分類を付ける記述（`from_failure` / `with_failure_kind` / `coded`）は同じ数え方で 26 ファイル・104 行にあり、内訳は adaptor/controller 24 ファイル・adaptor/presenter 1 ファイル・adaptor/protocol 1 ファイルである。
- HTTP local API の失敗は `src-tauri/src/adaptor/controller/api/error.rs` の `ApiError` が HTTP status へ変える。対応付けは `FailureKind` ではなく `WorkflowError` の変種に対して書かれている（`Store` と `StorageUnavailable` が 503、`Validation` が 400、`Conflict` と `InvalidState` が 409、`NotFound` が 404、`UnauthorizedApprovalTarget` が 403、`Technical` / `Editor` / `External` / `CorruptStoredState` / `IncompatibleStoredEvent` が 500）。
- domain の中で、業務の語彙の上に `FailureKind` が重なっている箇所がある。`src-tauri/src/domain/comment/mod.rs:342-345` は `ReviewError::Technical` の `kind` が `Expired` か `Cancelled` かで `ReviewErrorCode` を決める。`src-tauri/src/domain/local_event/failure.rs:56` の `SafeOperationFailure` は `classification: FailureKind` を持ち、`Display` が `retryable=`（`Temporary` かどうか）を出す。

## 失敗の分類がそのまま画面の表示になっている

- `src-tauri/src/adaptor/controller/api/state_subscription.rs:24` が `classification: Some(format!("{:?}", record.kind))` として、`FailureKind` の Debug 表記を画面へ送る。
- `proto/client.proto:3159` の `FailureRecord.classification` は `string` で `json_required` である。
- 画面はこの文字列をそのまま描く。`src/components/workflow/BackgroundFailures.tsx:31` が `{record.operation} · {record.classification}` として出し、`:27` が React の key にも使う。文字列を解釈するコードは `src/` に無い。
- `src-tauri/src/domain/failure_records.rs:38-40` の `observe` は `(operation, target, kind)` が一致する記録を同じものとして数える。分類が 16 値あるため、同じ対象・同じ操作でも分類が違えば別の行になる。

## 画面・CLI・Tauri のシェルが見ている失敗

- 画面は `src/lib/client.ts:471` で Connect のステータスコード `ResourceExhausted` を見て、1 秒後に購読を開き直す。この失敗の発生元は `src-tauri/src/adaptor/controller/api/client.rs:119` で、同時に処理できる呼び出しの枠が埋まったときに `AppError::coded("CLIENT_REQUEST_LIMIT", ..., FailureKind::Capacity)` を返す。
- 画面は `src/components/panels/useDiffOperations.ts:25` で、`AppError` の serialize 表現の `code` が `STALE_REVIEW_GROUP_TARGET` かどうかを見る。
- CLI は `src-tauri/src/cli/api_client.rs:193-199` で HTTP status を読み替える（404 が `NotFound`、400 / 409 / 422 が `InvalidInput`、401 が認証の失敗、それ以外がその他）。応答の body の `code` 文字列は読まない。
- Tauri のシェルは `src-tauri/src/adaptor/gateway/desktop_client.rs:57` で、5 秒ごとの生存確認が `FailureKind::Capacity` で失敗したときだけ監視を続け、それ以外の失敗では daemon が落ちたと見なして監視を止める。

## store の失敗の意味が 2 つの場所で表されている

- `src-tauri/src/domain/local_event/batch.rs:159` の `CommitBatchError::failure_kind()` と `src-tauri/src/domain/local_event/query.rs:80` の `LocalEventQueryError::failure_kind()` が、変種を `FailureKind` へ潰す。`StreamHeadConflict` / `OutcomeUnknown` / `TreeHeadConflict` / `AppendOutcomeUnknown` は `RestartRequired`、`QueueBusy` は `Temporary`、`CapacityExceeded` / `SequenceExhausted` は `Capacity`、`Corrupt` は `Corrupt`、`PayloadConflict` は `StateRequired` になる。
- 潰した結果は変種を失ったまま運ばれる。`src-tauri/src/usecase/workflow/runtime_error.rs:10-14` の `Store(FailureKind)` と `StorageFailure { kind, message }`、`src-tauri/src/domain/workflow/error.rs:10`・`:13-16` の `Store(FailureKind)` と `StorageUnavailable { message, kind }` である。
- `src-tauri/src/domain/workflow/error.rs:94` がその `kind` をそのまま返すため、`WorkflowError::Store(kind)` は `kind` 次第で `UNAVAILABLE` / `DATA_LOSS` / `FAILED_PRECONDITION` / `ABORTED` / `RESOURCE_EXHAUSTED` / `INTERNAL` のどれにもなる。
- `WorkflowRuntimeError` は `src-tauri/src/adaptor/gateway/workflow/runtime_command_gateway.rs:93-100` と `src-tauri/src/usecase/workflow/control_plane.rs:747-756` で `WorkflowError` へ変換され、presenter と CLI に届く。
- `src-tauri/src/domain/local_event/failure.rs:54` の `SafeOperationFailure` は `SessionOperationFailureKind`（`StorageUnavailable` / `PersistFailure` / `OutcomeUnknown`）と別軸で `classification: FailureKind` を持つ。その値は `src-tauri/src/adaptor/gateway/local_event_store/reader.rs:43` の `sqlite_failure_kind()` が rusqlite のエラーコードから決める。
- store 由来の版の競合が業務の失敗になる経路と、ならない経路がある。`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:378` は `CommitBatchError::TreeHeadConflict` を `WorkflowRuntimeError::Conflict` にするが、同 `:366-374` の読み直しの失敗は `TreeHeadConflict` でも `StorageFailure { kind }` になる。
- store 以外の技術的な失敗も `Store` / `StorageFailure` へ変換されている箇所がある。`src-tauri/src/usecase/workflow/execution_archive.rs`、`src-tauri/src/usecase/workflow/startup.rs`、`src-tauri/src/usecase/workflow/node_startup.rs` である。
- `src-tauri/src/cli/file_direct.rs:67-81` の `workflow_error_to_cli_error` は `WorkflowError` を網羅的に match する。

## やり直すかどうかが `FailureKind` から導かれている

- `FailureKind::retry_action()` は `Temporary` を `Retry`、`RestartRequired` を `Restart`、それ以外を `Stop` とする。
- 業務の手順のやり直しは usecase にある。`src-tauri/src/usecase/workflow/control_plane.rs:108` と `:126` が `retry_stage`、`src-tauri/src/usecase/workflow/node_startup.rs:83` が `run_borrowed`、`src-tauri/src/usecase/workflow/command/mod.rs:31` の `retry_control_plane_operation` が `retry` を呼ぶ。`src-tauri/src/usecase/workflow/delegate.rs:69` は `work_queue::retry` を `RetryBackoff::CONFLICT` で呼ぶ。`retry_control_plane_operation` の非テストの呼び出しは `command/mod.rs:28` と `src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:266`・`:2098` である。
- 実際にやり直すかは `src-tauri/src/usecase/work_queue.rs:291`・`:308`・`:310`・`:321` が `error.kind.retry_action()` で決める。同 `:628` の `retry_due` は `RetryAction::Restart` のときだけ `RetryBackoff::CONFLICT` を選ぶ。
- 一時的な失敗のやり直しは gateway の中で閉じた包みになっている。`src-tauri/src/adaptor/gateway/work_queue.rs:78` の `retry` が `WorkFailure`（`kind: FailureKind`）を作って `src-tauri/src/common/retry.rs:69` の `requested` へ渡す。
- `WorkFailure`（`src-tauri/src/usecase/work_queue.rs:27-30`）は `kind: FailureKind` と `message` を持つ。非テストの生成元は 20 箇所あり、うち 11 箇所が `WorkFailure::from_error`（`ClassifiedFailure` から `kind` を取る）である。

## 作業列が入口へ渡す引数も同じ分類から作られている

- 作業列は試行ごとに `RetryAction` をコールバックへ渡す。`src-tauri/src/usecase/work_queue.rs:221` が初回を `Retry` とし、`:321` が前回の失敗の `retry_action()` で更新し、`:260` が取り出し、`:636` の `BorrowedRequest` が型に含む。
- コールバックが受け取るのは `Retry` か `Restart` の 2 値だけである。`:321` の更新は `retry_action() != Stop` の分岐の中でだけ起きる。
- 受け手は 4 箇所で、どれも「前回が版の競合で終わったなら手元の状態を捨てて読み直す」ために見ている。`src-tauri/src/adaptor/gateway/comment/watcher.rs:67`（worker を止めて作り直す）、`src-tauri/src/adaptor/gateway/workflow/workflow_host/node_startup.rs:43`（実行を読み直す）、`src-tauri/src/usecase/agent_session/provider_session_title_ingestion.rs:91`（進捗を捨てる）、`src-tauri/src/usecase/workflow/startup.rs:70`（検査済みの印を落とす）である。
- これとは別に、`src-tauri/src/usecase/work_queue.rs:645`・`:652` の `restart: bool` がある。こちらは「この対象が版の競合の後のやり直しを許すか」を表し、`key.stage` を決める。
- `RetryAction` を直接見る非テストの箇所は他に `src-tauri/src/usecase/workflow/node_startup.rs:77`・`:126` がある。

## 「やり直しで直る失敗は Node の失敗にしない」が 2 か所で別の形に書かれている

- `settle_runtime_failure_for_node`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:2086-2095`）は `WorkflowRuntimeError::Conflict` のときだけ Node の失敗にせずに返す。
- `record_node_start_failure`（同 `:1393-1425`）は `error.failure_kind().retry_action()` で判定し、`:1408` で `Stop` でなければやり直す対象として記録し、`:1414` で `Restart` でなければ Node の失敗として確定させる。
- `WorkflowRuntimeError::Store(kind)` と `StorageFailure { kind, .. }` は store 側の分類をそのまま返す（`src-tauri/src/usecase/workflow/runtime_error.rs:150-152`）。このため store 由来の `RestartRequired` について、2 つの判定の結果が食い違う。
- `settle_runtime_failure_for_node` の非テストの呼び出し元は 3 か所である（`workflow_host.rs:1415`・`:2082`、`src-tauri/src/adaptor/gateway/workflow/workflow_host/node_startup.rs:135`）。

## 「要対応」の判定が `FailureKind` から導かれ、状態が 2 か所に持たれている

- `FailureKind::requires_attention()` は `retry_action()` が `Stop` かつ `Cancelled` でないことを指す。
- 「対象が今『要対応』か」は 2 つの表現で持たれている。記録の側は `src-tauri/src/domain/failure_records.rs` の `FailureRecords` で、`observe` が同じ `(operation, target)` の既存の記録を `resolve` で非 active にしてから新しい記録を active にするため、`record.active && record.kind.requires_attention()` で答えが出る（`:65`・`:72`）。対象ごとの状態の側は `src-tauri/src/domain/failure.rs:51-66` の `TargetFailures`（`HashMap<(operation, target), FailureKind>`。要対応でない `kind` を受けると項目を消す）と `:68-72` の `BackgroundFailures` trait で、実装は `src-tauri/src/domain/repository/background_failure.rs`、`src-tauri/src/domain/terminal_surface/background_failure.rs`、`src-tauri/src/domain/agent_session/background_failure.rs` の 3 つである。どれも `TargetFailures` を 1 つ持ち、固定の operation 名（`repository_scan` / `terminal_checkpoint` / `provider_session_title`）を渡すだけの包みである。
- 書き込みは両方へ行われる。`src-tauri/src/usecase/work_queue.rs:405-418` の `observe` と `:442-455` の `clear_attention` が、記録と対象ごとの状態の両方を更新する。
- 読み出しは `src-tauri/src/usecase/failure_query_service.rs:85-90` で、operation が 3 つのどれかなら対象ごとの状態を、それ以外なら記録を見る。このため 3 つの operation では、対象が今失敗していれば過去の（active でない）記録にも `requires_attention` が付く。
- ページ全体の `requires_attention`（同 `:44-70`）と、画面の表示（`src/components/workflow/BackgroundFailures.tsx:22`）は、どちらの表現でも同じ答えになる。記録ごとの `requires_attention` を画面は使わない。
- 記録ごとの `requires_attention` はサーバ側で使われる。`src-tauri/src/usecase/failure_query_service.rs:109-118` の `apply_workflow_failures` が、`requires_attention` な記録ごとに `tree.observe_background_failure(target, kind, message)` を呼ぶ。呼ばれた Node は `error_reason` を上書きし（`src-tauri/src/domain/workspace_tree/background_failure.rs:5`）、`status_classification` を要対応にする。3 つの operation では過去の記録も呼ぶため、Node に残る `error_reason` は最後に回った記録の message になる。
- `requires_attention()` は `src-tauri/src/domain/workspace_tree/entities/mod.rs:36` でも見られ、`src-tauri/src/adaptor/controller/api/state_subscription.rs:15`・`:29` を通って画面へ出る。

## 期限切れは呼び出し元の期限と自前の期限を区別せず、結果も揃っていない

- 期限切れは `src-tauri/src/common/operation_context.rs:69-72` の `OperationStopped::Expired` の 1 値で表される。`src-tauri/src/adaptor/gateway/shared/operation_context.rs:6` が `FailureKind::Expired` へ変え、`DEADLINE_EXCEEDED` で終わる。
- Connect の入口（`src-tauri/src/adaptor/controller/api/client.rs:273`）は既定 120 秒の期限を持ち、`:133`・`:176` の `ingress` がその期限を `OperationContext` に入れる。`src-tauri/src/common/operation_context.rs:38-46` の `with_deadline` は親の期限と `minimum` を取り、`:51-65` の `check` はどちらが勝ったかを区別せずに `Expired` を返すため、期限が切れたときに呼び出し元の期限か自前の期限かは値に残らない。`:125` の `with_timeout`、`:214` の `timeout_sync`、`:218` の `timeout` も同じ経路を通る。
- 自前の期限を持つ非テストの箇所は次のとおり。`src-tauri/src/adaptor/gateway/git_host/github.rs:61`・`:187`（`gh` 実行）、`src-tauri/src/adaptor/gateway/notion/service_impl.rs:641`（HTTP 呼び出し）、`src-tauri/src/adaptor/gateway/local_event_store/reader.rs:378`（クエリの期限）、`src-tauri/src/adaptor/gateway/shared/background_io.rs:17`（`io::ErrorKind::TimedOut`）、`src-tauri/src/adaptor/gateway/work_queue.rs:57`（試行の 20 秒の期限）。
- `src-tauri/src/adaptor/gateway/local_event_store/connection.rs:48` の SQLite の busy の待ち（2 秒）の期限切れは、`busy_handler` の戻りが真偽値であるため `SQLITE_BUSY` のまま返り、`FailureKind::Temporary` を経て `UNAVAILABLE` で終わる。
- 画面（`src/lib/client.ts:471`）と CLI（`src-tauri/src/cli/api_client.rs:193-199`）は、`DEADLINE_EXCEEDED` と `UNAVAILABLE` を読み分けていない。Rust 側にも `DeadlineExceeded` を読み戻す非テストの箇所は無い。
- HTTP local API は `FailureKind` を見ない。期限切れの技術的な失敗は `WorkflowError::Technical` を経て 500 になる（`src-tauri/src/adaptor/controller/api/error_test.rs:5-15`）。

# Scope / Non-goals

今回変更する対象。

- `FailureKind` と `ClassifiedFailure` の廃止と、それぞれの置き換え。`src-tauri/src/domain/failure.rs` の定義と、参照する domain 35・usecase 28・adaptor 41 ファイルを含む。`RetryAction`・`retry_action()`・`requires_attention()`・`TargetFailures`・`BackgroundFailures` も同じファイルから無くす。
- 技術的な失敗の変種が持つ失敗の性質を、転送のコードの別名ではない 4 つ（一時的・期限切れ・取り消し・それ以外）として定義し直すこと。性質を決めるのは失敗を生んだ側の gateway で、変換は `src-tauri/src/adaptor/gateway/shared/` に集約する。store の失敗も同じで、rusqlite のエラーコードから性質を決める（`src-tauri/src/adaptor/gateway/local_event_store/reader.rs`）。
- 作業列に載る失敗と失敗の記録が運ぶ値を新設すること。中身は「業務の失敗（集約の版の競合か、それ以外か）」か「技術的な失敗（性質 4 値）」のどちらかで、既存の 2 つを 1 つに包むだけの値とする。判断のメソッドはこの値に持たせない。
- 作業列が試行ごとに入口へ渡す引数（今の `RetryAction`）を、受け手の動きを表す 2 値に置き換え、作業列（`src-tauri/src/usecase/work_queue.rs`）が所有すること。
- store 由来の版の競合（`CommitBatchError::StreamHeadConflict`・`TreeHeadConflict`）を、`WorkflowError` と `WorkflowRuntimeError` の既存の `Conflict` と同じ業務の失敗の変種で受けること。
- `WorkflowRuntimeError::Store(FailureKind)`・`StorageFailure { kind }` と `WorkflowError::Store(FailureKind)`・`StorageUnavailable { kind }` の 4 変種を、store の失敗の値（`CommitBatchError` / `LocalEventQueryError`）をそのまま持つ技術的な失敗の変種 1 つにまとめること。統合は両方の型で揃え、その間の変換（`src-tauri/src/adaptor/gateway/workflow/runtime_command_gateway.rs:93-100`、`src-tauri/src/usecase/workflow/control_plane.rs:747-756`）も統合後の変種どうしで行う。統合後の変種は、store の失敗と、今そこへ変換されている store 以外の技術的な失敗（`src-tauri/src/usecase/workflow/execution_archive.rs`、`startup.rs`、`node_startup.rs`）の両方を運ぶ。
- 外部の失敗から失敗の性質を決める変換の集約。今の `src-tauri/src/adaptor/gateway/shared/operation_context.rs:6`、`src-tauri/src/adaptor/gateway/shared/background_io.rs:6-20`、`src-tauri/src/adaptor/gateway/local_event_store/reader.rs:43` が分類している。
- domain の中で業務の語彙の上に `FailureKind` が重なっている箇所の置き換え。`src-tauri/src/domain/local_event/failure.rs` の `SafeOperationFailure` から `classification` を無くすことを含む。
- 失敗を JSON 文字列にして転送の失敗の message に載せる経路の廃止。`src-tauri/src/usecase/comment/dto.rs` の `ReviewErrorDto`・`ReviewErrorCodeDto`・`review_error_to_json_string` と、`src-tauri/src/domain/comment/mod.rs` の `ReviewError::code()`・`ReviewErrorCode` を消す。`src-tauri/src/adaptor/controller/client/comment/commands.rs` の 6 箇所（`:40`・`:68`・`:98`・`:130`・`:152`・`:173`）は `ReviewError` の変種を presenter へ渡す形にし、presenter が変種からステータスコードを決める。
- `retry_action()` によるやり直しの判断の置き換え。業務の手順のやり直し（`src-tauri/src/usecase/workflow/control_plane.rs:108`・`:126`、`node_startup.rs:83`、`command/mod.rs:31`、`delegate.rs:69`）、作業列（`src-tauri/src/usecase/work_queue.rs`）、gateway が包みに渡す判定（`src-tauri/src/adaptor/gateway/work_queue.rs`）、`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs`、`RetryAction` を直接見る箇所を含む。
- 「やり直しで直る失敗は Node の失敗にしない」の判定を、`settle_runtime_failure_for_node` と `record_node_start_failure` の両方で「集約の版が競合したか」の 1 条件にすること。前者の呼び出し元 3 か所を含む。
- 「要対応」の判定の置き換えと、判定の置き場所を domain から失敗の記録を読む側（`src-tauri/src/usecase/failure_query_service.rs`）へ移すこと。判定が見るのは技術的な失敗の性質と業務の失敗だけにする。
- 「対象が今『要対応』か」の二つ目の表現の削除。`TargetFailures`、`BackgroundFailures`、3 つの `background_failure.rs`（`src-tauri/src/domain/repository/`、`src-tauri/src/domain/terminal_surface/`、`src-tauri/src/domain/agent_session/`）、`FailureQueryService.target_failures`、`src-tauri/src/usecase/work_queue.rs:405-418`・`:442-455` の二重の書き込みを含む。
- 失敗の記録の分類の表示文字列を presenter が作ること。今は `src-tauri/src/adaptor/controller/api/state_subscription.rs:24` が内部の型の Debug 表記をそのまま送っている。
- usecase のエラー型から分類を外すこと。`ClassifiedFailure` を実装する usecase の 23 ファイルが対象で、エラー型そのものは残す。
- `AppError` の中身（`FailureKind` とフロントへの対応付け）。
- 転送のステータスコードへの対応付けの presenter への移動。Connect の `src-tauri/src/adaptor/protocol/connect.rs` と、HTTP local API の `src-tauri/src/adaptor/controller/api/error.rs` が対象である。presenter は業務の失敗の変種と技術的な失敗の性質から直接ステータスコードを決め、`FailureKind` を経由しない。store の失敗では、より細かいコード（`DATA_LOSS`・`RESOURCE_EXHAUSTED`・`FAILED_PRECONDITION`・`ABORTED`）を選ぶときだけ元の値を見る。
- CLI の `src-tauri/src/cli/file_direct.rs:67-81` の `workflow_error_to_cli_error` を、統合後の変種に合わせて書き換えること。CLI の出力と終了コードは今と同じに保つ。
- 上記に伴い使われなくなるコードの削除。`src-tauri/src/adaptor/gateway/shared/operation_context.rs` の `ClassifiedFailure for OperationStopped` と、その `failure_kind()` だけを確かめるテストを含む。

今回変更しない対象。

- `UsecaseError` などの usecase のエラー型そのものの廃止。マイルストーン #97 が扱う。
- 2 つの技術的な失敗の変種（store の失敗を含む統合後の変種と、`Technical`）を 1 つにまとめること。2 つ並ぶのは HTTP local API の 503 と 500 の出し分けを保つためであり、1 つにまとめるのはマイルストーン #100 で HTTP local API が無くなった後とする。
- `src-tauri/src/adaptor/protocol/` の `connect.rs` 以外と、HTTP local API のリクエスト・レスポンスの型の presenter への移動。#1931 が扱う。
- 画面（`src/`）のコード。ステータスコードを変えないため、合わせる必要が無い。
- CLI の振る舞い。出力と終了コードは今と同じに保つ。「CLI を変えない」は振る舞いを変えないという意味であり、ファイルに触れないという意味ではない。
- Tauri のシェルの `src-tauri/src/adaptor/gateway/desktop_client.rs`。動き続けるように合わせるだけにする。
- 失敗の記録の分類の表示文字列と、記録がまとまる単位を今と同じに保つこと。分類を作り替えるため、どちらも変わる。
- 作業列の入口・包み・記録の分解と、`src-tauri/src/domain/work_queue.rs`・`src-tauri/src/domain/failure_records.rs` の廃止。#1930 が扱う。新設する値を domain に置くのは、失敗の記録が #1930 まで domain に残り同じ値を持つ必要があるためであり、#1930 で失敗の記録と一緒に domain から出す。作業列が入口へ渡す引数も、#1930 で作業列を分けるときに持ち主が移る。
- 取り消しによる失敗を失敗の記録に残すこと。今も記録され、「要対応」でない失敗として一覧に出ている。記録をやめるかどうかは #1930 が扱う。
- 「要対応」を画面へ出す経路（`src-tauri/src/usecase/failure_query_service.rs` から `src-tauri/src/adaptor/controller/api/state_subscription.rs` まで）の組み直し。#1930 が扱う。この変更は判定の置き場所を移すだけにとどめる。
- やり直しの待ち時間の式の値と、単位時間あたりのやり直しの回数の上限の値。#1890 が扱う。
- 呼び出し元の期限と自前の期限を区別する仕組み。出ていく側の自前の期限切れが `DEADLINE_EXCEEDED`、SQLite の busy の待ちの期限切れが `UNAVAILABLE` で結果が揃っていない点は、別の ISSUE で扱う。
- `src-tauri/src/adaptor/gateway/local_event_store/connection.rs` の `busy_handler` が期限切れを戻り値で運べないこと。rusqlite の `busy_handler` の戻りは真偽値である。
- 規約（`docs/architecture/`）そのものの変更。

# Requirements

- R-001: domain が表す失敗は業務の意味があるものだけであり、転送の語彙（gRPC のステータスコード、HTTP status）と 1 対 1 に対応する分類を domain は持たない。
- R-002: trait の失敗は、業務の結果と、中身を見ない技術的な失敗の変種 1 つで表される。技術的な失敗が一時的か、やり直してよいかを domain は判断に使わない。
- R-003: Usecase が出した失敗から転送のステータスコード（Connect のステータスコード、HTTP local API の HTTP status）を決めるのは presenter の 1 か所だけであり、業務の失敗と技術的な失敗を区別して対応付ける。presenter の外で転送の失敗を組み立てない。
- R-004: 業務の手順をやり直すかは、集約の版が競合したという業務の失敗で決まる。一時的な失敗をやり直すかは、技術的な失敗が持つ性質で決まる。
- R-005: 「やり直しで直る失敗は Node の失敗にしない」の判定は 1 つであり、その条件は集約の版が競合したかである。この判定を通るすべての経路で同じ結果になる。
- R-006: 画面と CLI が受け取る転送のステータスコードは、この変更の前と同じである。Connect では、出ていく側の自前の期限切れが `DEADLINE_EXCEEDED`、SQLite の busy の待ちの期限切れが `UNAVAILABLE` である。HTTP local API では、期限切れによる技術的な失敗が 500 である。
- R-007: 技術的な失敗は、presenter が転送のステータスコードを決めるために必要な情報を、それ自身が運ぶ。少なくとも一時的・期限切れ・取り消し・それ以外の区別が、失敗の外の情報を使わずに導ける。store の失敗では元の失敗の値も運ばれ、presenter はより細かいステータスコードを選ぶときだけそれを見る。
- R-008: `AppError` がフロントへ返す serialize 表現は変わらない。通常の失敗はメッセージのプレーン文字列であり、`code` を持つ失敗は `code` と `message` の object である。
- R-009: Tauri のシェルの daemon の生存確認は、同時に処理できる呼び出しの枠が埋まったことによる失敗と、それ以外の失敗を、今と同じに区別して続く。
- R-010: 「要対応」として表示される対象は、この変更の前と同じである。一時的な技術的な失敗、取り消しによる技術的な失敗、集約の版の競合という業務の失敗は「要対応」にしない。期限切れによる技術的な失敗、それ以外の技術的な失敗、版の競合以外の業務の失敗は「要対応」にする。
- R-011: この変更で使われなくなるコードと、この変更で触れたファイルの中で使われていないコードは無い。
- R-012: 失敗の記録の分類は、業務の失敗（集約の版の競合、それ以外）と技術的な失敗の性質（一時的、期限切れ、取り消し、それ以外）の 6 つで表される。画面へ出る分類の文字列は、その 6 つごとに presenter が決めた文字列である。同じ対象の失敗を 1 件の記録にまとめる単位も、この 6 つの値である。
- R-013: Node に表示される失敗の理由は、その対象について今有効な失敗の記録の message である。解消済みの記録の message は残らない。

# Assumptions / Open Questions

未確定事項は無い。人間が明示的に受け入れた仮定は無い。
