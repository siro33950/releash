# Context

- 正本: [#1931 `[06] 状態の配信を Output Boundary と presenter に移す`](https://github.com/siro33950/releash/issues/1931)
- 所属: [milestone #99「02. UI と daemon の間の通信の仕組みを一本化する」](https://github.com/siro33950/releash/milestone/99) の Wave 06。milestone の「層の整理（規約 `docs/architecture/` に合わせる。対象はサーバのコードだけ）」の 4 件（#1928・#1929・#1930・#1931）の 3 番目である。
- 規約の正本は `docs/architecture/`。この変更が根拠にする規定は次のとおり。
  - `README.md`「部品の一覧」: 「usecase / Output Boundary の trait / 結果を外へ出す口。画面への状態の配信もここを通る。adaptor/presenter が実装する」。「adaptor/presenter / Presenter、転送のメッセージ型 / Output Data を転送の形とステータスコードに変える」。「置いてよい部品は次の表のものだけである。表に無い部品は置かない」。
  - `README.md`「依存方向」: 「adaptor/presenter は usecase の Output Boundary を実装する」。「infrastructure は内側のどの層にも依存しない」。
  - `DOMAIN.md`「原則」: 「ビジネスロジック専念: ドメイン固有の概念・不変条件・状態遷移を表現する。通信・保存・実行制御の仕組みは、状態や判断を持っていても業務の概念ではないので、domain に置かない」。
  - `DOMAIN.md`「trait（Repository、ドメインサービス）」: 「domain が定義する trait は、Repository と、外部世界を必要とするドメインサービスの 2 種類だけである」。「Output Data を返す読み取りの口（QueryService）と、結果を外へ出す口（Output Boundary）は usecase に置く。domain の trait と混同しない」。
  - `USECASE.md`「Output Boundary」: 「Usecase が結果を外へ出す口。usecase が trait として定義し、adaptor/presenter が実装する。画面への状態の配信も、この口を通す。引数は Output Data であり、転送の形を持ち込まない」。
  - `PRESENTER.md`「原則」: 「画面への状態の配信（購読）も、Output Boundary を通して届いたものを転送の形に変えて送る。送る仕組みそのもの（接続、送り待ち、送る量の制御）は infrastructure と common の包みを使う」。
  - `PRESENTER.md`「転送のメッセージ型」: 「`proto/client.proto` のメッセージ型と、複数の入口で共有する転送の型は presenter に置く」。
  - `CONTROLLER.md`「local API」: 「リクエスト型は `api/protocol.rs` に置く。レスポンスへの変換（`ApiError` を含む）は presenter が行う」。
  - `CONTROLLER.md`「失敗」: 「Usecase の結果（失敗を含む）を転送の形とステータスコードに変えるのは presenter である。controller は変えない」。
  - `INFRASTRUCTURE.md`「原則」: 「外部世界の都合を、その形のまま扱う」「変換しない」「内側の層を知らない」。
- 依存: #1929。`9aceb249`（PR #1935）で merge 済みである。#1929 が残した前提は次のとおり（`docs/specs/issues-1929/requirements.md`）。
  - 転送のステータスコードへの対応付けは `src-tauri/src/adaptor/presenter/connect.rs` と `src-tauri/src/adaptor/presenter/api_error.rs` に集約済みである。
  - `AppError` は `src-tauri/src/adaptor/presenter/error.rs` にある。
  - domain の失敗は業務の失敗と技術的な失敗（`TechnicalFailure`）に分かれている。
- 基準は `main` の `9aceb249`。作業ブランチ `feat/issues/1931` の派生点も同じ commit である。Current Behavior はこの commit で確認した。
- 正本の「今の作り」の記述は `5c64534d` を基準にしており、`9aceb249` では次の 2 点が成立しない。Current Behavior は `9aceb249` の事実で置き換える。
  - 正本は「転送の形への変換と Connect のエラーへの変換が `src-tauri/src/adaptor/protocol/` にある」とするが、Connect のエラーへの変換は #1929 で presenter へ移っている。`adaptor/protocol/connect.rs`（18 行）に残るのは、生成された Connect のスタブの取り込みと `to_wire` / `to_rpc` だけである。
  - 正本は「HTTP local API のリクエスト・レスポンスの型が `src-tauri/src/adaptor/controller/api/protocol.rs` と `src-tauri/src/adaptor/controller/api/protocol/` にある」とするが、`api/protocol/` にあるのは `connect.rs`（2 行、Connect の再 export）だけであり、HTTP local API の型は無い。
- 正本が「#1929 が扱う」として `adaptor/protocol/connect.rs` を対象外に置いた理由は、#1929 の merge で満たされている。この変更では `connect.rs` も presenter へ移す。
- 参照する既存実装: `src-tauri/src/domain/state_subscription/`、`src-tauri/src/domain/terminal_surface/value_objects/output_flow_control.rs`、`src-tauri/src/domain/terminal_surface/gateway.rs`、`src-tauri/src/domain/repository/gateway.rs`、`src-tauri/src/usecase/state_subscription.rs` と `src-tauri/src/usecase/state_subscription/`、`src-tauri/src/usecase/agent_session/agent_session_change_notifier.rs`、`src-tauri/src/usecase/repository_state/worktree.rs`、`src-tauri/src/adaptor/protocol/`、`src-tauri/src/adaptor/controller/api/`、`src-tauri/src/adaptor/gateway/push.rs`、`src-tauri/src/adaptor/gateway/repository/`、`src-tauri/src/infrastructure/push.rs`。

# Outcome

対象者は、daemon を実装・保守する開発者である。Releash の UI と CLI を使う利用者は、この変更の前後で同じ振る舞いを見る。

現在、状態の配信の仕組み（版、送り待ち、bookmark、送る量の制御）は domain に置かれ、その実体を usecase が直接持って操作している。画面へ届ける口は、domain の trait として定義されているものと、usecase の trait を gateway が実装しているものに分かれている。転送の形への変換は `adaptor/protocol/` にあり、その呼び出しは controller・gateway・presenter に散っている。stream の要素ごとの変換は controller にある。HTTP local API の応答は、変換を持つものと、usecase の Output Data をそのまま JSON にするものの 2 通りで作られている。結果として、通信の仕組みが業務の言語に混ざり、同じ「外へ出す」という行為が層をまたいで別々の形で書かれている。

変更後は、usecase が外へ出す口は Output Boundary の trait だけになり、adaptor/presenter がそれを実装する。配信の仕組みは presenter と infrastructure にあり、domain と usecase には無い。転送の形への変換は presenter の 1 か所にあり、`adaptor/protocol/` は無くなる。HTTP local API はリクエスト型を controller に、レスポンスの型と変換を presenter に持つ。利用者から見た画面の表示、CLI の出力、転送のメッセージと JSON の形、ステータスコードは変わらない。

# Current Behavior

`9aceb249` のコードで確認した挙動である。

## 状態の配信の仕組みが domain にある

- `src-tauri/src/domain/state_subscription/mod.rs`（671 行）が、版（`Version` の epoch と sequence）、配信の種類（`Delivery::Full` / `Delta`）、配信の要素（`Event::Snapshot` / `Change` / `Bookmark`）、target ごとの履歴（`RETAINED_CHANGES` = 64）と snapshot、client ごとの送り待ちの列と送信済みの版、overflow の判定、続きから再開できるかの判定（`resumable`）、bookmark の発行、次に送る要素の取り出し（`next`）を持つ。
- `src-tauri/src/domain/state_subscription/subscription_target.rs`（262 行）が、購読の対象の識別子（21 種類）、その文字列表現と解析、対象が要求する watch（`WatchRequirement::Git` / `Files`）、変化の発生源（`StateChangeSource` の 9 種類）と対象との対応（`affected_by`）を持つ。`SubscriptionTarget::Terminal` は `domain::terminal_surface::TerminalSurfaceOwner` を、`from_parts` は `domain::workspace_tree::WorkspaceIdentity` を使う。
- 転送のステータスコードへの対応付けは、`domain::state_subscription::SubscriptionError` に対する `ConnectFailure` の実装として `src-tauri/src/adaptor/presenter/connect.rs:302` にある。
- `domain::state_subscription` を参照する非テストのファイルは 20 である。内訳は domain 3（定義そのもの）、usecase 7、adaptor 10 である。

## 送る量の制御が domain にある

- `src-tauri/src/domain/terminal_surface/value_objects/output_flow_control.rs`（65 行）が、client ごとの未処理量、上限（`OUTPUT_HIGH_WATERMARK` = 100,000）と下限（`OUTPUT_LOW_WATERMARK` = 5,000）、報告の単位（`OUTPUT_REPORT_UNITS` = 5,000）、送り待ちの上限（`OUTPUT_PENDING_LIMIT` = 200,000）、一時停止するかの判定を持つ。
- `OUTPUT_PENDING_LIMIT` と `OUTPUT_REPORT_UNITS` は `src-tauri/src/usecase/state_subscription/terminal.rs` と `src-tauri/src/adaptor/protocol/client/mod.rs` が参照する。

## 購読の手順が配信の仕組みの実体を直接持っている

- `src-tauri/src/usecase/state_subscription.rs`（437 行）の `StateSubscriptionUsecase` と `StateSubscriptionPublisher` が、`Arc<Mutex<Subscriptions<StateValue>>>` を直接持ち、`register` / `start` / `publish` / `publish_delta` / `set_delta_snapshot` / `bookmark` / `next` を呼ぶ。
- `StateSubscriptionUsecase::open` は、配信の要素を 1 件ずつ取り出す `Stream` を組み立て、bookmark の間隔（`BOOKMARK_INTERVAL` = 10 秒）のタイマーを持ち、overflow した購読の snapshot の作り直しを spawn する。
- `src-tauri/src/usecase/state_subscription/terminal.rs`（359 行）が、terminal の版の epoch を `boot:runtime_generation` の形で組み立て、送り待ちの量を数えて送る量の制御へ渡し、`TerminalSurfaceStateSink` を `StateSubscriptionPublisher` に対して実装する。
- `src-tauri/src/usecase/state_subscription/value.rs`（32 行）の `StateValue` が、21 種類の購読の対象に対応する Output Data を束ねる。
- `src-tauri/src/usecase/state_subscription/reads.rs`（341 行）が、読み取りの trait（`StateSubscriptionRead`）と、その実体（`WorkspaceStateReads`）と、読み取りの失敗（`StateReadError` / `StateReadFailure`）を持つ。

## 画面へ届ける口が 2 通りの場所に定義されている

- domain の trait として定義されているもの。`RepoPathsNotifier`（`src-tauri/src/domain/repository/gateway.rs:14`）、`TerminalSurfaceStateSink`（`src-tauri/src/domain/terminal_surface/gateway.rs:93`）、`TerminalSurfaceEventSink`（同 `:100`）、`TerminalSurfaceEventSource`（同 `:139`）。`TerminalSurfaceEventSource` は、画面への配信のための `set_state_sink` / `subscribe_output` / `unsubscribe_output` / `processed_output` と、daemon の中で terminal の終了を観測するための `subscribe` を 1 つの trait に持つ。
- usecase の trait として定義され、gateway が実装しているもの。`AgentSessionChangeNotifier`（`src-tauri/src/usecase/agent_session/agent_session_change_notifier.rs`、実装は `src-tauri/src/adaptor/gateway/push.rs:123`）、`RepositoryStateNotifier`（`src-tauri/src/usecase/repository_state/worktree.rs:34`、実装は `src-tauri/src/adaptor/gateway/repository/state.rs:307`）。
- どちらの実装も `StateSubscriptionPublisher` を直接持ち、`invalidate` と `publish` を呼ぶ。`ClientRepositoryStateNotifier` は加えて push も送る。
- usecase の 6 ファイル（`workspace_state/usecase.rs`、`agent_session/provider_availability.rs`、`repository_usecase.rs`、`work_queue.rs`、`git_host/git_host_usecase.rs`、`workflow/execution_archive.rs`）が `StateSubscriptionPublisher::invalidate` を、domain の `StateChangeSource` を引数にして直接呼ぶ。

## 転送の形への変換が presenter の外にある

- `src-tauri/src/adaptor/protocol/`（非テストで 14 ファイル）が、Connect の生成メッセージ型と変換（`client/mod.rs` 120 行、`client/conversions.rs` 4,660 行、`client/errors.rs` 84 行、`client/workflow_values.rs` 74 行、`client/json.rs` 356 行）、HTTP local API と CLI が共有する型（`workflow.rs` 248 行、`provider_lifecycle.rs` 120 行）、Tauri と Connect の入出力の型（`code.rs` 260 行、`notion.rs` 357 行、`agent_session.rs` 92 行、`terminal.rs` 221 行、`application_lifecycle_v1.rs` 73 行）、生成スタブの取り込みと encode / decode（`connect.rs` 18 行）を持つ。
- `adaptor::protocol` を参照する非テストのファイルは 39 である。内訳は adaptor/controller 16、adaptor/gateway 10、adaptor/presenter 3、adaptor/protocol 2、cli 5、lib.rs 1、usecase 1（`#[cfg(test)]` の中のみ）である。
- gateway が転送の形を組み立てている箇所がある。`src-tauri/src/adaptor/gateway/push.rs:13` の `BackendPush::emit` が `wire::Push` を組み立て、`infrastructure/push.rs` の `PushSink` へ渡す。
- `adaptor/gateway/workflow/` の 5 ファイルが `adaptor::protocol::workflow` から `DiagnosticItem` / `DiagnosticSpan` / `DiagnosticStage` / `Severity` を import している。これらは `usecase::workflow::diagnostic_dto` の再 export である。

## controller が stream の要素ごとに転送の形へ変換している

- `src-tauri/src/adaptor/controller/api/state_subscription.rs`（158 行）が、`StateValue` の 21 種類すべてを `wire::StatePayload` へ変換し、`Event` を `rpc::StateSubscriptionEvent` へ変換する。変換の失敗は `adaptor::presenter::connect` の関数で転送の失敗にする。
- `src-tauri/src/adaptor/controller/api/client_stream.rs`（22 行）が、識別子の長さ（128 バイト）の検査と、`AppError` からの転送の失敗の組み立てを持つ。
- `src-tauri/src/adaptor/controller/api/client_service.rs` が、購読の開始・停止・報告の入口で `SubscriptionTarget::from_parts` と `Version` を組み立て、`open_state_stream` で stream の各要素へ変換関数を掛ける。`subscribe_push` は push の byte 列を Connect の stream に載せ、取りこぼし時に `wire::push::Event::Resync` を組み立てる。

## HTTP local API の応答が 2 通りの作られ方をしている

- 変換を持つもの。`src-tauri/src/adaptor/controller/api/protocol.rs`（200 行）に `StartExecutionResponse`・`MutationResponse`・`ValidateArtifactResponse`・`GetArtifactResponse` があり、後ろの 2 つは usecase の結果からの `From` を持つ。
- 変換を持たないもの。`GET /workflows` は `Json<Vec<WorkflowSummaryDto>>`、`GET /workflows/executions` は `Json<Vec<WorkflowExecutionSummaryDto>>`、`GET .../log` は `Json<Vec<WorkflowEventView>>`、`GET /workflows/diagnostics` は `Json<DiagnosticReport>` を返す。これらの型は usecase（`usecase/workflow/dto.rs`、`usecase/workflow/query_service.rs`、`usecase/workflow/diagnostic_dto.rs`）で `Serialize` と `#[serde(rename_all = ...)]` を持ち、JSON の形を usecase が決めている。CLI（`cli/diagnostics.rs`）はこの JSON を `DiagnosticReport` として deserialize する。
- `api/protocol.rs` には HTTP local API のリクエスト型（`StartExecutionRequest`・`ApproveNodeRequest`・`SubmitOutputRequest`・`RetryNodeRequest`・`ValidateArtifactRequest`）もある。hook の入口のリクエスト型（`ProviderLifecycleReceiveRequest`・`ProviderLifecycleSignalRequest`・`ProviderLifecycleUnavailableRequest`）と `WorkflowSubmitArtifactInput` は `adaptor/protocol/` にある。
- `ApiError` と `ApiErrorBody` の型は `src-tauri/src/adaptor/controller/api/error.rs`（25 行）にあり、その組み立てと HTTP status への対応付けは `src-tauri/src/adaptor/presenter/api_error.rs`（129 行）にある。

## Output Boundary の実装が gateway にある

- #1928 が作った最初の Output Boundary である `PerformanceOutput`（`src-tauri/src/usecase/telemetry.rs:120`）は、`src-tauri/src/adaptor/gateway/telemetry.rs:69` が実装している。`README.md` の部品の一覧は Output Boundary を adaptor/presenter が実装すると定めている。

# Scope / Non-goals

今回変更する対象。

- 状態の配信の仕組み（版、送り待ち、bookmark、overflow の判定、続きからの再開、次に送る要素の取り出し）を domain と usecase から出し、presenter と infrastructure へ置くこと。`domain/state_subscription/` の廃止を含む。
- 送る量の制御（`domain/terminal_surface/value_objects/output_flow_control.rs`）を domain から出すこと。
- 状態の配信のための usecase の Output Boundary の trait の新設と、adaptor/presenter による実装。
- 画面へ届ける口を Output Boundary へ寄せること。対象は `RepoPathsNotifier`、`TerminalSurfaceStateSink`、`TerminalSurfaceEventSink`、`TerminalSurfaceEventSource` のうち画面への配信に当たる部分、`AgentSessionChangeNotifier`、`RepositoryStateNotifier` である。
- usecase の 6 ファイルが `StateSubscriptionPublisher` を直接持って `invalidate` を呼ぶ形を、Output Boundary の呼び出しに置き換えること。
- 購読の開始・停止・再読み取り・watch の調整の手順を usecase の Usecase として残し、配信の仕組みの実体ではなく Output Boundary を通すようにすること。
- controller の側の stream の処理（`controller/api/state_subscription.rs` の変換、`controller/api/client_stream.rs`、`controller/api/client_service.rs` の購読と push の stream の組み立て）のうち、転送の形への変換を presenter へ移すこと。
- `adaptor/protocol/` の廃止。`connect.rs` を含む全ファイルを presenter へ移す。
- push のメッセージの組み立て（`adaptor/gateway/push.rs` の `BackendPush::emit`）を presenter へ移すこと。
- HTTP local API のリクエスト型を `adaptor/controller/api/protocol.rs` に集めること。`adaptor/protocol/provider_lifecycle.rs` の各 Request 型と `WorkflowSubmitArtifactInput` を含む。
- HTTP local API のレスポンスの型と、Output Data からの変換を presenter へ移すこと。`StartExecutionResponse`・`MutationResponse`・`ValidateArtifactResponse`・`GetArtifactResponse`・`ProviderLifecycleReceiveResponse` に加え、いま usecase の Output Data をそのまま JSON にしている 4 応答（`WorkflowSummaryDto`・`WorkflowExecutionSummaryDto`・`WorkflowEventView`・`DiagnosticReport`）に対応する転送の型を presenter に置き、usecase の Output Data から転送の都合（serde の属性）を外すこと。
- `adaptor/gateway/workflow/` の 5 ファイルが `adaptor::protocol::workflow` 経由で参照している `usecase::workflow::diagnostic_dto` の型を、usecase から直接参照する形にすること。
- 上記に伴い使われなくなるコードの削除。

今回変更しない対象。

- 購読の対象を足すこと。#1886・#1887・#1898 が扱う。
- stream ごとの bookmark。#1879 が扱う。
- 失敗の記録の表示。#1930 が、この変更で作る Output Boundary を使って扱う。
- 作業列の分解と `domain/work_queue.rs`・`domain/failure_records.rs`・各ドメインの `background_failure.rs` の廃止。#1930 が扱う。`usecase/work_queue.rs` が状態の配信へ通知する経路は、Output Boundary の呼び出しへ置き換えるだけにする。
- 失敗の形とステータスコードへの対応付け。#1929 で完了している。
- push の送信機構（`infrastructure/push.rs` の `PushSink`）と、controller の購読の入口（`client_service.rs` の `subscribe_push`）。転送の形への変換だけを presenter へ移す。
- `PerformanceOutput` の実装が gateway にあること。所見として別に報告する。
- 転送のメッセージ、JSON の形、Connect のステータスコード、HTTP status の変更。
- 画面（`src/`）と CLI の振る舞い。CLI のコードは、型の置き場所の変更に追従する範囲で変える。
- Tauri のシェルの `src-tauri/src/adaptor/gateway/desktop_client.rs` と `daemon_supervision.rs` の振る舞い。動き続けるように合わせるだけにする。
- 規約（`docs/architecture/`）の記述の変更。

# Requirements

- R-001: 状態の配信の仕組み（版、送り待ち、bookmark、overflow の判定、続きからの再開、次に送る要素の取り出し、送る量の制御）は domain と usecase に無く、presenter と infrastructure にある。送り待ちと送る量の制御は infrastructure にある。
- R-002: usecase が状態と結果を外へ出す口は Output Boundary の trait であり、adaptor/presenter が実装する。usecase は配信の仕組みの実体も転送の形も参照しない。Output Boundary の引数は Output Data である。
- R-003: 画面へ届ける口は domain の trait として定義されない。domain が定義する trait は Repository と、外部世界を必要とするドメインサービスの 2 種類だけである。
- R-004: Output Data を転送の形（Connect のメッセージ、HTTP local API のレスポンス、push のメッセージ）に変えるのは presenter だけである。controller と gateway と infrastructure は転送の形を組み立てない。
- R-005: `src-tauri/src/adaptor/protocol/` は無くなり、そこにあった部品は規約の部品の一覧のいずれかの部品として置かれる。
- R-006: HTTP local API のリクエスト型は `src-tauri/src/adaptor/controller/api/protocol.rs` にある。レスポンスの型と Output Data からの変換は presenter にある。usecase の Output Data は転送の都合（serde の属性）を持たない。
- R-007: 購読の開始・停止・再読み取り・watch の調整の手順は usecase の Usecase にある。読み取りは usecase の trait を通す。
- R-008: 画面と CLI が受け取る転送のメッセージ、JSON の形、Connect のステータスコード、HTTP status は、この変更の前と同じである。
- R-009: 購読を続きから再開できる条件、できないときに snapshot から配信し直す条件、bookmark を送る条件は、この変更の前と同じである。
- R-010: terminal の出力について、未処理量が上限を超えたときに provider の出力を止め、下限を下回ったときに再開すること、および client の処理の報告の単位は、この変更の前と同じである。
- R-011: push（file-change / git-status-changed / review-comments-changed / resync）を画面が受け取る内容と、取りこぼしたときの立て直しの結果は、この変更の前と同じである。
- R-012: この変更で使われなくなるコードと、この変更で触れたファイルの中で使われていないコードは無い。

# Assumptions

人間が明示的に受け入れた仮定は無い。
