# Context

- 要求の正本: https://github.com/siro33950/releash/issues/1852（[01] サーバの受け入れテストから Tauri への依存を抜く）
- 補助資料: https://github.com/siro33950/releash/issues/1853（[03] サーバと Tauri シェルを別のクレートに分ける）、https://github.com/siro33950/releash/issues/1902（[05] CLI を薄いクライアント crate にし HTTP を削除する）
- マイルストーン: 03. サーバを画面から独立させ、CLI を整える（#100）
- この変更は [03]（#1853）でサーバと Tauri シェルを別のクレートに分けるための前提である。
- 「Tauri を参照する」は Tauri クレート（`tauri` と `tauri::*`）の参照を指す。Origin の文字列 `tauri://localhost`、コメント、テスト名は含まない。
- [03] で Tauri シェルのクレートへ移すもの: `desktop.rs`、`adaptor/controller/command/`、`infrastructure/platform/` の tray・menu・window など、`tests/desktop_*.rs`、シェル用に分けたハーネス、および #1853 本文が挙げる `desktop` で gate されたモジュール。
- 規約: `docs/architecture/TEST.md`、`AGENTS.md`。

# Outcome

- 対象者: Releash の開発者（サーバのテストをビルド・実行する人と CI）。
- 現在の問題: サーバ（daemon と CLI）の受け入れテストとサーバ側の lib テストの一部が、Tauri のアプリ（`tauri::App` / `AppHandle` / `tauri::test` の mock runtime）を依存の入れ物として組み立てて動いている。そのため、サーバのテストは `desktop` feature と `tauri`（`test` feature）なしではビルド・実行できず、[03] でサーバのクレートを Tauri から切り離せない。
- 変更後の状態: サーバのテストは Tauri のアプリを作らずに、サーバの入口（usecase、Connect の client API）を通して実行でき、`desktop` feature を無効にした構成でもビルド・実行できる。Tauri を参照するのは [03] でシェルのクレートへ移すものだけになる。

# Current Behavior

main `69214d09` で確認した。パスは `src-tauri/` 起点。

- `tauri` が無条件の dev-dependency（`test` feature 付き）になっている（`Cargo.toml:104`、`:112`）。
- 受け入れテスト用のハーネス `workflow_control_plane_acceptance`、`agent_session_tui_acceptance`、`client_api_acceptance`、`provider_lifecycle_acceptance`、`workflow_diagnostics_acceptance`、`workflow_delegate_acceptance` と `desktop_test_support` は、`cfg(all(debug_assertions, feature = "desktop"))` の下でだけ存在する（`src/lib.rs:2-19`、`:88-89`）。`terminal_subscription_acceptance` は `cfg(debug_assertions)` だけで公開されている（`src/lib.rs:12-13`）が、`src/terminal_subscription_acceptance.rs:71,76,81` の getter は `desktop` 限定である。
- ハーネスのうち `src/workflow_control_plane_acceptance.rs`、`src/agent_session_tui_acceptance.rs`、`src/client_api_acceptance.rs`、`src/desktop_test_support.rs` が Tauri を参照する。`tauri::App` を `manage` / `state` / `try_state` で依存の入れ物として使い（例: `src/workflow_control_plane_acceptance.rs:463-466`、`:519`、`:835-848`、`src/desktop_test_support.rs:6-78`）、`tauri::async_runtime::spawn` で task を起動している（`src/workflow_control_plane_acceptance.rs:583`、`src/agent_session_tui_acceptance.rs:304`）。
- `src/client_api_acceptance.rs` には、サーバ用（`ClientApiAcceptanceHost`、`ClientRecoveryAcceptanceHost`、`connect_client`、`request_client`、`read_state` など）とシェル用（`desktop_connection_app`、`desktop_supervision_status`、`stop_desktop_daemon`、`initialize_desktop_settings`、`desktop_window_preferences`、`desktop_client_endpoint`、`apply_desktop_update`、`spawn_desktop_successor` など。`tests/desktop_*.rs` と `tests/daemon_termination.rs` が使う）が混在している。
- サーバ用ハーネスの Connect 呼び出しは、シェル側の `adaptor::gateway::desktop_client`（`desktop` 限定）の `client` / `call` を使っている（`src/client_api_acceptance.rs` の `connect_client`・`request_client`）。
- サーバの中に、ハーネスのためだけに `desktop` 有効時に公開しているものがある（`cfg(any(test, all(debug_assertions, feature = "desktop")))` 等）。domain（`src/domain/workflow/value_objects/state.rs:1-42`、`src/domain/workflow/mod.rs:50-53`、`src/domain/workflow/value_objects/mod.rs:67`、`src/domain/workflow/entities/workflow_execution/mod.rs:3519`）、usecase（`src/usecase/workflow/runtime_snapshot.rs`、`src/usecase/workflow/mod.rs:471`、`src/usecase/agent_session/agent_session_launch.rs:1168`）、gateway（`src/adaptor/gateway/workflow/workflow_host.rs:1023`、`workflow/mod.rs:45`、`local_event_store/store.rs:416`、`local_event_store/writer.rs:199`、`provider_lifecycle/launch_spec.rs:166,171`、`agent_session/provider_availability_gateway.rs:40,70`）、infrastructure（`src/infrastructure/local_api/client_token.rs:20`、`local_api/server.rs:99,108`）、controller / presenter（`src/adaptor/controller/agent_session_wiring.rs:49-51,397-399`、`src/adaptor/presenter/client/mod.rs:26-62`、`presenter/client/errors.rs:67`、`presenter/client/json.rs:2,19,40,134,173`、`presenter/state_subscription.rs:222`）、生成コード（`build.rs:22`、`:50` が `CommandRequest` / `CommandResult` に同じ cfg を埋め込む）。
- `src/adaptor/gateway/application_lifecycle.rs:23-51` の `TauriApplicationQuitIntentPort` は `tauri::AppHandle` を持つ。参照は `src/desktop_test_support.rs:76` だけで、本番の daemon は `DaemonProcessActionPort` を使う（`src/adaptor/controller/daemon.rs:510`）。
- Tauri の `tauri::test` を使うテスト: `tests/client_api/mod.rs`、`tests/workflow_control_plane_acceptance_test.rs`、`tests/agent_session_tui_acceptance.rs`、`tests/state_subscription/flow_test.rs:26`、`src/adaptor/controller/client/workflow/mod.rs:208-`（`cfg(all(test, feature = "desktop"))`）、`src/adaptor/controller/api/client_test.rs:1115-1123`（`api/client.rs:145` で `cfg(all(test, feature = "desktop"))`）、`src/adaptor/controller/client/app_config/shared_test.rs:5-15`、`src/adaptor/controller/client/workspace_tree_shared_test.rs:2-12`。`cfg(all(test, feature = "desktop"))` の lib テストは `desktop` 無効の構成では走らない。
- `tests/client_api/mod.rs:73-117` は、サーバ側の断定（401/403、Connect 経由の相関と結果）と、Tauri の invoke が拒否される断定（`:101-113`）を 1 つのテストに含む。`:146-173` は Tauri の invoke が `"Command get_terminal_stream_endpoint not found"` で拒否されることを断定する。invoke の宛先は、空の command router（`src/client_api_acceptance.rs:155-156`、`:173`）を持つ mock アプリである。
- CI の `desktop` 無効の構成（`.github/workflows/ci.yml:308-316`）は lib テスト、`state_subscription_scenarios`、`daemon_smoke` だけを実行している。受け入れテストは `desktop` 無効ではビルドできない。

# Scope / Non-goals

変更するもの

- サーバ用の受け入れテストのハーネスと、それを使う `tests/` のサーバのテスト。
- Tauri のアプリを使っているサーバ側の lib テスト。
- `src/client_api_acceptance.rs` のサーバ用とシェル用の分離。
- Tauri の invoke の拒否を断定するテストの、シェルのテストへの切り出し。
- サーバ内の、ハーネスのためだけの `desktop` 限定の公開と、`build.rs` が生成コードに埋め込む同じ cfg。
- `TauriApplicationQuitIntentPort` の削除。
- 本番から読まれていない `ClientConnectionUsecase` とその登録の削除。

変更しないもの

- ハーネスの中で HTTP `/v1` を呼んでいる部分（`src/workflow_control_plane_acceptance.rs` の `/v1` 呼び出し）。[05]（#1902）が HTTP の削除と一緒に Connect へ移す。
- `tauri` の dev-dependency からの削除、`desktop` feature の削除、クレートの分割、CI ジョブの置き換え。[03]（#1853）で行う。
- シェル用のハーネス（`desktop_test_support` と `client_api_acceptance` のシェル用の部分）とシェルのテスト（`tests/desktop_*.rs`、`tests/daemon_termination.rs`、`adaptor/controller/command/` のテスト）を、サーバの入口を通す形に直すこと。[03] で行う。
- `adaptor/gateway/desktop_client.rs` と、それを使うシェル側のコード。
- シェルとサーバが共有するクライアント側コードの持ち方。[03] で決める。ただし、本番で登録されるだけで読まれていない `ClientConnectionUsecase`（`usecase/client_connection.rs`）とその登録は削除する。共有の型（`ClientConnectionDto`・`ClientConnectionError`・`ClientConnectionQueryService`）は変えない。
- 本番（release ビルド）の振る舞い。

# Requirements

- R-001: サーバの受け入れテスト（workflow control plane、agent session TUI、client API、provider lifecycle、workflow diagnostics、workflow delegate、state subscription）とサーバ側の lib テストは、Tauri のアプリを作らずに、サーバの入口（usecase、Connect の client API）を通して実行される。
- R-002: サーバの受け入れテスト、そのハーネス、サーバ側の lib テストは、`desktop` feature を無効にした構成でもビルド・実行できる。
- R-003: Tauri を参照するファイルが、[03]（#1853）で Tauri シェルのクレートへ移すものだけになる。
- R-004: 既存の受け入れテストと lib テストは、今と同じ振る舞いを同じ断定のまま検証し、成功する。
- R-005: Tauri の invoke が拒否されることの断定は、シェルのテストとして今と同じ断定のまま存続する。

# Assumptions

なし
