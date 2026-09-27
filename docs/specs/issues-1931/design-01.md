# Design 01

## 開始状態

初回である。差分の基準は `main` の `9aceb249`（#1929 の実装 PR #1935）であり、作業ブランチ `feat/issues/1931` の派生点も同じ commit である。開始時点の実装は `requirements.md` の Current Behavior を参照する。この周までに閉じた Thread は無い。

## 変える部分

- 配信の仕組みの移動: `domain/state_subscription/`（`mod.rs` 671 行、`subscription_target.rs` 262 行）と `domain/terminal_surface/value_objects/output_flow_control.rs`（65 行）を domain から出し、presenter と infrastructure へ置く。`domain/state_subscription/` は無くなる。根拠: R-001、R-003、B-001、B-003。ルート: 送り待ちと送る量の制御は infrastructure に置く。それ以外の分け方は委任
- Output Boundary の新設: 状態の配信のための Output Boundary の trait を usecase に置き、adaptor/presenter が実装する。`StateSubscriptionPublisher` を usecase の Usecase と adaptor の各所が直接持つ形を、この trait の呼び出しへ置き換える。根拠: R-002、B-002。ルート: 委任
- 画面へ届ける口の付け替え: `RepoPathsNotifier`、`TerminalSurfaceStateSink`、`TerminalSurfaceEventSink`、`TerminalSurfaceEventSource` のうち画面への配信に当たる部分、`AgentSessionChangeNotifier`、`RepositoryStateNotifier` を usecase の Output Boundary にし、adaptor/presenter が実装する。根拠: R-002、R-003、B-002、B-003。ルート: 対象はこの 6 つに限る。`PerformanceOutput` は含めない
- 購読の手順の維持: 購読の開始・停止・変更検知後の再読み取り・watch の調整の手順は usecase の Usecase として残す。読み取りは usecase の `StateSubscriptionRead` を通したままにする。根拠: R-007、B-007。ルート: 人間が固定（下記「固定するルート」）
- `adaptor/protocol/` の廃止: `connect.rs` を含む全ファイルを adaptor/presenter へ移す。`adaptor/gateway/workflow/` の 5 ファイルが `adaptor::protocol::workflow` 経由で参照している `usecase::workflow::diagnostic_dto` の型は、usecase から直接参照する形にする。根拠: R-004、R-005、B-004、B-005。ルート: 人間が固定（下記「固定するルート」）
- controller の側の stream の処理: `controller/api/state_subscription.rs`（158 行）の `StateValue` と `Event` から転送の形への変換、`controller/api/client_stream.rs`（22 行）の転送の失敗の組み立て、`controller/api/client_service.rs` の購読の入口での転送の形の組み立てを presenter へ移す。根拠: R-004、B-004。ルート: 委任
- push のメッセージの組み立て: `adaptor/gateway/push.rs` の `BackendPush::emit` が組み立てる `wire::Push` の変換を presenter へ移す。gateway は通知の契約を呼ぶだけにする。根拠: R-004、R-011、B-004、B-014。ルート: 人間が固定（下記「固定するルート」）
- HTTP local API のリクエスト型の集約: `adaptor/protocol/provider_lifecycle.rs` の `ProviderLifecycleReceiveRequest`・`ProviderLifecycleSignalRequest`・`ProviderLifecycleUnavailableRequest`・`ProviderLifecycleProvider`・`ProviderActivityRequest`・`ProviderLifecycleUnavailableReasonRequest` と、`adaptor/protocol/workflow.rs` の `WorkflowSubmitArtifactInput` を `adaptor/controller/api/protocol.rs` へ移す。根拠: R-006、B-006。ルート: 人間が固定（下記「固定するルート」）
- HTTP local API のレスポンスの presenter 化: `StartExecutionResponse`・`MutationResponse`・`ValidateArtifactResponse`・`GetArtifactResponse`・`ProviderLifecycleReceiveResponse` と、その変換を presenter へ移す。根拠: R-006、B-006。ルート: 委任
- Output Data をそのまま JSON にしている 4 応答の変換の新設: `GET /workflows`、`GET /workflows/executions`、`GET .../log`、`GET /workflows/diagnostics` に対応する転送の型を presenter に置き、`usecase::workflow::dto::WorkflowSummaryDto`・`WorkflowExecutionSummaryDto`、`usecase::workflow::query_service::WorkflowEventView`、`usecase::workflow::diagnostic_dto` の各型から serde の属性を外す。JSON のフィールド名と形は変えない。`cli/diagnostics.rs` の deserialize は presenter の型を使う形にする。根拠: R-006、R-008、B-006、B-009。ルート: 人間が固定（下記「固定するルート」）
- 使われなくなるコードの削除: 上記に伴って使われなくなるコードを消す。根拠: R-012、B-015。ルート: 委任

## 固定するルート

- 購読の手順は usecase の Usecase として残す。usecase から出すのは配信の仕組み（版、送り待ち、bookmark、送る量の制御）と転送の形への変換だけであり、「購読を開始する → 読む → Output Boundary へ出す」「変更を受けて読み直して出す」という手順は usecase に置く。`StateSubscriptionRead` は usecase の trait のままにし、usecase の外から呼ばない。
- HTTP local API のリクエスト型は `adaptor/controller/api/protocol.rs` に置く。`CONTROLLER.md`「local API」の「リクエスト型は `api/protocol.rs` に置く」に従う。レスポンスの型と変換は presenter に置く。
- push は変換だけを presenter へ移す。送信機構（`infrastructure/push.rs` の `PushSink`）と controller の購読の入口（`client_service.rs` の `subscribe_push`）は変えない。gateway から presenter の転送の型を参照する形を作らない。
- Output Boundary へ寄せる画面へ届ける口は、`RepoPathsNotifier`、`TerminalSurfaceStateSink`、`TerminalSurfaceEventSink`、`TerminalSurfaceEventSource` のうち画面への配信に当たる部分、`AgentSessionChangeNotifier`、`RepositoryStateNotifier` の 6 つに限る。
- `adaptor/protocol/connect.rs` も presenter へ移し、`adaptor/protocol/` を無くす。正本が `connect.rs` を対象外に置いた理由（#1929 が扱う）は、#1929 の merge で満たされている。
- usecase の Output Data をそのまま JSON にしている HTTP local API の 4 応答も、この周で presenter の変換に置き換える。usecase の Output Data から serde の属性を外す。

## 変えないもの

- `PerformanceOutput`（`usecase/telemetry.rs:120`）の実装が `adaptor/gateway/telemetry.rs:69` にあること。規約と合っていないが、この周では変えない。所見として別に報告する。
- `infrastructure/push.rs` の `PushSink` と、`controller/api/client_service.rs` の `subscribe_push`。push の送信機構と購読の入口は今のままにする。
- 転送のメッセージ、JSON のフィールド名と形、Connect のステータスコード、HTTP status。画面と CLI が受け取るものを変えないため。
- 画面（`src/`）のコード。
- `docs/architecture/` の記述。この周で決めたことは、いずれも既存の規定の範囲に収まる。

## 未確定・リスク

- `TerminalSurfaceEventSource` は、画面への配信の口（`set_state_sink`・`subscribe_output`・`unsubscribe_output`・`processed_output`）と、daemon の中で terminal の終了を観測する口（`subscribe`）を 1 つの trait に持つ。前者だけを Output Boundary へ寄せるため、分け方を誤ると `adaptor/controller/agent_session_exit_observer.rs` の終了の観測が動かなくなる。
- 配信の仕組みを infrastructure へ置くには、infrastructure が内側の層を参照しない形（購読の対象の識別子と Output Data を型引数で受ける形）に組み直せることが前提になる。組み直せない場合、R-001 の「送り待ちと送る量の制御は infrastructure にある」を満たせない。
- `usecase::workflow::diagnostic_dto` の各型から serde の属性を外すと、これらの型を serde で扱う経路が残っていた場合に壊れる。非テストで確認できた利用は HTTP local API の 4 応答と `cli/diagnostics.rs` の deserialize だけだが、`DiagnosticItem` などを使う `adaptor/gateway/workflow/` の 5 ファイルに serde の利用が残っていないかの確認が要る。
