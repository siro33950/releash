# Design

## 変える部分
- gateway・infrastructure の付け替え先の error の型: `ProviderAgentTerminalGatewayError`・`AgentSessionHistoryGatewayError`・`ProviderSessionTitleGatewayError`・`ProviderExecutableConfigRepositoryError`・`ProviderAgentLaunchGatewayError`・`ProviderHookHealthFailureQueryError`・`ProviderHookHealthMarkerError`・`ProviderLaunchFilesError` の中身を持たない `Unavailable` を、元の性質とメッセージを持つ技術的な失敗（`TechnicalFailure`）に置き換え、各付け替え箇所で元の失敗から性質を決める。根拠: R-001「技術的な失敗なら元の性質（…）とメッセージを持ったまま」、B-001。ルート: 委任
- usecase の付け替え先の error の型: `AgentSessionLifecycleUsecaseError`・`AgentSessionLaunchUsecaseError` の `LaunchUnavailable`・`TerminalUnavailable`、`ProviderAvailabilityUsecaseError` の `ConfigUnavailable`・`RefreshUnavailable` を、依存先ごとの変種が下の層（domain の trait）の失敗をそのまま持つ形にする。根拠: R-001、R-003「文言（…）は今のまま」、B-003。ルート: 下記「固定するルート」1
- terminal の domain の失敗: `TerminalSurfaceGatewayError` を業務の結果と `Technical(TechnicalFailure)` の形にし、`adaptor/gateway/terminal_surface/runtime_gateway_impl.rs` の作り元 23 か所で、対象が無い失敗を業務の結果に、外部の失敗を元の性質を持つ技術的な失敗にする。根拠: R-004、B-004、B-005。ルート: 下記「固定するルート」2
- terminal の usecase の失敗: `usecase/terminal_surface/error.rs` の `Gateway(String)` を業務の失敗と技術的な失敗に分け、`From<TerminalSurfaceGatewayError>` でメッセージだけを写す形をやめる。根拠: R-004、B-004、B-005。ルート: 下記「固定するルート」2
- terminal の起動の失敗: usecase の `PtySpawn`・`OtherSpawnFailure` を業務の結果と `Technical(TechnicalFailure)` の形にし、domain の `ProviderAgentTerminalSpawnError` を消して spawn の失敗も `ProviderAgentTerminalGatewayError` で表す。`usecase/terminal_surface/spawn_usecase.rs` の 6 か所と `map_spawn_error`（`adaptor/gateway/agent_session/provider_agent_terminal_gateway.rs`）でメッセージだけを写す形をやめる。画面の文言・ステータスコード・Node の失敗の分類は、spawn の失敗を別の型で表していたときと変わらない。根拠: R-004「terminal の起動の経路でも同じ」、B-005。ルート: 下記「固定するルート」3
- terminal の終了中の操作の拒否: `usecase/terminal_surface/application.rs` の mutation_rejected（domain の TerminalSurfaceRuntimeLifecycle::admit_mutation が ShuttingDown の状態で返す拒否）は、「今の状態では受け付けない操作」にあたる業務の失敗として UsecaseError::InvalidOperation で表す。画面に返るステータスコードは INTERNAL から FAILED_PRECONDITION に変わる。この 2 つはどちらも画面のつなぎ直しの対象のコード（UNAVAILABLE・ABORTED・RESOURCE_EXHAUSTED）ではないため、つなぎ直す・つなぎ直さないは変わらない（`src/lib/client.ts` の reconnects は、この対象のコードのときだけつなぎ直す）。根拠: R-001「業務の失敗なら業務の結果として」、R-002、R-009、B-002、B-011。ルート: 委任
- presenter のステータスコードの対応: 付け替えた後の分類から固定のコードを出している対応（`AgentSessionLifecycleUsecaseError`・`AgentSessionLaunchUsecaseError`・`ProviderAvailabilityUsecaseError`・各 gateway の失敗・terminal の `UsecaseError`。terminal の起動の失敗の一律の `FAILED_PRECONDITION` と terminal の一律の `INTERNAL` を含む）を、中の失敗に従う形にする。根拠: R-002、B-001、B-002、B-003。ルート: 委任
- 起動時の store を開く失敗: `adaptor/gateway/local_event_store/store.rs` の 16 か所で、既存の分類の部品（`classify_sqlite_error`・`classify_connection_error`・`classify_writer_lock_error`）で分けられる失敗はその分類にし（reader の接続の `ConnectionError` を `classify_connection_error` に通す等）、それ以外の技術的な失敗は `LocalEventStoreOpenError::StorageUnavailable` に元の性質とメッセージを持たせる。根拠: R-005、B-006。ルート: 下記「固定するルート」4
- 起動の失敗の `retry_on_next_launch`: `StartupFailureKind::StorageUnavailable` に性質を持たせ、`retry_on_next_launch` を `StorageUnavailable` のとき性質で決める。根拠: R-006、B-007、B-008。ルート: 下記「固定するルート」4
- 失敗の性質の判定: 起動時の SQLite の失敗と io::Error の性質を、実行中と同じ判定の関数を呼んで決める。根拠: R-007、B-009。ルート: 下記「固定するルート」5
- 保存側の `Unavailable` からの写し: 付け替え先の型へ写すとき、保存側の `Unavailable`・`StorageUnavailable`（`AgentSessionRepositoryError` 等）は `Technical { nature: Transient }` に写す。根拠: R-001、B-010。ルート: 下記「固定するルート」6

## 固定するルート
1. usecase のエラー型は、依存先ごとの変種が下の層（domain の trait）の失敗を分類し直さずにそのまま持つ形にする（例: `Launch(ProviderAgentLaunchGatewayError)`、`Terminal(ProviderAgentTerminalGatewayError)`）。技術的な失敗を 1 つの変種にまとめ、業務の失敗を usecase の変種へ写し直す形にはしない。理由: 業務の失敗の側で分類し直すことになるため。presenter は変種で画面の文言を決め、中の失敗でステータスコードを決める。
2. `TerminalSurfaceGatewayError`（`domain/terminal_surface/gateway.rs`）を業務の結果と `Technical(TechnicalFailure)` の形（`DOMAIN.md`「失敗」）に変える。`runtime_gateway_impl.rs` の 23 か所の作り元で、対象が無い失敗は業務の結果に、外部の失敗（checkpoint のファイル操作、PTY の resize・kill 等）は元の性質を持つ技術的な失敗にする。usecase の `Gateway(String)` も同じく分ける。性質の分からない失敗を一律に `Other` にする形は採らない。
3. terminal の起動の失敗（usecase の `PtySpawn`・`OtherSpawnFailure`）も業務の結果と `Technical(TechnicalFailure)` の形にする。domain では、spawn の失敗も `ProviderAgentTerminalGatewayError` で表し、同じ変種を持つ別の型（`ProviderAgentTerminalSpawnError`）を置かない。presenter の一律の `FAILED_PRECONDITION`（terminal の起動の失敗）と `INTERNAL`（terminal の `UsecaseError`）を、中の失敗に従う形にする。spawn の失敗を 1 つの型にまとめても、画面の文言・ステータスコード・Node の失敗の分類は変わらない。
4. 起動時の store の失敗は、既存の分類の部品で分けられるものをその分類にする。それ以外は `StorageUnavailable` に性質とメッセージを持たせる。`StartupFailureKind` は中身を持たない `Copy` の enum から、`StorageUnavailable` が性質を持つ形に変える。`retry_on_next_launch` は `StorageUnavailable` なら `Transient`・`TimedOut` で true、それ以外で false、`StoreInUse`・`SchemaEvolutionFailed` は true とする。
5. 性質の判定は、SQLite の失敗は `adaptor/gateway/local_event_store/commit.rs` の判定（busy は `Transient`、inaccessible は `Other`）、io::Error は `adaptor/gateway/shared/background_io.rs` の判定と同じ関数を呼んで使い、対応を写さない。今の判定が `commit.rs`・`reader.rs` の中に閉じていて呼べない場合は、1 か所にまとめて起動時と実行中の両方から呼ぶ。理由: 同じ失敗が場面によって違う性質にならないようにするため（`docs/architecture/README.md`「同じ操作の実装は 1 つに集約する」）。
6. 保存側の 4 つの型（`AgentSessionRepositoryError::Unavailable`・`ProviderLifecycleRepositoryError::StorageUnavailable`・`ProviderHookHealthRepositoryError::StorageUnavailable`・`AgentSessionQueryError::Unavailable`）の変種と、`adaptor/gateway/shared/storage_failure.rs` の変換は変えず、付け替え先の型へ写すときに `Technical { nature: Transient }` に写すだけにする。

## 変えないもの
- 保存側の 4 つの型の `Unavailable`・`StorageUnavailable` 変種と、`storage_failure.rs` の対応する変換。技術的な失敗の変種を複数持つ問題で、マイルストーン #97 で扱うため。
- 付け替え先として `Unavailable` を `TechnicalFailure` に置き換える domain の型でも、ほかの技術的な変種（`Store`・`Corrupt` 等。例: `domain/agent_session/provider_history_gateway.rs:19-26`）はまとめない。技術的な変種を複数持つ形はマイルストーン #97 で扱うため。
- usecase が依存先ごとのエラーの enum を持つこと。usecase が独自のエラーの型を持つ問題で、マイルストーン #97 で扱うため。
- 画面のつなぎ直しの対象のステータスコード（proto の `reconnect_status_code`）。#1951 で proto の規則として決めたもので、この開発の範囲の外であるため。
- AgentSession の Provider の操作と Terminal の操作の失敗の画面の文言（`adaptor/presenter/provider_tui.rs`）。

## 未確定・リスク
なし
