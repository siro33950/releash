# Context

- 正本: [#1890 \[04\] daemon の中の繰り返し処理を作業列に載せる](https://github.com/siro33950/releash/issues/1890)
- 所属: [milestone #99「02. UI と daemon の間の通信の仕組みを一本化する」](https://github.com/siro33950/releash/milestone/99) の Wave 04。
- 依存の #1880 は merge 済み（`42be41e0`）。失敗の分類は `src-tauri/src/domain/failure.rs` の `FailureKind`（16 変種）と `ClassifiedFailure` が運び、Connect のエラーコードとの対応は `src-tauri/src/adaptor/protocol/connect.rs:29-46` にある。`Temporary` が `UNAVAILABLE`、`RestartRequired` が `ABORTED`、`StateRequired` が `FAILED_PRECONDITION`、`Capacity` が `RESOURCE_EXHAUSTED`、`Expired` が `DEADLINE_EXCEEDED`、`Cancelled` が `CANCELLED`、`Internal` が `INTERNAL`。分類の要求は `docs/specs/issues-1880/requirements.md`、受入条件は `docs/specs/issues-1880/behavior.md`。
- 従う標準と、その標準が定めている内容。
  - [AIP-194](https://google.aip.dev/194): retry してよいのは `UNAVAILABLE` のみ。`ABORTED` は「個別リクエストではなくトランザクション全体で再試行すべき」。`RESOURCE_EXHAUSTED`・`INTERNAL`・`UNKNOWN`・`FAILED_PRECONDITION`・`NOT_FOUND`・`ALREADY_EXISTS`・`PERMISSION_DENIED`・`UNAUTHENTICATED`・`OUT_OF_RANGE`・`UNIMPLEMENTED` は「一般的に retry してはいけない」。`CANCELLED`・`DEADLINE_EXCEEDED`・`INVALID_ARGUMENT`・`DATA_LOSS` は「絶対に retry してはいけない」。`CANCELLED` と `DEADLINE_EXCEEDED` の理由は、いずれもアプリケーション自身の指示を「尊重すべき」こと。
  - [gRPC connection backoff](https://github.com/grpc/grpc/blob/master/doc/connection-backoff.md): `current_backoff = Min(current_backoff * MULTIPLIER, MAX_BACKOFF)` と、deadline への `UniformRandom(-JITTER * current_backoff, JITTER * current_backoff)` の加算。既定は `INITIAL_BACKOFF` 1 秒 / `MULTIPLIER` 1.6 / `JITTER` 0.2 / `MAX_BACKOFF` 120 秒、1 回の試行の deadline は `Max(current_deadline, now() + MIN_CONNECT_TIMEOUT)`（`MIN_CONNECT_TIMEOUT` 20 秒）。retry 回数の上限は持たない。
  - [client-go workqueue](https://pkg.go.dev/k8s.io/client-go/util/workqueue): 既定は per-item の指数 backoff（5 ms〜1000 秒）と全体の token bucket（10 回/秒・一度に 100）の `MaxOf`。rate limiter 自体は回数の上限を持たず、「max failures and expiration are up to the caller」。
  - [client-go retry](https://pkg.go.dev/k8s.io/client-go/util/retry): `RetryOnConflict` は Conflict のみ再試行し、他は即座に返す。`DefaultBackoff` は Steps 4 / Duration 10 ms / Factor 5.0 / Jitter 0.1。
  - client-go Reflector: `ListAndWatch` の失敗は上限なく backoff して retry する（800 ms / Factor 2.0 / Cap 30 秒 / Jitter 1.0、2 分成功でリセット）。
  - Kubernetes controller（deployment controller）: `maxRetries` に達したら作業列から落とすが、対象のオブジェクトは削除しない。落としてよいのは informer が対象の変化と resync で積み直すためであり、Releash にはこれに当たる仕組みが無い。
  - [Kubernetes Event v1](https://kubernetes.io/docs/reference/kubernetes-api/cluster-resources/event-v1/) と client-go `tools/record/events_cache.go`: Event は API 資源でログではない。`series.count` と `series.lastObservedTime`、`eventTime` を持つ。集約キーは source + involvedObject + Type + Reason で、文面が違うものも畳む。保持は限定的（「Events have a limited retention time」）で、集約の状態は上限付きの LRU（4096）。
  - Kubernetes API conventions: 人の対応を要する状態は対象オブジェクトの `status.conditions`（type / status / reason / message / lastTransitionTime）で表す。要対応だけを集めた専用の資源は無い。
- 待ち時間の計算と失敗の記録は、この変更より後の ISSUE が利用する。生存の判定は #1879、同時実行の枠の拒否の記録は #1894。どちらもこの変更では扱わない。
- 隣接する ISSUE の分担: client のつなぎ直しは #1891、接続状態の一元化は #1895、store と外部プロセスへの期限の引き継ぎおよび lock 待ちは #1893、画面がサーバを起動し直さない形は #1904。store の読み込み・書き込みの入口は #1881・#1892 で async の 1 つになっている。
- 参照する既存実装: `src-tauri/src/usecase/workflow/startup.rs`、`src-tauri/src/adaptor/controller/daemon.rs`、`src-tauri/src/adaptor/gateway/workflow/startup_repository.rs`、`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs`、`src-tauri/src/infrastructure/terminal/checkpoint_scheduler.rs`、`src-tauri/src/usecase/agent_session/provider_session_title_ingestion.rs`、`src-tauri/src/infrastructure/comment/watcher.rs`、`src-tauri/src/usecase/repository_state/worker.rs`、`src-tauri/src/usecase/repository_state/worktree.rs`、`src-tauri/src/usecase/workflow/node_startup.rs`、`src-tauri/src/usecase/workflow/command/mod.rs`、`src-tauri/src/adaptor/gateway/provider_lifecycle/event_repository_impl.rs`、`src-tauri/src/domain/workflow/error.rs`、`src-tauri/src/usecase/workflow/runtime_error.rs`、`src-tauri/src/domain/workspace_tree/value_objects/mod.rs`、`proto/client.proto`。

# Outcome

対象は、workflow を実行してその状態を画面で見る利用者と、daemon を変更する開発者である。

現在は、daemon の中で繰り返す処理と失敗の後にやり直す処理が、箇所ごとに別の待ち時間と別の判断で書かれている。そのため、一時的な失敗で実行木が abort され、直らない失敗が同じ間隔でやり直され続けてログを埋め、逆に一度失敗したまま二度とやり直されない処理もある。失敗の記録は失敗の回数だけ増え、どの対象に手が必要なのかが分からない。

変更後は、daemon の中の繰り返しとやり直しが 1 つの作業列と 1 つの待ち時間の計算に載る。やり直すかどうかは失敗の分類だけで決まり、同じ原因の失敗はどの処理で起きても同じ扱いになる。同じ失敗は 1 件の記録にまとまり、回数と最初・最後の時刻を持つ。手が必要な失敗は、その対象の「要対応」として利用者が観測できる。
# Current Behavior

2026-09-25 に worktree `feat/issues/1890` の HEAD（`9194bd57`）で確認した。正本は `42be41e0` を基準にしているが、その後 #1881・#1883・#1885・#1892 が merge されている。以下は `9194bd57` での位置である。

起動時の再開処理

- `src-tauri/src/adaptor/controller/daemon.rs:356-361` が `recover_startup()` を 1 度だけ `tokio::spawn` し、失敗は `log::warn` を 1 行出して終わる。やり直しは無い。
- `src-tauri/src/usecase/workflow/startup.rs:29-72` の `execute()` は、`recovery_lock` によりプロセスごとに 1 回だけ本体を実行する。`list_tree_ids()` の失敗は `?` でそのまま返り、`recovery_lock` は立ったままになるため、実行木の一覧が読めなかった場合は再開処理そのものが二度と動かない。
- 実行木ごとに `reconcile_tree` を呼び、失敗すると失敗の分類に関係なく `abort_startup_failure` でその実行木を abort する。store が混んでいた（`Temporary`）だけでも abort される。
- 失敗の分類は経路の途中で `Internal` に潰れる。`src-tauri/src/adaptor/gateway/workflow/startup_repository.rs:31-36` が `Conflict` 以外を `WorkflowError::external` にし（`src-tauri/src/domain/workflow/error.rs:93` で `Internal`）、`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:657-662` が `Conflict` 以外を `WorkflowRuntimeError::SessionStore` にする（`src-tauri/src/usecase/workflow/runtime_error.rs:154` で `Internal`）。

daemon の中の他の繰り返し処理

| 処理 | 場所 | 周期 | 失敗したときの扱い |
|---|---|---|---|
| terminal の保存 | `src-tauri/src/infrastructure/terminal/checkpoint_scheduler.rs:20-70` | 250 ms（`src-tauri/src/adaptor/gateway/terminal_surface/runtime_gateway_impl.rs:52`） | 同じ間隔でやり直し、回数の上限なし。毎回 `log::error` |
| provider session のタイトルの取得 | `src-tauri/src/adaptor/controller/daemon.rs:159-166`、`src-tauri/src/usecase/agent_session/provider_session_title_ingestion.rs:38-100` | 20 秒（`src-tauri/src/domain/agent_session/provider_session_title_cadence.rs:3`） | `log::warn` して次の周期まで待つ |
| review comment の監視 | `src-tauri/src/infrastructure/comment/watcher.rs:102-131` | 1 秒 | 監視を始められなければ `log::error` を出して return し、以後この監視は動かない |
| repository の走査 | `src-tauri/src/usecase/repository_state/worker.rs:60-133` | 変化を待って起動 | `log::warn` と `mark_scan_failed` の後、次の変化を待つ。やり直しは無い |

失敗の後にやり直す処理。待ち時間とやり直すかどうかの判断が箇所ごとに別に書かれている。

| 種類 | 箇所 | 間隔・回数 | やり直すかどうかの判断 |
|---|---|---|---|
| 起動のやり直し | `src-tauri/src/usecase/workflow/node_startup.rs:22-61`、待ち時間は `src-tauri/src/domain/workflow/value_objects/node_execution.rs:3-5` | 1・2・4・8 秒、4 回、ばらつきなし | 失敗の理由を問わない |
| 競合のやり直し | `src-tauri/src/usecase/workflow/command/mod.rs:19-48`、`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:255-263` | 待ち無し、4 回 | `Conflict` の変種かどうか。分類は見ない |
| 同上 | `src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:2045-2056` | 待ち無し、4 回 | `Conflict` または `SessionStore`（`SessionStore` の分類は `Internal`） |
| 呼び出しの失敗のやり直し | `src-tauri/src/adaptor/gateway/provider_lifecycle/event_repository_impl.rs:171-186` | 10 ms から倍々、4 回、ばらつきなし | `FailureKind::Temporary` かどうか |

- 分類とやり直しが噛み合っていない。`WorkflowError::Conflict` の分類は `RestartRequired`（`src-tauri/src/domain/workflow/error.rs:99`）だが、同じ `RestartRequired` を運びうる `WorkflowError::Store` や `StorageUnavailable` はやり直されない。`CommitBatchError` は `StreamHeadConflict`・`OutcomeUnknown`・`TreeHeadConflict`・`AppendOutcomeUnknown` を `RestartRequired`、`QueueBusy` を `Temporary`、`CapacityExceeded`・`SequenceExhausted` を `Capacity` に分類する（`src-tauri/src/domain/local_event/batch.rs:148-162`）。一方、分類が `Internal` の `SessionStore` はやり直される。
- 待ち時間にばらつきを掛けている箇所は無い。同時に失敗した対象のやり直しは同じ時刻に揃う。
- やり直しの頻度に、処理をまたぐ上限は無い。
- 同じ失敗を 1 件にまとめる記録は無い。上記の各箇所は失敗のたびに `log::warn` または `log::error` を出す。
- 状態が直るまでやり直すべきでない失敗を、その対象の「要対応」として持つ状態も、それを画面へ出す経路も無い。

要対応を持つ状態の受け皿

| 対象 | 受け皿 | 現状 |
|---|---|---|
| 実行木・Node | ある | `WorkspaceNodeStatusClassification`（`Active` / `Attention` / `Idle` / `Unbound`、`src-tauri/src/domain/workspace_tree/value_objects/mod.rs:53-86`）と `error_reason`。`classify_own_status`（同 155-186）が導出し、`most_severe` で祖先へ巻き上がる。proto にも `WorkspaceStatusClassification.attention`（`proto/client.proto:2804`）と `error_reason`（同 1824）がある |
| repository | ない | 走査の状態は `usecase/repository_state/worktree.rs` の `WorktreeState` が持つ。`mark_scan_failed`（同 242-248）は `refreshing` を降ろすだけで、失敗した事実は残らない |
| terminal | ない | `log::error` のみ |
| agent session | ない | `log::warn` のみ |

# Scope / Non-goals

変更する。

- やり直しの待ち時間の計算。処理の種類によらず同じ式で、値だけを処理ごとに指定できる形にすること。
- 失敗した対象を間隔を延ばしてやり直す作業列と、やり直しの頻度の上限。
- 同じ失敗を 1 件にまとめ、回数と最初・最後の時刻を持つ記録と、その観測の経路。
- やり直すかどうかの判断を、失敗の分類だけに基づく形へ置き換えること。
- 起動時の再開処理（`src-tauri/src/usecase/workflow/startup.rs:29-72`、`src-tauri/src/adaptor/controller/daemon.rs:356-361`）の、作業列への移行と、再開の失敗で実行木を abort しない形への変更。実行木の一覧の読み込みを含む。
- 起動時の再開処理の経路で分類を潰している変換（`src-tauri/src/adaptor/gateway/workflow/startup_repository.rs:31-36`、`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:657-662`）を、分類を保つ形にすること。
- Current Behavior の「他の繰り返し処理」の表の 4 つの処理の、作業列への移行。
- Current Behavior の「やり直す処理」の表の 4 箇所の、待ち時間の計算と分類による判断への置き換え。
- 繰り返す処理の 1 回の試行が使う期限。
- 要対応の状態と、その観測。対象は実行木・Node、terminal、agent session、repository。実行木・Node は既存の `Attention` を使い、他の 3 種類には状態を新設する。proto（`proto/client.proto`）と画面（TypeScript）の変更を含む。
- repository の要対応の状態を `src-tauri/src/domain/repository/` に持たせること。

変更しない。

- 各処理が成功しているときの実行の周期。正本の実装境界は作業列への移行だけを挙げており、周期の見直しを含まない。
- `usecase/repository_state/` が持つ走査の進行の状態（`refreshing`、`loading`、generation、`scan_lock`）の domain への移設。この変更の要求はこれらに触れない。
- Tauri のシェルの生存確認のループ（`src-tauri/src/adaptor/gateway/desktop_client.rs:46-63`）。#1879 が削除する。
- Tauri のシェルによる daemon の起動し直し（`src-tauri/src/domain/daemon_supervision.rs`）。#1904 が扱う。
- Notion の `Retry-After` の待ち（`src-tauri/src/adaptor/gateway/notion/service_impl.rs:71-107`）。#1893 が扱う。
- 失敗ではなく lock の空きを待つ処理。#1893 が扱う。
- store の読み込みと書き込みの入口。#1881・#1892 が async にしたものを使う。
- 画面（TypeScript）側のつなぎ直し。#1891 が扱う。
- 呼び出しから始まる処理の期限。#1893 が扱う。繰り返す処理が使う期限はこの変更で決める。
- 生存の判定の変更（#1879）と、同時実行の枠の分け方および拒否の記録（#1894）。この変更は、両者が使う待ち時間の計算と失敗の記録を用意するに留める。

# Requirements

- R-001: daemon の中で繰り返す処理が、R-005 でやり直す分類の失敗をしたとき、失敗した対象は、やり直しの間隔を延ばしながらやり直される。同じ対象が同じ間隔でやり直され続けることはない。やり直しの回数に上限はなく、間隔が上限に達した後も、その間隔でやり直され続ける。
- R-002: やり直しの待ち時間は、どの処理でも同じ式で決まる。式は `min(初回 × 倍率^(n-1), 上限)` にばらつき（0.8 倍から 1.2 倍）を掛けた値であり、処理ごとに別の式にはならない。初回・倍率・上限は処理の種類ごとに指定できる。
- R-003: 多数の対象が同時に失敗しても、daemon が行うやり直しは単位時間あたりの回数の上限を超えない。
- R-004: ある失敗をやり直すかどうかは、その失敗の分類だけで決まる。同じ分類の失敗は、どの処理で起きても同じ扱いになる。失敗の文面、原因の型、エラーの変種からやり直すかどうかを決めることはない。
- R-005: `UNAVAILABLE` に相当する分類の失敗は、その処理をやり直す。`ABORTED` に相当する分類の失敗は、状態を読み直した上の段階からやり直す。この 2 つ以外の分類の失敗はやり直さない。
- R-006: やり直さない分類のうち、利用者またはシステムが意図して止めたことを表す分類（`CANCELLED` に相当）を除く分類で失敗した対象は、その対象が「要対応」であることを、その対象の表示から利用者が観測できる。対象は実行木・Node、terminal、agent session、repository である。
- R-007: 同じ失敗は 1 件の記録にまとまり、その記録は発生回数と最初・最後の時刻を持つ。同じ失敗とは、処理の種類・対象・失敗の分類が同じことをいい、失敗の文面は問わない。同じ失敗が続く間、記録の件数は増えない。この記録は daemon が保持する状態であり、利用者が観測できる。保持には件数の上限があり、daemon を起動し直すと引き継がれない。
- R-008: 起動時の再開処理は、実行木ごとにやり直しの対象になる。実行木の一覧の読み込みも、同じくやり直しの対象になる。再開の失敗によって実行木が abort されることはない。
- R-009: 起動時の再開処理の失敗は、失敗が起きた場所の理由に対応した分類で呼び出し元に届く。store が混んでいて失敗した場合と、状態が壊れていて失敗した場合とで、届く分類は異なる。
- R-010: 繰り返す処理を始められなかったときも、その失敗は R-005 の判断の対象になり、やり直す分類であればその処理の開始がやり直される。開始が一度失敗したというだけで、その処理が以後動かないままになることはない。
- R-011: 繰り返す処理の 1 回の試行には期限がある。終わらない試行が、同じ対象の以後のやり直しや他の対象の処理を止め続けることはない。
- R-012: やり直さない分類で失敗した対象について、その後に処理すべき新しい変化が生じたときは、その変化に対する処理が行われる。過去の失敗を理由に、その対象の繰り返す処理が daemon を起動し直すまで行われないままになることはない。失敗した試行そのものがやり直されるわけではない。

# Assumptions / Open Questions

- 自動判断: R-001 の「失敗したとき」を「R-005 でやり直す分類の失敗をしたとき」に限定した。元の文は分類を問わずやり直すと読め、「この 2 つ以外の分類の失敗はやり直さない」とした R-005 と両立しなかった。やり直す範囲を広げない側へ寄せた。
- 自動判断: R-010 の「その処理はやり直される」を R-005 の判断の対象に含めた。元の文は開始の失敗だけ分類の判断の外に置くと読め、R-004 の「やり直すかどうかは分類だけで決まる」と両立しなかった。開始の失敗も同じ規則で扱う側へ寄せた。開始が要対応として残る場合の扱いは R-006 が定める。
- 自動判断: B-013 の「同じ対象の次のやり直しと、他の対象の処理が進む」を、R-011 の本文どおり「止め続けられることはない」へ揃えた。元の文は期限切れの後に必ずやり直しが起きると読め、`DEADLINE_EXCEEDED` に相当する分類をやり直さないとした R-005 と両立しなかった。
- 自動判断: R-012 を追加した。やり直さない分類で失敗した後に、その対象へ処理すべき新しい変化が生じたときの扱いを、正本と R-001〜R-011 のいずれも定めていなかった。変更前は失敗の後も新しい変化が次の周期で処理されていたため、既存の挙動を維持する側へ寄せた。時間の経過だけを契機とする処理へは広げていない。
