# Context

- 入力: https://github.com/siro33950/releash/issues/2018
- 規約の正本: `docs/architecture/TEST.md`（「種類」「書かないテスト」「配置と実行」）。ISSUE と TEST.md の記述が食い違う場合は TEST.md に従う。
- TEST.md の「配置と実行」は、手動で実行するテストをコミットしないと定める（main の `c7826ab6`）。`#[ignore]` で通常は実行されず、手動で起動する前提のテストもこれに当たる。
- テストの種類は、関わる仕組みと操作で決める。
  - 単体: 単一の仕組みで完結する。
  - 統合: 2つ以上の仕組み（サーバ/DB、フロント/サーバ、サーバ/OS）の関連。gateway が実 SQLite・実 git を使うテストは統合。
  - 振る舞い: 本物のサーバ・DB・OS を含む構成に対し、ユーザと同様の操作をする。偽サーバ相手の Playwright は振る舞いでも統合でもなく、フロントの単体。
- `AGENTS.md`「構成で押さえる点」: テスト用区画は `test-support` feature でのみ有効になる。
- `AGENTS.md`「ビルド・テスト・Lint」「リリース」は、PR 層と nightly 層のコマンドと、nightly のリリース関門を記載している。
- 開発開始時点の必須チェックの名前は `frontend`・`quality`・`integration`・`rust`。
- GitHub Actions のキャッシュ上限は1リポジトリ 10GB。Playwright 公式はブラウザのキャッシュを推奨していない。Biome 公式は CI で `biome ci` を推奨している。

# Outcome

対象者: Releash の開発者（人と agent）。

現在の問題: TEST.md でテストの種類・書かないテスト・置き場所・実行・CI を定めたが、既存のテスト、実行コマンド、CI はこの規約に合っていない。書かないテストに当たるテストと手動で実行するテストが残り、置き場所から種類が決まらず、単体と統合が同じコマンド・同じ CI ジョブで実行されている。規約から外れたテストが新たに入っても CI で検出されない。

変更後の状態: すべてのプロダクトのテストが TEST.md の「書かないテスト」に当たらず、手動で実行するテストはリポジトリに無く、種類に対応する置き場所にあり、種類ごとのコマンドと CI ジョブで実行される。置き場所の違反と、単体テストが外部の資源を使う違反は CI が検出して落とす。

# Current Behavior

開発開始時点（`47e44084`）で確認した状態。

書かないテストに当たるテスト（ISSUE 記載の例。全件は未調査）:
- 削除済み機能が存在しないことを確かめるテスト: `src-tauri/releash-desktop/src/adaptor/controller/command/client_test.rs:15`、`src-tauri/releash-desktop/tests/desktop_command_rejection.rs:29`、`src-tauri/releash-desktop/src/adaptor/controller/command/mod.rs:160`、`src-tauri/src/adaptor/controller/api/mod.rs:1070`、`src/components/panels/SettingsModal.test.tsx:765`
- 定数の値を確かめるテスト: `src-tauri/src/domain/agent_session/provider_session_title_cadence.rs:17-21`、`src-tauri/src/adaptor/gateway/workflow/lua/mod_test.rs:678`
- パフォーマンステスト: `tests/terminal-performance.spec.ts`、`tests/tauri-performance/`、`src-tauri/tests/agent_session_tui_acceptance.rs:1785`、`src-tauri/tests/client_api/mod.rs:154`（どちらも `#[ignore]` の計測）、`package.json` の `test:performance*` と `test:client-streams:macos`、`playwright.performance.config.ts`、`wdio.performance.conf.ts`、`wdio.client-streams.conf.ts`、`src/test/performance/`、計測ハーネス `tests/helpers/performance-daemon.mjs` とその自己テスト `tests/helpers/performance-daemon.test.mjs`

置き場所:
- `src-tauri/src/` と `src-tauri/releash-desktop/src/` に `#[test]` を持つファイルのうち、`*_test.rs` 以外のファイルが169ある。
- 単体テスト側のファイル（`*_test.rs`・`test_helpers*.rs`・`test_support/`）のうち、実 SQLite・実 git・`tempfile`・外部プロセス・TCP を使うものがおよそ89ある（統合に当たる）。
- `src-tauri/tests/state_subscription_scenarios.rs:1` は `include!("../src/lib.rs")` でライブラリ全体を取り込む単体テスト。`tests/state_subscription_reads/reads_test.rs` は `src-tauri/src/test_support/state_subscription.rs:1` から `#[path]` でライブラリ側に取り込まれている。
- ルートの `tests/*.spec.ts`（client-streams・settings・statusbar・terminal・workspace-manager）は偽の Connect サーバ（`tests/helpers/tauri-mock.ts`）相手の Playwright で、中身はフロント単体。
- `tests/desktop-bundle.mjs`（使い捨ての macOS アカウント専用）と `tests/desktop-login.mjs`（ログアウト/ログインを挟む段階実行）は配布 .app を手動で起動して操作するテストで、CI では実行されていない。`package.json` の `build:desktop:acceptance`（`--features performance-wdio`）・`test:desktop:bundle`・`test:desktop:login` がこれらを使う。
- 手動で起動する前提の `#[ignore]` テスト: `src-tauri/tests/agent_session_tui_acceptance.rs:1894,1958`（インストール済みの Claude Code・Codex CLI を使う gate）、`src-tauri/tests/provider_lifecycle_characterization_test.rs:436,501,552,608,696,761,864`（同 characterization gate）。
- `performance` feature（`src-tauri/Cargo.toml:87`、`src-tauri/releash-desktop/Cargo.toml:42`）は、データの置き場所（`RELEASH_PERFORMANCE_DATA_DIR`、`src-tauri/src/infrastructure/platform/app_data_dir.rs:6`、`path_aliases.rs:28`、`cli/common.rs:479`）、provider fixture（`RELEASH_PERFORMANCE_PROVIDER_FIXTURE_EXECUTABLE`、`src-tauri/src/adaptor/controller/daemon.rs:692-700`）、CLI 設置の probe（`src-tauri/releash-desktop/src/infrastructure/platform/cli_install.rs:35,39,66,308`、`desktop_client_acceptance.rs:125`）を切り替える。この切り替えを外から使うのは計測と `tests/desktop-bundle.mjs` 用のビルド（`tauri.conf.bundle.performance.json`、`package.json` の `build:performance`、`scripts/build-desktop-backend.mjs --performance`）だけで、ほかは feature 自体を確かめるテスト（`app_data_dir_test.rs:9,21`、`cli/common.rs:479`、`src-tauri/releash-desktop/tests/desktop_cli_install.rs`）だけである。フロントの `src/main.tsx:21` は `performance` モードの分岐を持つ。
- 計測にしか使われない実行時の仕組み:
  - 計測サンプルの収集: proto の `start_terminal_input_performance_collection`・`start_terminal_launch_performance_collection`・`take_terminal_input_performance_samples`・`take_terminal_launch_performance_samples`（`proto/client.proto:143-147,312-315`）と、サーバのサンプル保持（`src-tauri/src/infrastructure/telemetry/metrics/mod.rs` の `TERMINAL_INPUT_SAMPLES`・`TERMINAL_LAUNCH_SAMPLES` と入力 trace の記録）。呼ぶのは `tests/tauri-performance/` だけ。
  - A/B 切り替え: `src-tauri/src/infrastructure/performance_switches.rs` の `TerminalPerformanceSwitches`（`RELEASH_PERF_DISABLE_*`、`RELEASH_PERF_REAL_APP`）。本番では常に false。値は proto で画面へ配信され（`adaptor/presenter/terminal.rs`、`presenter/client/conversions.rs`、`state_subscription_wire.rs`）、サーバ（`terminal_surface_runtime.rs:75`、`terminal_event_hub.rs:30`）と画面（`src/hooks/useTerminal.ts`、`src/lib/terminalPerformanceSwitches.ts`）の分岐に使われる。環境変数を設定するのは計測と `tests/helpers/desktop-bundle.mjs` だけ。
  - 画面側の計測: `src/lib/terminalPerformanceProbe.ts`。`window.__RELEASH_TERMINAL_PERFORMANCE__` を入れるのは計測用の `src/test/performance/` だけ。描画側の起動段階（`first_xterm_parsed`・`first_paint`）は、この計測が有効なときだけ `record_terminal_launch_renderer_phase`（`proto/client.proto:118,287`）で送られ、サーバが OTLP に記録する（`src/hooks/useTerminal.ts:329-371`、`src-tauri/src/usecase/telemetry.rs:29-38`）。本番ではこの計測が有効にならないので、一度も送られていない。
  - サーバ側の起動段階の時間（`releash.terminal.launch.duration_ms`、`metrics/mod.rs:686-710`）は、利用者向けの設定「Send anonymous performance metrics」が有効なときに OTLP へ送られる。
- `performance-wdio` feature と `tauri-plugin-wdio`・`tauri-plugin-wdio-webdriver`・`@wdio/*` 依存は、計測と `tests/desktop-bundle.mjs` だけが使う（`src-tauri/releash-desktop/src/desktop.rs:41`、`src/main.tsx:22`、`src-tauri/releash-desktop/tauri.conf.performance.json`）。
- `tests/integration/` と `tests/behavior/` が無い。
- 統合テストがクレート内部に触れる手段として、`src-tauri/src/lib.rs:2-18` の `#[cfg(debug_assertions)] pub mod *_acceptance` と `acceptance_test_support`、`src-tauri/src/desktop_api.rs:60` の `#[cfg(feature = "test-support")] pub mod test_support` が併存している。

実行と CI:
- `package.json` の `test:integration` は偽サーバ相手の Playwright を実行する。`test:behavior` が無い。`lint` は `biome check .`。
- `.github/workflows/ci.yml` の Rust テストは `rust-test-desktop`（`cargo test -p releash-desktop`）と `rust-test-backend`（`cargo test`、`--test state_subscription_scenarios scenarios_tests::`、`--test daemon_smoke`）にパッケージ単位で分かれ、どちらも単体と統合を同じジョブで実行する。集約ジョブ `rust` の `needs` は `rust-lint`・`rust-test-desktop`・`rust-test-backend`。
- rust-cache の `key` はジョブごとに別（ci.yml に `rust-lint`・`rust-test-desktop`・`rust-test-backend`、nightly.yml に `nightly-performance`・`nightly-coverage`・`macos-universal`）。
- `.github/workflows/ci.yml:60-68` は Playwright のブラウザをキャッシュしている。
- `.github/workflows/nightly.yml` の `performance` ジョブは計測をしておらず、`cargo test --features performance --lib`、`pnpm test:performance:daemon`、`desktop_cli_install` の統合テストを実行する。`release` ジョブの `needs` に入っている。
- 置き場所の検査と、単体テストで外部の資源を使わない検査は存在しない。
- `AGENTS.md` の「ビルド・テスト・Lint」「リリース」に書かれたコマンドと関門が上記に依存している。

# Scope / Non-goals

Scope:
- `src-tauri/`・`src/`・ルートの `tests/` にある、プロダクト（サーバ・シェル・フロント）のすべてのテストの削除・書き換え・移設。
- 統合テストがクレート内部に触れるための公開入口の整理。
- `package.json` の実行コマンド、Playwright の設定、計測用と手動実行テスト用の設定・補助ファイル・依存（`performance-wdio` feature と wdio 関連の依存を含む）の削除。
- 計測と手動実行テストを消すことで使う側が無くなる `performance` feature、その `cfg` が付いたコードと分岐、環境変数 `RELEASH_PERFORMANCE_DATA_DIR`・`RELEASH_PERFORMANCE_PROVIDER_FIXTURE_EXECUTABLE`、フロントの `performance` モード、それらを確かめるテストの削除。
- 計測と手動実行テストを消すことで使う側が無くなる実行時の計測の仕組み（計測サンプルの収集 RPC とサーバのサンプル保持、`TerminalPerformanceSwitches` と画面へのその配信、画面側の計測 `terminalPerformanceProbe`、本番で送られていない描画側の起動段階の送信 `record_terminal_launch_renderer_phase`）の削除。
- `.github/workflows/ci.yml` と `.github/workflows/nightly.yml` のジョブ、キャッシュ、Lint。
- 置き場所の検査と、単体テストで外部の資源を使わない検査の追加（ast-grep の設定と規則、qlty の設定を含む）。
- `AGENTS.md` の「ビルド・テスト・Lint」と「リリース」の記述。
- `.gitignore` の `node_modules` の除外を、シンボリックリンクにも当たるようにすること。
- CI の設定を書き写して検査する `.github/scripts/workflows-test.mjs` と、それを実行する `workflow-tests` ジョブの削除。

Non-goals:
- 利用者向けの設定「Send anonymous performance metrics」（`update_performance_telemetry`）と、それで OTLP へ送る計測。
- フロントの統合テスト（`tests/integration/`）と振る舞いテスト（`tests/behavior/`）を新しく書くこと。
- `docs/architecture/TEST.md` の変更。
- `.github/scripts/coverage.test.py` の置き場所と実行箇所。
- 定数の値・削除済み機能の否定・パフォーマンス・層の重複・同じ入力区分の重複・主要パターン以外の振る舞いを lint で検出すること。

# Requirements

- R-001: プロダクトのどのテストも、TEST.md の「書かないテスト」に当たらない。
- R-002: プロダクトのすべてのテストが、TEST.md の「配置と実行」の表で種類に対応する置き場所にある。Rust の単体テストは `<impl>_test.rs` に置き、`<impl>.rs` から `#[path]` で取り込む。
- R-003: 種類ごとの実行コマンドが TEST.md の「配置と実行」の表と一致し、各コマンドはその種類のテストだけを実行する。対象のテストが0件の種類のコマンドも成功する。
- R-004: CI のジョブが、TEST.md の「配置と実行」の表の役割（サーバ・シェル・フロント）と種類ごとに分かれ、表の CI 列と一致する。ジョブ名は `<役割>-<種類>`（`server-lint`・`server-unit`・`server-integration`、`shell-lint`・`shell-unit`・`shell-integration`、`frontend-lint`・`frontend-unit`・`frontend-integration`）とし、並列に実行する。Rust のビルドは各ジョブで行う。振る舞いは nightly 層で実行する。
- R-005: 計測のためのビルド切り替え（Cargo の `performance` feature、フロントの `performance` モード）と、計測テストのためだけの実行時の仕組み（計測サンプルを集める呼び出し、計測用の A/B 切り替え、画面側の計測の差し込み口、計測時にだけ送る描画側の起動段階）が無い。利用者向けの設定で OTLP へ送る計測は変わらない。nightly 層に計測のジョブは無く、リリースの関門は計測のジョブに依存しない。
- R-006: 必須チェックは、役割ごとの集約ジョブ `server`・`shell`・`frontend` と、役割をまたぐ検査の `quality` である。各集約ジョブは、その役割の Lint・単体・統合のジョブの結果で成否が決まる。Rust に関係しない変更で Rust のジョブを飛ばしても、必須チェックは Pending のまま残らない。
- R-007: rust-cache は、ビルド条件（パッケージ・features・環境変数）が同じジョブ同士でキャッシュを共有し、保存は共有するキャッシュごとに main への push の1ジョブだけが行う。Playwright のブラウザはキャッシュしない。pnpm は `actions/setup-node` のキャッシュを使う。
- R-008: CI の Biome は `biome ci` で実行する。
- R-009: プロダクトのテストファイルが TEST.md の「配置と実行」の表の置き場所から外れている場合、`*_test.rs` と `tests/` 以外に `#[test]` がある場合、Rust の単体テストが TEST.md の「配置と実行」の対応（同じディレクトリに `<impl>.rs` があり、一つの実装に `<impl>_test.rs` が一つだけで、`#[path = "<impl>_test.rs"]` の mod 名が `<impl>_tests`。mod.rs の `<impl>` は `mod`）から外れている場合、`*_test.rs` が対応する `<impl>.rs` から `#[path]` で取り込まれていない場合、Rust の統合テストのファイルが Cargo の統合テストの入口（`tests/` 直下の `*.rs` と `[[test]]` で登録したファイル）から取り込まれていない場合、テストヘルパーが TEST.md の「テストヘルパー」の置き方（ディレクトリごとに `test_helpers.rs` 一つ）から外れている場合（`test_helpers_<名前>.rs` などの別名の補助ファイル）に、CI が落ちる。`.github/` 配下と `.ast-grep/` 配下（CI の道具のテスト）と、テストではない補助ファイル（`test_helpers.rs`・`test_support/`・`tests/helpers/`・`tests/fixtures/`・`src-tauri/tests/support/`・`src/test/` の setup 等）は検査の対象外とする。
- R-010: `src-tauri/src/` と `src-tauri/releash-desktop/src/` の単体テスト側のファイル（`*_test.rs`・`test_helpers*.rs`・`test_support/`）が `rusqlite::Connection::open*`・`git2::Repository::{init,open}`・`tempfile`・`std::process::Command`・`tokio::process`・`TcpListener`・`TcpStream` を、`use` 宣言と完全修飾のどちらで使っても、CI が落ちる。
- R-011: R-009 と R-010 の検査は、既存の違反を許可リストで除外せず、違反0件の状態で CI に入る。
- R-012: 統合テストがクレート内部に触れる入口は `test-support` feature の中だけにあり、層のモジュール（adaptor・domain・usecase など）はライブラリの外へ公開されない。
- R-013: `AGENTS.md` の「ビルド・テスト・Lint」のコマンド一覧と「リリース」の関門の記述が、TEST.md と CI に一致する。
- R-014: 手動で実行するテスト（`#[ignore]` で通常は実行されず手動で起動する前提のものを含む）がリポジトリに無い。

# Assumptions

なし
