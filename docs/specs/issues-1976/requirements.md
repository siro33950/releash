# Context

- 入力文書: https://github.com/siro33950/releash/issues/1976（マイルストーン #99 の [13]）
- 依存: #1929（失敗を業務の失敗と技術的な失敗に分ける。完了済み）。技術的な失敗は `TechnicalFailure { nature, message }`（`src-tauri/src/domain/failure.rs`）で表し、性質は `Transient`・`TimedOut`・`Cancelled`・`Other` の 4 つ。presenter は性質をステータスコード（順に `UNAVAILABLE`・`DEADLINE_EXCEEDED`・`CANCELED`・`INTERNAL`）へ対応付ける（`src-tauri/src/adaptor/presenter/connect.rs`）
- 規約: `docs/architecture/DOMAIN.md`「失敗」（trait の失敗は業務の結果と、中身を見ない技術的な失敗の変種 1 つで表す）、`docs/architecture/USECASE.md`「失敗」（技術的な失敗は中身で分類し直さずにそのまま出す。失敗を文字列へ変換して落とさない）、`docs/architecture/GATEWAY.md`「失敗の変換」、`docs/architecture/README.md`（同じ操作の実装は 1 つに集約する）
- 画面のつなぎ直しの対象のステータスコードは、proto の `reconnect_status_code`（`UNAVAILABLE`・`ABORTED`・`RESOURCE_EXHAUSTED`）として #1951 で決めたもの
- CLI は HTTP local API の status を `src-tauri/src/cli/api_client.rs:188-199` の対応で CLI のエラーにする
- 利用者のいない段階であり、ステータスコードの互換性を理由に変更を止めない

# Outcome

対象者: Releash の画面と CLI の利用者、および失敗の性質でやり直しを判断する daemon の処理。

現在の問題: gateway と usecase が、下の層から受け取った失敗を分類を見ずに「使えない」系の分類へ付け替えている。時間切れ・取り消し・一時的な失敗・入力の誤りが、画面と CLI には同じ「使えない」として届く。presenter が性質に従ってステータスコードを決めても効かず、やり直すかどうかの判断も付け替えた後の分類で決まる。terminal の失敗は文字列として運ばれ、起動時の store の失敗は、次の起動でも直らない失敗にも「次回の起動で直る」と出る。

変更後の状態: 下の層の失敗は、業務の失敗なら業務の結果として、技術的な失敗なら元の性質とメッセージを持ったまま、presenter と判断の処理まで届く。画面と CLI に返るステータスコード、やり直しの判断、起動の失敗の「次回の起動で直るか」が、元の失敗で決まる。

# Current Behavior

開始時点（main `506f9a98`）で確認した挙動。

- 下の層の失敗を分類を見ずに付け替える `map_err(|_| …)` が 67 か所ある（`src-tauri/src/adaptor/gateway/local_event_store/store.rs` 16、`src-tauri/src/usecase/agent_session/agent_session_lifecycle.rs` 14、`src-tauri/src/adaptor/gateway/agent_session/agent_session_history_gateway.rs` 7、`src-tauri/src/infrastructure/provider_lifecycle/health_marker.rs` 6、`src-tauri/src/usecase/agent_session/agent_session_launch.rs` 4、`src-tauri/src/usecase/agent_session/provider_availability.rs` 3、`src-tauri/src/infrastructure/provider_lifecycle/launch_files.rs` 3、`src-tauri/src/adaptor/gateway/app_config/repository_impl.rs` 3、`src-tauri/src/adaptor/gateway/agent_session/provider_agent_terminal_gateway.rs` 3、`provider_executable_config_repository.rs` 2、`provider_agent_launch_gateway.rs` 2、`hook_health_failure_query_impl.rs` 1）
- AgentSession の launch の cleanup や terminal の操作が失敗すると、元の失敗が一時的・時間切れ・取り消し・入力の誤りのどれであっても、`AgentSessionLifecycleUsecaseError::LaunchUnavailable`・`TerminalUnavailable` 等になり、画面には一律に `FAILED_PRECONDITION` が返る（`connect.rs` の `AgentSessionLifecycleUsecaseError`・`AgentSessionLaunchUsecaseError` の対応）
- Provider の設定の読み書き・検索パスの更新・履歴の読み取り・hook の健全性の読み取りの失敗は、元の失敗に関係なく一律に `UNAVAILABLE` になる。`spawn_blocking` の panic・取り消しも `UNAVAILABLE` になる
- terminal の失敗は、`usecase/terminal_surface/error.rs` の `Gateway(String)`・`PtySpawn { error: String }`・`OtherSpawnFailure { error: String }`、domain の `TerminalSurfaceGatewayError`（メッセージ文字列と入力の原因だけを持つ struct）、`ProviderAgentTerminalSpawnError`（文字列を持つ変種）として文字列で運ばれる。対象が無い失敗と外部の失敗（checkpoint のファイル操作、PTY の resize・kill 等）が区別されず、画面には terminal の操作では一律に `INTERNAL`、AgentSession の起動の経路では一律に `FAILED_PRECONDITION` が返る
- 起動時に store を開く処理の失敗のうち 16 か所は、元の失敗を見ずに `StorageUnavailable` になる。reader の接続の失敗は、SQLite の版が古い場合も `UnsupportedRuntime` ではなく `StorageUnavailable` になる。`StorageUnavailable` の起動の失敗は常に `retry_on_next_launch` が true になり、権限が無い・ディスクが一杯といった次の起動でも直らない失敗でも「次回の起動で直る」と画面に出る（`src-tauri/src/usecase/application_startup.rs:54-59`）

# Scope / Non-goals

変更するもの:

- 上の 67 か所の付け替えと、それに伴う付け替え先の error の型
- terminal の失敗を文字列で持つ型: usecase の `Gateway(String)`・`PtySpawn`・`OtherSpawnFailure`、domain の `TerminalSurfaceGatewayError`・`ProviderAgentTerminalSpawnError`、それらを作る箇所と写す箇所
- 起動時の store を開く失敗の分類と、起動の失敗の `retry_on_next_launch`
- 付け替えた後の分類から固定のステータスコードを出している presenter の対応

変更しないもの:

- domain の各エラーの enum が技術的な失敗の変種を複数持つこと、usecase が独自のエラーの型（依存先ごとの変種を持つ enum を含む）を持つこと。保存側の `AgentSessionRepositoryError::Unavailable`・`ProviderLifecycleRepositoryError::StorageUnavailable`・`ProviderHookHealthRepositoryError::StorageUnavailable`・`AgentSessionQueryError::Unavailable` の変種と、`adaptor/gateway/shared/storage_failure.rs` の対応する変換を含む。マイルストーン #97 で扱う
- 転送のステータスコードへの対応付けの場所。HTTP local API で workflow の技術的な失敗が性質に関係なく 500 になること（`adaptor/presenter/api_error.rs`）を含む。#1897 で扱う
- 画面のつなぎ直しの対象のステータスコード（proto の `reconnect_status_code`）。#1951 で決めたもの
- CLI が HTTP status を CLI のエラーにする対応（`src-tauri/src/cli/api_client.rs:188-199`）

# Requirements

- R-001: 変更対象の箇所で下の層から受け取った失敗は、業務の失敗なら業務の結果として、技術的な失敗なら元の性質（`Transient`・`TimedOut`・`Cancelled`・`Other`）とメッセージを持ったまま、presenter とやり直しの判断まで届く。分類を見ずに別の分類へ付け替えず、文字列へ変換して落とさない
- R-002: 画面に返るステータスコードは、元の失敗で決まる。技術的な失敗は性質に従ったステータスコードになり、付け替えた後の分類からは決まらない
- R-003: AgentSession の Provider の操作と Terminal の操作が失敗したときに画面に出る文言（「Provider の操作」「Terminal の操作」を完了できなかった旨）は今のままである
- R-004: terminal の失敗は、対象が無い等の業務の結果と、外部の失敗（checkpoint のファイル操作、PTY の起動・resize・kill 等）による技術的な失敗とが区別され、技術的な失敗は元の性質を持つ。terminal の起動の経路でも同じである
- R-005: 起動時に store を開く処理の失敗は、既存の分類（SQLite の inaccessible・busy、SQLite の版が古い、writer の lock の取り合い）で分けられるものはその分類になる。それ以外の技術的な失敗は `StorageUnavailable` として、元の性質とメッセージを持つ
- R-006: 起動の失敗の `retry_on_next_launch` は、`StorageUnavailable` のとき元の性質で決まり、`Transient` と `TimedOut` なら true、それ以外なら false である。`StoreInUse` と `SchemaEvolutionFailed` は true である
- R-007: 起動時の SQLite の失敗と io::Error の性質は、実行中の store の書き込み・読み取りと同じ判定で決まる。SQLite の busy は `Transient`、inaccessible は `Other`。io::Error は `Interrupted`・`WouldBlock`・接続の切断が `Transient`、`TimedOut` が `TimedOut`、それ以外が `Other`
- R-008: やり直すかどうかの判断（一時的な失敗ならやり直す）は、付け替えた後の分類ではなく元の失敗の性質で決まる
- R-009: この変更でステータスコードが変わっても、画面がつなぎ直す・つなぎ直さないは変更前と同じである。CLI は、受け取った HTTP status を今の対応（`src-tauri/src/cli/api_client.rs:188-199`。404 は NotFound、400・409・422 は InvalidInput、それ以外は Other）に従って CLI のエラーにする

# Assumptions

なし
