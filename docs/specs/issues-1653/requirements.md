# Context

- Issue: https://github.com/siro33950/releash/issues/1653 （`[observability] terminal spawn 失敗理由が変換で破棄され、ロガー未初期化でパッケージ版にログが一切残らない`、label: bug）
- 契機となった実障害: 2026-08-19、PJT-2308 worktree で AgentSession 起動が連続失敗した。切り分けにイベントストアの直接照会（sqlite3）と OS レベルのプロセス列挙（lsof / pgrep）が必要だった。アプリ内のどこにも失敗理由が記録されていなかったため。
- 本 Spec は Issue が挙げる 2 つの原因の両方を対象にする。Issue 本文は原因2（logger 未初期化）について「別途議論して決める」としているが、2026-08-23 に人間が両方を Scope に含めると確定した。この確定が Issue 本文の記述に優先する。
- ログの出力先は 2026-08-23 に人間が「ローカルファイル（ローテーション付き、外部送信なし）」と確定した。
- 対象コード:
  - `src-tauri/src/usecase/agent_session/agent_session_launch.rs`
  - `src-tauri/src/adaptor/gateway/agent_session/provider_agent_terminal_gateway.rs`
  - `src-tauri/src/adaptor/gateway/workflow/node_session_boundary.rs`
  - `src-tauri/src/usecase/terminal_surface/error.rs`
  - `src-tauri/src/domain/terminal_surface/entities/terminal_surface_registry.rs`
  - `src-tauri/src/infrastructure/telemetry/`
- 既存の観測基盤: OpenTelemetry (OTLP) の trace / metrics / logs パイプラインが `src-tauri/src/infrastructure/telemetry/mod.rs` にある。logs signal は `SdkLoggerProvider` として構築されるが、利用者は `crash::report_error`（panic hook と frontend からのエラー報告）だけである。

# Outcome

対象者は、Releash の terminal / AgentSession 起動が失敗したときに原因を特定する開発者および利用者である。

現在は、起動失敗が起きてもアプリが残す記録に失敗理由が含まれない。per-worktree PTY cap 到達も、PTY の open / fork 失敗も、同じ `TerminalUnavailable` としてしか残らない。さらに `log::warn!` / `log::error!` の呼び出しは 145 箇所あるが logger が初期化されていないため、どの経路の警告・エラーも出力先を持たない。結果として、障害のたびにイベントストアの直接照会と OS レベル調査が必要になる。

変更後は、terminal spawn 失敗の具体的な理由が Releash 自身の記録から判別でき、`log` マクロ経由の警告・エラーがパッケージ版でもローカルのログファイルに残る。障害の一次切り分けが、外部ツールによる forensics なしに完了する。

# Current Behavior

## 原因1: spawn 失敗理由がエラー変換で破棄される

失敗理由は発生源では区別されているが、AgentSession 起動経路の 2 段階の変換で失われる。

理由が保持されている段階:

- `domain/terminal_surface/entities/terminal_surface_registry.rs:157-178` の `reserve_spawn` は、`TerminalSurfaceSpawnReservationError` として `OwnerOccupied(session_key)` / `WorktreeCapReached(worktree_path)` / `TotalCapReached` を区別して返す。cap の既定値は `domain/terminal_surface/value_objects/terminal_surface_lifecycle_config.rs:10-11` で `per_worktree_cap: 32` / `max_panes_total: 64`。
- `usecase/terminal_surface/error.rs:24-38` は cap 到達をメッセージ付きの `UsecaseError::CapReached("PTY cap reached for worktree {worktree_path}")` / `UsecaseError::CapReached("PTY total cap reached")` へ変換する。PTY の open / fork 失敗は `TerminalSurfaceGatewayError`（`domain/terminal_surface/gateway.rs:22-24`、メッセージ文字列を保持する）から `UsecaseError::Gateway(String)` になる。
- owner 衝突として呼び出し元へ到達するのは、`usecase/terminal_surface/spawn_usecase.rs:232-237,258-263` の `UsecaseError::Gateway("Terminal Surface owner identity collision")` である。`TerminalSurfaceSpawnReservationError::OwnerOccupied` は `spawn_usecase.rs:255-271` で呼び出し元へ返されず、`wait_for_spawn_resolution` の後に再試行へ戻るため、`error.rs:27-29` の `"Terminal Surface owner is already being created: {session_key}"` へは到達しない。
- 上記4分類のいずれにも当たらない失敗も、AgentSession が呼ぶ `usecase/terminal_surface/application.rs:385-404` の `get_or_spawn_process` から `UsecaseError` として返る。runtime lifecycle による mutation 拒否（`application.rs:194-196,212-223`）、checkpoint 読込失敗（`spawn_usecase.rs:277-283`）、output reader 開始失敗（`spawn_usecase.rs:70-74`）、既存の exited surface に対する drain 失敗（`spawn_usecase.rs:238-244`）がこれに当たり、いずれも発生源のメッセージを保持している。

理由が失われる段階:

- `adaptor/gateway/agent_session/provider_agent_terminal_gateway.rs:22-31` の `spawn` が `get_or_spawn_process` の結果を `.map_err(|_| ProviderAgentTerminalGatewayError::Unavailable)` で潰す。`ProviderAgentTerminalGatewayError` は `domain/agent_session/provider_terminal_gateway.rs:7-9` で `Unavailable` の単一 variant である。
- `usecase/agent_session/agent_session_launch.rs:599-613`（新規起動 `spawn_prepared`）は `self.terminal.spawn(...).is_err()` だけを見て `AgentSessionLaunchUsecaseError::TerminalUnavailable` を返す。
- `usecase/agent_session/agent_session_launch.rs:707-721`（`resume_history`）も同様に `.is_err()` だけを見て、失敗を Paused 結果へ落とす。

その結果として残る記録:

- workflow 経路: workflow の Session Node で terminal spawn を行うのは、`usecase/agent_session/agent_session_launch.rs:345-399` の `activate_workflow_node` から呼ばれる `spawn_prepared` である。`prepare_workflow_node`（同 301-343）は `terminal.spawn` を呼ばない。失敗は `adaptor/gateway/workflow/node_session_boundary.rs:141-155` が `WorkflowRuntimeError::AgentSession(format!("activate Workflow AgentSession '{node_session_id}': {error:?}"))` として組み立て、`adaptor/gateway/workflow/workflow_host.rs:1284-1317` が `settle_runtime_failure_for_node` へ渡し、同 2081-2119 が `format!("workflow runtime activation failed: {error}")` を `WorkflowEvent::NodeFailed { reason }`（`domain/workflow/value_objects/runtime_event.rs:105-115`）の reason にする。`WorkflowRuntimeError::AgentSession` の `Display` はメッセージをそのまま出力し（`usecase/workflow/runtime_error.rs:51`）、`AgentSessionLaunchUsecaseError` は `agent_session_launch.rs:70-79` の fieldless enum なので `{error:?}` は variant 名だけになる。実際に残る文字列は `workflow runtime activation failed: activate Workflow AgentSession '<agent_session_id>': TerminalUnavailable` であり、含まれる識別子は AgentSession の id で、失敗理由も NodeExecution id も含まれない。この reason が `adaptor/gateway/workflow/fact_log.rs:299-315` で `ProcessExitedFact.failure_reason` としてイベントストアへ永続化される。
- standalone 経路: `adaptor/controller/command/agent_session/provider_tui.rs:384-387` が `AppError::coded("AGENT_SESSION_TERMINAL_UNAVAILABLE", "AgentSession Terminal Surface is unavailable")` を frontend へ返すだけで、永続化される記録はない。

対比: terminal surface を直接扱う command 経路 `adaptor/controller/command/terminal_surface/commands.rs:122` は `UsecaseError::CapReached(_) => PTY_ERROR_CODE_CAP_REACHED` として cap 到達を区別している。AgentSession 起動経路だけが区別を落としている。

再現手順: 同一 worktree で AgentSession を `per_worktree_cap`（既定 32）まで起動した状態で、さらに 1 つ起動する。workflow の Session Node から起動した場合はイベントストアに `workflow runtime activation failed: activate Workflow AgentSession '<agent_session_id>': TerminalUnavailable` が残り、standalone の場合は code `AGENT_SESSION_TERMINAL_UNAVAILABLE` が frontend へ返る。いずれにも cap 到達である事実、どちらの cap か、対象 worktree path は残らない。2026-08-19 の実障害（per-worktree cap 到達）でも、イベント上は `TerminalUnavailable` としか残らなかった。

## 原因2: logger が初期化されておらず log マクロの出力先がない

- `src-tauri/Cargo.toml:56` に `log = "0.4"` があり、`src-tauri/src/` 配下に `log::warn!` / `log::error!` / `log::info!` / `log::debug!` / `log::trace!` の呼び出しが 145 箇所ある。
- logger 実装の登録は存在しない。`src-tauri/` 全体に対する調査で、`log::set_logger` / `log::set_boxed_logger` / `impl log::Log` / `log::set_max_level` / `LevelFilter` / `env_logger` / `tracing-subscriber` / `simple_logger` / `fern` のいずれも見つからない。`tauri-plugin-log` は `Cargo.toml` に無く、`src/lib.rs:622-634` の plugin 登録にも無い。
- `log` crate は logger 未登録時にすべてのマクロを no-op として扱う。したがって開発ビルドでもパッケージ版でも、これら 145 箇所の出力はどこにも残らない。macOS 統合ログにも残らない。
- CLI プロセスも同じ状態にある。`src-tauri/src/main.rs:13-18` は引数がある場合に Tauri builder を構築せず `releash_lib::cli::run()`（`src/cli/mod.rs:102`）へ分岐するため、GUI 側の起動処理を通らない。`src/cli/hook.rs:116,144` など CLI から通る経路にも `log::warn!` の呼び出しがある。
- OTLP の logs signal は存在するが `log` facade とは接続されていない。`infrastructure/telemetry/mod.rs:104-112` が構築した `SdkLoggerProvider` は `crash::init_crash_reporting` へ渡され、`infrastructure/telemetry/crash.rs:40-70` の `report_error` だけが使う。`report_error` は panic hook（`crash.rs:72` 以降）と frontend からのエラー報告（`adaptor/controller/command/telemetry/commands.rs:13`）の 2 経路からしか呼ばれず、かつ crash reporting 無効時と OTLP 未設定時は早期 return する。`log` マクロを OTLP logs へ橋渡しする appender は導入されていない。
- 具体的な影響例: `usecase/agent_session/agent_session_launch.rs:794-796` の `rollback_failed_new_launch_preserving_cause` は、起動失敗のロールバックに失敗した事実を `log::warn!` で記録しているが、この警告は実際にはどこにも出ていない。同様に `adaptor/gateway/terminal_surface/runtime_gateway_impl.rs:274,318,322` の terminal 出力収集・終了検出・永続化の失敗も残らない。

再現手順: パッケージ版（`.app`）として配布した Releash を起動し、`log::warn!` を通る経路（例: AgentSession 起動失敗後のロールバック失敗）を発生させる。アプリのログファイルは存在せず、`log show --predicate 'process == "releash"'` にも該当レコードが出ない。

# Scope / Non-goals

## Scope

- AgentSession 起動経路（新規起動および history resume）における terminal spawn 失敗理由の保全。理由の区別（per-worktree cap 到達 / 総数 cap 到達 / owner 衝突 / PTY の open・fork 失敗 / これら以外の spawn 失敗）が失敗後に参照できる記録へ残るようにする。
- workflow の Session Node 起動失敗として残る失敗理由への、上記区別の反映。
- workflow 外（standalone 起動、history resume）の AgentSession 起動失敗に対する、失敗理由の記録の追加。
- logger の初期化の導入。`log` crate 経由の警告・エラーがローカルのログファイルへ書き出されるようにする。対象は GUI プロセスと CLI プロセス（`releash workflow` / `releash review` / `releash hook`）の両方とする。

## Non-goals

- terminal spawn 失敗そのものを減らすこと。cap の既定値の変更、cap 到達時の待機・再試行・キューイングの導入は行わない。
- 既存の OTLP telemetry（trace / metrics）および crash reporting の送信内容・送信条件の変更。
- ログを閲覧・検索する UI をアプリ内に追加すること。
- 既存 `log` マクロ呼び出し箇所の網羅的な見直し（レベルの是正、文言の統一、呼び出しの追加・削除）。本 Spec は既存の呼び出しに出力先を与えることに限る。
- AgentSession 以外のドメイン（review、repository、workflow 定義評価など）の観測性の改善。
- 失敗理由を利用者向けに UI で案内すること（cap 到達時の誘導表示など）。

# Requirements

- R-001: AgentSession の terminal spawn が失敗したとき、その失敗が「per-worktree PTY cap 到達」「PTY 総数 cap 到達」「owner 衝突」「PTY の open / fork 失敗」「これら以外の spawn 失敗」のいずれであるかを、失敗後に参照できる Releash の記録から区別できる。
- R-002: 失敗が per-worktree PTY cap 到達である場合、R-001 の記録に対象の worktree path が含まれる。
- R-003: 失敗が PTY の open / fork 失敗、または R-001 の「これら以外の spawn 失敗」である場合、R-001 の記録に発生源が返したエラー内容が含まれる。
- R-004: workflow の Session Node が terminal spawn 失敗によって失敗したとき、その Node の失敗理由として残る記録に R-001 から R-003 の内容が含まれる。
- R-005: workflow 外の AgentSession 起動（standalone 起動および history resume）が terminal spawn 失敗で終わったときも、R-001 から R-003 の内容が失敗後に参照できる記録として残る。
- R-006: パッケージ版として配布した Releash の GUI プロセスおよび CLI プロセス（`releash workflow` / `releash review` / `releash hook`）が `log` crate 経由で記録した警告およびエラーがローカルのログファイルへ書き出され、プロセス終了後にそのファイルから参照できる。
- R-007: R-006 のログファイルは無制限に増大せず、サイズまたは世代の上限を持つ。
- R-008: R-006 のログファイルの内容は Releash から外部へ送信されない。

# Assumptions / Open Questions

## Assumptions

- 本 Spec の Scope に Issue #1653 の原因1 と原因2 の両方を含めることは、2026-08-23 に人間が確定した。Issue 本文の「logger 初期化の導入を別途議論して決める」という記述より、この確定が優先する。
- `log` マクロの出力先をローカルファイル（ローテーション付き、外部送信なし）とすることは、2026-08-23 に人間が確定した。
- terminal spawn 失敗の記録対象を Issue 本文が挙げる4分類に限らず、呼び出し元へ到達する spawn 失敗すべてとすることは、2026-08-23 に人間が確定した。
- logger の対象に GUI プロセスと CLI プロセスの両方を含めることは、2026-08-23 に人間が確定した。

## Open Questions

なし。
