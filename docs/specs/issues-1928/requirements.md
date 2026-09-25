# Context

- 正本: [#1928 `[04] 横断的関心事を common の包みにする`](https://github.com/siro33950/releash/issues/1928)
- 所属: [milestone #99「02. UI と daemon の間の通信の仕組みを一本化する」](https://github.com/siro33950/releash/milestone/99) の Wave 04。milestone の「層の整理（規約 `docs/architecture/` に合わせる。対象はサーバのコードだけ）」の 4 件（#1928・#1929・#1930・#1931）の先頭である。
- 規約の正本は `docs/architecture/`。この変更が根拠にする規定は次のとおり。
  - `README.md`「部品の一覧」: 置いてよい部品は表のものだけであり、「表に無い部品は置かない。新しい種類の部品が要るときは、先にこの表を変える」。common の部品は「横断的関心事の包み」だけで、役割は「処理を外から包んで振る舞いを足す」。
  - `README.md`「依存方向」: `usecase / adaptor / infrastructure → common`。domain は「外側を一切知らない（依存を持たない）。common も使わない」。「common はどの層にも依存しない」。
  - `README.md`「横断的な設計原則」: 「横断的関心事は、処理の中に書かず、包みとして掛ける。包みの定義は common に置く。掛ける位置は、受け手の側（controller の手前）、Usecase の入口、出ていく側（adaptor/gateway と infrastructure）の 3 つ。domain は包みを持たない」。
  - `DOMAIN.md`: 「横断的関心事を持たない: 包み（common）を使わず、期限・取り消し・再試行・ログのような横断的関心事を受け取らない」。
  - `USECASE.md`: 「横断的関心事は Usecase の中に書かない: ログ・計測・検証などは、common の包みを Usecase の入口に掛けて足す」。
  - `CONTROLLER.md`「受け手の側の横断的関心事」: 「期限・取り消し・同時実行の制限・優先度・ログと計測は、common の包みを入口の手前に掛けて足す。handler の中に書かない」。
  - `GATEWAY.md`「出ていく側の横断的関心事」: 「外部世界への呼び出しに掛ける期限・やり直し・同時実行の制限は、common の包みを外部との接続に掛けて足す。trait の実装の中に直接書かない。一時的な失敗のやり直しは gateway の中で閉じ、内側へは結果だけを返す」。この一覧に計測が無いのは規約側の欠落であり、外部世界への呼び出しの計測は包みで掛けるのが標準である（go-kit の instrumenting middleware、OpenTelemetry の HTTP client の計測）。この ISSUE で計測を足す。
  - `INFRASTRUCTURE.md`: 「横断的関心事の包みを定義しない。包みの定義は common に置き、外部世界への呼び出しに掛ける」。
  - `TEST.md`: `common/` のテストの必須／柔軟は「柔軟」、役割は「横断的関心事の包み」。
- 最初の周の調査基準は `main` の `5c64534d`。正本が「確認済みの事実」として挙げる基準と同じ commit であり、正本の記載がこの commit で成立することを確認した（Current Behavior に記載）。
- この変更が前提にする、既に入っている変更。
  - #1880: 失敗の分類は `src-tauri/src/domain/failure.rs` の `FailureKind`（16 値）と `ClassifiedFailure` が持ち、Connect のステータスコードへの対応付けは `src-tauri/src/adaptor/protocol/connect.rs` の 1 か所で行う。
  - #1883: 単発の呼び出しには daemon 既定 120 秒の期限が当たり、取り消しは `ClientApiDeps::execute` の中の `tokio_util::sync::CancellationToken` が持つ。
  - #1893: 呼び出しの期限と取り消しは store・外部プロセス・lock へ引き継がれる。期限の合成は置き換えではなく先に来る方を採る。
  - #1890: daemon の中の繰り返し処理は作業列に載り、やり直しの待ち時間の式、単位時間あたりのやり直しの回数の上限、失敗の記録と「要対応」の表示を持つ。
- 隣接 ISSUE。[#1929](https://github.com/siro33950/releash/issues/1929)（失敗を業務の失敗と技術的な失敗に分ける）と [#1930](https://github.com/siro33950/releash/issues/1930)（作業列を入口・包み・記録に分ける）はこの ISSUE に依存する。#1930 は「呼び出しを包んでやり直す部分は、#1928 で作った common の包みを使う」とし、「包みの定義。#1928 が扱う」を自らの対象外に置く。[#1931](https://github.com/siro33950/releash/issues/1931)（状態の配信を Output Boundary と presenter に移す）は #1929 に依存する。
- 参照する既存実装: `src-tauri/src/other/`（`dispose.rs`、`error.rs`、`id.rs`、`operation_context.rs`、`performance_switches.rs`、`telemetry/`、`utils.rs`）、`src-tauri/src/domain/operation_context.rs`、`src-tauri/src/domain/retry.rs`、`src-tauri/src/domain/work_queue.rs`、`src-tauri/src/usecase/work_queue.rs`、`src-tauri/src/usecase/telemetry.rs`、`src-tauri/src/adaptor/controller/api/client.rs`、`src-tauri/src/adaptor/gateway/shared/`、`src-tauri/src/infrastructure/telemetry/`。

# Outcome

対象者は、daemon を実装・保守する開発者である。Releash の UI を使う利用者は、この変更の前後で同じ振る舞いを見る。

現在、期限・取り消し・やり直し・計測は、処理の中に直接書かれている。期限と取り消しは domain の型として置かれ、domain と usecase の失敗の型がそれを変種として持ち、業務手順の中でその値を確認している。やり直しは usecase の作業列の中にあり、待ち時間の計算は domain にある。計測は、包みが 1 つだけあり、残りは記録の呼び出しとして usecase と adaptor の処理の中に書かれている。これらの置き場は `other/` であり、規約の部品の一覧に無い名前で、中身も一覧のどの部品にも対応していない。結果として、横断的関心事が業務の言語に混ざり、掛かる位置が入口ごとに違い、同じ関心事の実装が複数の層に散っている。

変更後は、期限・取り消し・やり直し・計測の包みの定義が common の 1 か所にある。包みは、受け手の側、Usecase の入口、出ていく側の 3 つの位置に掛かり、処理の中には書かれていない。domain と usecase は期限・取り消し・やり直しを受け取らず、業務の言語だけで書かれる。`other/` は無くなり、そこにあった部品は規約の部品の一覧のいずれかとして置かれる。利用者から見た振る舞い（期限切れ・中断の結果、期限の引き継ぎ、やり直し、要対応の表示、計測の値）は変わらない。

# Current Behavior

`5c64534d` のコードで確認した挙動である。

## 横断的関心事の置き場が規約の部品の一覧と合っていない

`src-tauri/src/other/`（`mod.rs` 10 行）に 7 つの部品がある。規約の一覧に `other` という層は無く、common に置ける部品は包みだけである。

- `dispose.rs`（40 行）: `dispose_in_background` が値の drop を使い捨てスレッドで行う。非テストの参照は adaptor 1 ファイル、infrastructure 1 ファイル。
- `error.rs`（119 行）: `AppError`。`FailureKind` を持ち、`ClassifiedFailure` を実装し、フロントへ返す serialize 表現（プレーン文字列、または `code` / `message` の object）を決める。非テストの参照は adaptor 37 ファイル（`controller` 36、`protocol` 1）で、usecase と domain と infrastructure からの参照は無い（`usecase/code_error.rs` と `usecase/repository_error.rs` の 2 箇所は doc コメント内の言及のみ）。
- `id.rs`（16 行）: `unique_simple_id` が uuid v4 の 32 桁 hex を返す。非テストの参照は usecase 3 ファイル。
- `operation_context.rs`（84 行）: 期限と取り消しを task-local（`ASYNC_CONTEXT`）と thread-local（`SYNC_CONTEXT`）で運ぶ。`current` / `scope` / `sync_scope` / `spawn_blocking` / `check` / `with_timeout` / `wait` / `sleep` を持つ。
- `performance_switches.rs`（59 行）: 環境変数 `RELEASH_PERF_DISABLE_*` と `RELEASH_PERF_REAL_APP` から `TerminalPerformanceSwitches` と `performance_real_app_mode` を読む。非テストの参照は adaptor 5 ファイル、usecase 1 ファイル。
- `telemetry/`（`mod.rs` 993 行、`attributes.rs` 196 行、`resource.rs` 51 行）: 計測器の登録（`install_metrics`）、記録（`record_*` / `set_*`）、計測の包み（`measure_result`）、性能計測の sample の収集（terminal 起動の各段階、terminal 入力の各段階）、プロセス資源の観測。
- `utils.rs`（12 行）: `unix_timestamp_seconds`。非テストの参照は adaptor 1 ファイル。

`other::` を参照する非テストのファイルは adaptor 62、usecase 10、infrastructure 2、および Tauri のシェルの `desktop.rs` 1 である。

## 期限と取り消しが domain にあり、処理の中で値を確認している

- `domain/operation_context.rs`（97 行）が `Deadline`、`Cancellation` trait、`OperationContext`、`OperationStopped`（`Expired` / `Cancelled`）を持つ。`OperationStopped` は `ClassifiedFailure` を実装して `FailureKind::Expired` / `Cancelled` を返す。
- `operation_context` を参照する非テストのファイルは adaptor 32、domain 11、usecase 8、`other/` 2 である。正本は domain 10・usecase 8・adaptor 32 とする。domain の 11 は定義そのもの（`operation_context.rs`）と module 宣言（`mod.rs`）を含み、usecase の 8 のうち 3（`code_query_service.rs`、`git_host/git_host_usecase.rs`、`repository_usecase.rs`）は同一ファイル内の `#[cfg(test)]` からの参照だけである。
- domain 側の参照は、9 個の失敗の型が持つ `Stopped(OperationStopped)` 変種と `From<OperationStopped>` の実装である（`agent_session/provider_launch_gateway.rs`、`code/error.rs`、`comment/mod.rs`、`git_host/git_host.rs`、`local_event/batch.rs`、`local_event/query.rs`、`notion/error.rs`、`repository/error.rs`、`workflow/error.rs`）。この変種があることで、期限切れ・取り消しが `FailureKind::Expired` / `Cancelled` として呼び出し元へ届き、Connect のステータスコード `DEADLINE_EXCEEDED` / `CANCELLED` になる。
- usecase 側の非テストの参照は、失敗の型が持つ `Stopped` 変種（`agent_session/agent_session_launch.rs`、`workflow/runtime_error.rs`、`workflow/runtime_resolver.rs`）と、業務手順の中の `check()` / `wait()` / `current()` / `spawn_blocking`（`worktree_operation.rs` 4 箇所、`state_subscription/reads.rs` 2 箇所）である。
- 掛ける位置は揃っていない。受け手の側は `adaptor/controller/api/client.rs` の `ClientApiDeps::execute` と購読の入口が、handler の中で `OperationContext` を作り `scope` で包む。出ていく側は `adaptor/gateway/shared/` の `file_lock.rs`（43 行）・`process.rs`（77 行）・`git_operation.rs`（93 行）が `&OperationContext` を引数で受け取る共通処理であり、包みではない。Usecase の入口には何も掛かっていない。
- `Cancellation` trait の実装は `adaptor/gateway/shared/operation_context.rs` が `tokio_util::sync::CancellationToken` に対して与えている。
- 出ていく側の自前の期限は 5 箇所にある（`adaptor/gateway/git_host/github.rs`、`comment/mod.rs`、`notion/service_impl.rs`、`local_event_store/connection.rs`、`local_event_store/reader.rs`）。うち 4 箇所は `check()` / `wait()` の戻りを失敗の型の `Stopped` 変種へ入れるため、期限切れは `FailureKind::Expired` として `DEADLINE_EXCEEDED` になる。`connection.rs` の busy_handler は戻りが真偽値であり、期限切れを `.is_ok()` で潰して `false` を返すため、`SQLITE_BUSY` が `FailureKind::Temporary` に分類されて `UNAVAILABLE` になる。

## やり直しが usecase の作業列の中にあり、待ち時間の計算が domain にある

- `usecase/work_queue.rs`（644 行）が 1 つの型に、作業列の駆動（`enqueue*` / `dispatch`）、呼び出しを包んでやり直す関数（`retry` / `retry_stage` / `retry_with_scope` / `run_borrowed`）、単位時間あたりのやり直しの回数の上限（`acquire_retry`、`RetryBucket`）、失敗の記録と「要対応」の発行（`observe` / `clear_attention` / `publish_failure_change`）を持つ。
- 待ち時間の計算は `domain/retry.rs`（67 行）の `RetryBackoff`（`ITEM` / `RECOVERY` / `CONFLICT` / `SERVICE`）と `RetryBucket` にある。`domain/work_queue.rs`（141 行）の `retry` が `RetryBackoff` を引数で受け取り、`RetryAction::Restart` のときは `RetryBackoff::CONFLICT` へ差し替える。
- 包んでやり直す関数の非テストの呼び出しは 7 箇所である。usecase は 3 ファイルの 4 箇所（`workflow/control_plane.rs` の `retry_stage` 2 箇所、`workflow/command/mod.rs` の `retry` 1 箇所、`workflow/node_startup.rs` の `run_borrowed` 1 箇所）で、集約の版が競合したときに読み直して再実行する業務の手順である。adaptor は 2 ファイルの 3 箇所（`gateway/workflow/workflow_host.rs` の `retry_stage` 1 箇所、`gateway/provider_lifecycle/event_repository_impl.rs` の `retry_with_scope` 2 箇所）で、外部世界への呼び出しの一時的な失敗のやり直しである。
- 業務の手順をやり直すかは、呼び出し側が `retry`（手順の先頭からやり直す）と `retry_stage`（段階からやり直す）のどちらを呼ぶかで選び、実際にやり直すかは失敗の分類（`FailureKind::retry_action()`）が決める。
- `RetryBackoff` を参照する非テストのファイルは、`domain/work_queue.rs`、usecase 7 ファイル、adaptor 4 ファイルである。

## 計測が処理の中に直接書かれている

- 計測の包みは `other/telemetry::measure_result` の 1 つだけで、非テストの呼び出しは `adaptor/gateway/repository/status.rs` の 3 箇所である。
- 計測の記録点は非テストで 29 箇所ある（`usecase/telemetry.rs` と `adaptor/gateway/telemetry.rs` の画面向けの機能を除く）。受け手の側（controller 5 ファイル）に 7 箇所、Usecase の入口に 3 箇所、Usecase の中に 5 箇所、出ていく側（`adaptor/gateway/terminal_surface/runtime_gateway_impl.rs` 9 箇所、`adaptor/gateway/repository/status.rs` 4 箇所、`adaptor/gateway/workflow/workflow_host/lifecycle_commands.rs` 1 箇所）に 14 箇所である。このうち 17 箇所は terminal の起動の各段階と terminal 入力の遅延の内訳を取るもので、呼び出しの境界ではなく処理の内部の時点を測る。
- それ以外の計測は記録の呼び出しとして処理の中に直接書かれている。usecase では `terminal_surface/spawn_usecase.rs` 3 箇所、`terminal_surface/application.rs` 2 箇所、`agent_session/agent_session_launch.rs` 3 箇所が `other::telemetry` を直接呼ぶ。`spawn_usecase.rs` と `agent_session_launch.rs` の 6 箇所は、外部との接続の呼び出しを 1 つ以上挟む段階の所要時間を測る。`application.rs` の 2 箇所は、同時実行の枠を取った時点と client の送信時刻を記録するもので、外部との接続の呼び出しを挟まない。このうち client の送信時刻を記録する `start_terminal_input_trace` は terminal 入力の sample を作る唯一の場所であり、同時実行の枠を取った時点を記録する `record_terminal_input_admission` が無い sample は `take_terminal_input_samples` が捨てる。この 2 箇所のどちらかが欠けると、terminal 入力の遅延の内訳 9 項目すべてが空になる。gateway 側の 4 箇所（`adaptor/gateway/terminal_surface/runtime_gateway_impl.rs`）は、sample が存在するときだけ記録する。`application.rs` の `write_attached` の非テストの呼び出し元は `adaptor/controller/terminal_surface.rs` の 1 か所であり、この関数は Usecase の入口である。
- `usecase/telemetry.rs` は `TelemetryPort` trait と `TelemetryUsecase` を持ち、画面向けの機能（frontend のエラー報告、性能計測の sample の収集と取得、計測に関する設定の反映）を提供する。その trait の型に `other::telemetry` と `other::performance_switches` の型を使う。
- OTLP の送信そのものは `infrastructure/telemetry/`（`mod.rs` 189 行、`config.rs` 117 行、`crash.rs` 376 行）にあり、`init_telemetry` が exporter と provider を組み立て、`other::telemetry` の設定関数を呼ぶ。
- Tauri のシェルの `desktop.rs` が起動時の計測（`set_startup_origin`、`record_startup_from_origin`）で `other::telemetry` を呼ぶ。

# Scope / Non-goals

今回変更する対象。

- `src-tauri/src/common/` の新設と、期限・取り消し・やり直し・計測の包みの定義の配置。
- 期限・取り消しの包み化。定義（`domain/operation_context.rs`、`other/operation_context.rs`）と、掛け方（受け手の側は `adaptor/controller/api/client.rs`、出ていく側は `adaptor/gateway/shared/` の `file_lock.rs`・`process.rs`・`git_operation.rs` ほか `OperationContext` を引数で受け取る各所）。
- 一時的な失敗のやり直しの包み化。`adaptor/gateway/workflow/workflow_host.rs` 1 箇所と `adaptor/gateway/provider_lifecycle/event_repository_impl.rs` 2 箇所を、出ていく側の包みとして掛け、gateway の中で閉じること。`usecase/work_queue.rs` の `retry` / `retry_stage` / `retry_with_scope` / `run_borrowed`、`domain/retry.rs` の待ち時間の計算、`domain/work_queue.rs` が `RetryBackoff` を受け取る形を含む。
- やり直しの待ち時間の式と、単位時間あたりのやり直しの回数の上限の型と計算を common に置くこと。これらはやり直しの包みを構成するものであり、新しい種類の部品ではない。回数の上限の実体は Main が 1 つ作り、出ていく側の包みと、繰り返し処理を回す側の両方へ渡す。
- 集約の版の競合で読み直してやり直すときに `CONFLICT` の待ち時間を使う選択を、usecase が持つこと。
- 業務の手順のやり直しを Usecase の手順として残すこと。`usecase/workflow/control_plane.rs` 2 箇所、`usecase/workflow/command/mod.rs` 1 箇所、`usecase/workflow/node_startup.rs` 1 箇所は、集約の版が競合したときに読み直して再実行する業務の手順であり、common の包みにしない。
- 計測の包み化。受け手の側（controller 5 ファイルの 7 箇所）と出ていく側（`adaptor/gateway/repository/status.rs` の `measure_result`、`adaptor/gateway/workflow/workflow_host/lifecycle_commands.rs` の `record_workflow_node_failure`）に、common の包みとして掛けること。
- `docs/architecture/GATEWAY.md` の「出ていく側の横断的関心事」に計測を足すこと。
- 性能計測の sample の記録点の呼び先の変更。usecase の中の記録点（`usecase/terminal_surface/spawn_usecase.rs`、`usecase/agent_session/agent_session_launch.rs`、`usecase/terminal_surface/application.rs`）は Output Boundary の trait を通し、実装を adaptor 側に置く。gateway の中の記録点（`adaptor/gateway/terminal_surface/runtime_gateway_impl.rs`）は infrastructure の計測の仕組みを直接使う。
- `other/telemetry/` の中身の行き先。
- domain と usecase から期限・取り消し・やり直しを外すこと。domain 9 個の失敗の型の `Stopped` 変種と `From<OperationStopped>`、usecase の非テストの参照 5 ファイルを含む。domain が受け取るのは「技術的に失敗した」という事実だけにする。
- 出ていく側の自前の期限（`adaptor/gateway/git_host/github.rs`、`comment/mod.rs`、`notion/service_impl.rs`、`local_event_store/reader.rs` の 4 箇所）が切れたときに、gateway がそれを翻訳して、port の失敗の型の技術的な失敗の変種として内側へ返すこと。技術的な失敗の変種を持たない失敗の型には、変種を 1 つ足す。
- `other/` の各ファイル（`dispose.rs`、`error.rs`、`id.rs`、`operation_context.rs`、`performance_switches.rs`、`telemetry/`、`utils.rs`）を、規約の部品の一覧に従った行き先へ移すこと。`error.rs` の `AppError` は、中身（`FailureKind` と `Serialize` でのフロントへの表現）を変えずに `adaptor/presenter/` へ移す。移すために必要な、usecase から使う能力の trait 化と、trait が使う型の置き場所の変更を含む。
- 上記に伴い使われなくなるコードの削除。

今回変更しない対象。

- 失敗の分類の形。`FailureKind` と `ClassifiedFailure` の廃止、ステータスコードへの対応付けの presenter への移動は #1929 が扱う。
- 出ていく側の自前の期限切れを、最終的にどのステータスコードで返すか。#1928 では技術的な失敗の変種に `FailureKind::Expired` を持たせて今と同じ結果にし、#1929 が決める。
- `adaptor/gateway/local_event_store/connection.rs` の busy_handler が持つ自前の期限（2 秒）が切れたときの、技術的な失敗の変種への翻訳。rusqlite の busy_handler の戻りは真偽値であり、期限切れを戻り値で運べない。包み化の前と同じ結果のままにする。
- 繰り返し処理を回す入口と、失敗の記録の分解。作業列の駆動（`enqueue*` / `dispatch`）、`domain/work_queue.rs`・`domain/failure_records.rs`・各ドメインの `background_failure.rs` の廃止は #1930 が扱う。作業列とそこへ仕事を積む経路がやり直しの待ち時間の式と回数の上限を持つことも、#1930 で作業列が usecase から出るときに解消する。
- 状態の配信の仕組みの移動と `adaptor/protocol/` の presenter への移動は #1931 が扱う。
- Tauri のシェルのコード。`other/` が無くなることによる参照先の付け替えだけは行う（`src-tauri/src/desktop.rs` が `other::telemetry` を呼ぶ箇所）。シェルの振る舞いは変えない。
- 画面（`src/`）のコード。この milestone の「層の整理」の対象はサーバのコードだけである。
- 期限と取り消しを受け取る仕組みと、既定の期限の値（120 秒）。#1883 が扱う。
- CLI / hook 用の HTTP local API が呼び出しの期限を受け取る仕組み。#1883 で対象外と決定済みである。
- 期限を引き継ぐ対象と、処理の先が持つ期限の値。#1893 が扱う。
- やり直しの待ち時間の式の値、やり直す分類の判定、単位時間あたりのやり直しの回数の上限の値。#1890 が扱う。
- `FailureKind::retry_action()` を業務の失敗（版の競合）へ置き換えること。#1929 が扱う。#1928 では `retry_action()` を見たままにする。
- 同時実行の枠の優先度。#1894 が扱う。
- 計測の項目を増やすこと。
- `usecase/telemetry.rs` が提供する画面向けの機能（frontend のエラー報告、性能計測の sample の収集と取得、計測に関する設定の反映）。これは機能であり横断的関心事の包みではない。
- 性能計測の sample の収集（terminal の起動の各段階、terminal 入力の遅延の内訳）を包みにすること。処理の内部の時点を測る機能であり、包みの位置からは観測できない。記録する時点と値の意味は変えず、呼び先だけを R-011 のとおりに変える。
- 規約（`docs/architecture/`）の部品の一覧そのものの変更。

# Requirements

- R-001: 期限・取り消し・やり直し・計測の包みの定義は `src-tauri/src/common/` にあり、common は他のどの層も参照しない。
- R-002: 期限・取り消し・やり直し・計測は処理の中に書かれず、関心事ごとに決まった位置で包みとして掛かる。取り消しは受け手の側（controller の手前）に掛かる。期限は受け手の側と出ていく側（adaptor/gateway と infrastructure）に掛かる。一時的な失敗のやり直しは出ていく側に掛かり、gateway の中で閉じて内側へは結果だけを返す。計測の包みは、受け手の側・Usecase の入口・出ていく側以外の位置には掛からない。
- R-003: domain の型・失敗・trait は、期限・取り消し・やり直しを受け取らない。domain は common を参照しない。
- R-004: usecase の Usecase・Input Data・Output Data・trait は、期限・取り消し・やり直しを受け取らない。集約の版の競合で業務の手順をやり直すかの判断は usecase が持つ。繰り返し処理を回す作業列と、そこへ仕事を積む経路は、作業列が #1930 で usecase から出るまでの一時的な例外とする。
- R-005: `src-tauri/src/other/` は無くなり、そこにあった各部品は規約の部品の一覧のいずれかの部品として置かれる。
- R-006: 期限切れまたは client の中断で終わった呼び出しの結果は変わらない。期限切れは `DEADLINE_EXCEEDED`、中断は `CANCELLED` で終わる。
- R-007: 呼び出しの期限と取り消しが store の問い合わせ・外部プロセスの実行・lock の待ちへ引き継がれることと、止まった処理が外部プロセスとファイルの lock を残さないことは変わらない。
- R-008: やり直しの待ち時間の決まり方、単位時間あたりのやり直しの回数の上限、失敗の記録、対象が「要対応」であることの表示は変わらない。
- R-009: 計測の項目と、計測した値の意味は変わらない。
- R-010: この変更で使われなくなるコードと、この変更で触れたファイルの中で使われていないコードは無い。
- R-011: 性能計測の sample の記録点は、usecase では Output Boundary の trait を通り、gateway では infrastructure の計測の仕組みを使う。usecase は計測の実装を参照しない。

# Assumptions / Open Questions

人間が明示的に受け入れた仮定は無い。未確定事項は無い。
