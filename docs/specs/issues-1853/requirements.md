# Context

- 要求の正本: ISSUE #1853「[03] サーバと Tauri シェルを別のクレートに分ける」（https://github.com/siro33950/releash/issues/1853）。ISSUE 本文の file:line と数は main 4533af2f 時点のもので、この文書は main d5ee057c で読み直した事実を使う。
- 所属: マイルストーン #100「03. サーバを画面から独立させ、CLI を整える」（https://github.com/siro33950/releash/milestone/100）。着手順の 3 番目。先行の #1852（サーバの受け入れテストから Tauri への依存を抜く）と #1908（daemon ドメイン）は main に入っている。どの ISSUE の後でも、Tauri アプリ・CLI・hook が動く状態を保つ。
- 補助資料: #1852、#1902（CLI を薄いクライアント crate `releash` にする。CLI のクレートと実行ファイルに `releash` を使う）。
- 実行時には、すでにシェルとサーバは別プロセスに分かれている。シェルの実行ファイルがサーバの実行ファイル `releash-backend` を子プロセスとして起動し、Connect で通信する。
- クレートの構成は 2 つ。シェルのクレートがサーバのクレートの lib に依存し、依存の向きはシェルからサーバへの一方向だけにする。クライアント側の共有クレートは作らない。共有クレートは、サーバのクレートに依存してはならないクライアント（#1902 の CLI）ができるときに、要るものだけで作る。
- 名前: サーバの package は `releash-backend`（lib `releash_lib`、bin `releash-backend` は今のまま）。シェルの package と GUI の実行ファイルは `releash-desktop`。`releash` は #1902 のために空けておく。`releashd` への改名は #1903。
- Cargo と Tauri の決まりで、この変更に効くもの:
  - workspace は Cargo.lock と target を 1 つ共有し、1 回の cargo の実行で複数の package を選ぶと共有する依存の feature が統合される（https://doc.rust-lang.org/cargo/reference/workspaces.html 、https://doc.rust-lang.org/cargo/reference/features.html#feature-unification）。
  - `CARGO_BIN_EXE_<name>` は同じ package の integration test にだけ設定される（https://doc.rust-lang.org/cargo/reference/environment-variables.html）。
  - `#[cfg(test)]` の項目と build script の出力（OUT_DIR）は、そのクレートの外から使えない。
  - Tauri の bundler が .app に自動で入れる bin は、Tauri のクレート自身の bin だけで、別の実行ファイルを同梱して署名させる経路は `bundle.externalBin`（https://v2.tauri.app/develop/sidecar/）。

# Outcome

対象者は、Releash の開発者である。

今は、サーバ（daemon と CLI）と Tauri シェルが 1 つのクレート `releash` に入り、`desktop` feature で出し分けている。サーバのクレートが Tauri を dev-dependency に持ち、lib のテストを `desktop` あり・なしの 2 つの構成で 2 回走らせている。シェルのコードとテストが、サーバの内部（`pub(crate)` の部品やテスト用の部品）を同じクレートの中から直接使っている。

変更後は、サーバとシェルが別のクレートになり、サーバのクレートは Tauri に依存せずにビルド・テストできる。シェルがサーバから使えるものは、サーバの lib の公開の入口 1 か所に並んだものだけになる。lib のテストは、サーバもシェルも 1 つの構成で 1 回だけ走る。Tauri アプリは今と同じに動く。

# Current Behavior

確認した時点: main d5ee057c。パスは `src-tauri/` を起点とする。

- クレートは package `releash` の 1 つ（`Cargo.toml:2`）。lib は `releash_lib`（`:16`）で、`crate-type = ["staticlib", "cdylib", "rlib"]`（`:17`）。bin は `releash`（GUI、`src/main.rs`、`required-features = ["desktop"]`、`:19-21`）と `releash-backend`（`src/bin/backend.rs`、`:23-25`）。`default-run = "releash"`（`:8`）。`.app` に `releash-backend` が入るのは、GUI と同じ package の bin だからである。
- feature は `default = ["desktop"]`、`desktop`（tauri・tauri-build・tauri-plugin-*・objc2 系）、`vendored-openssl`（git2）、`performance`、`performance-wdio`（`Cargo.toml:96-101`）。`tauri` は `test` feature 付きの無条件の dev-dependency（`:109`）。
- `feature = "desktop"` を含む行は 62 行・27 ファイル（`src`・`tests`・`build.rs`）。`cfg(feature = "desktop")` で gate されたモジュールは、`desktop`（`src/lib.rs:35`）、controller の `command`・`desktop_lifecycle`（`src/adaptor/controller/mod.rs:9-13`）、presenter の `daemon_status`（`src/adaptor/presenter/mod.rs:10`）、gateway の `daemon_supervision`・`desktop_update`・`login_item`・`desktop_client`・`cli_install`（`src/adaptor/gateway/mod.rs:8-34`）、usecase の `daemon_supervision`・`desktop_update`・`login_item`・`cli_install`（`src/usecase/mod.rs:34-46`）、domain の `login_item`（`src/domain/mod.rs:24`）、infrastructure の `desktop_channel`（`src/infrastructure/mod.rs:2`）と `platform` の 9 つ（`src/infrastructure/platform/mod.rs:2-21`）。`cfg(any(test, feature = "desktop"))` のモジュールは `usecase::client_connection`（`src/usecase/mod.rs:32`）と `domain::daemon_supervision`（`src/domain/mod.rs:8`）。`usecase::test_helpers` は `cfg(all(test, feature = "desktop"))`（`src/usecase/mod.rs:39`）。
- サーバ側のファイルの中に、`desktop` のときだけ有効な項目がある: `RetryBackoff::DESKTOP_POLL`（`src/common/retry.rs:19-21`）、`LocalLogProcess::Gui`（`src/infrastructure/local_log.rs:17,26`）、起動の計測（`src/infrastructure/telemetry/metrics/mod.rs:19,493-498`、`metrics/attributes.rs:61-64,122-125`）、`matches_client`（`src/domain/daemon/identity.rs:31-34`）、`TryFrom<DesktopSettings> for DesktopSettingsDto`（`src/adaptor/presenter/client/mod.rs:135-150`）、`ClientConnectionFileQuery`（`src/adaptor/gateway/local_api.rs:189-240` 付近）、`ApplicationQuitIngress`（`src/adaptor/controller/application_lifecycle.rs:1-23`）、テスト fixture の `app_config` 欄（`src/adaptor/controller/client/workflow/mod.rs:218,1289`）。
- シェル側のコード（約 9,500 行）は、サーバ側の次のものを使っている: proto の生成型と変換（`presenter::client`、`connect_wire::{rpc,to_rpc,to_wire}`、`presenter::client::descriptor`、生成された `client_calls.rs` の include。`src/adaptor/gateway/desktop_client.rs:1-9,:460`、`gateway/login_item.rs:61-88`、`gateway/daemon_supervision.rs:1`）、発見ファイルの読み取り（`gateway/daemon_supervision.rs:79,336`、`platform/desktop_restart.rs:3,51`）、`common::retry`・`common::operation_context`・`domain::failure`・`terminate_descendants`、data dir・ログ・telemetry（`src/desktop.rs:28-35,46,63-66`）、設定の型 `DesktopSettingsDto`。サーバ側の本番コードが、シェルへ移すものを使っている箇所は無い。
- シェルは `releash.toml` をサーバを通さずに直接読み、起動を隠すかの判定とログイン項目の復元に使う（`src/desktop.rs:68-74`）。サーバを起動する前（`:72-81`）に読んでいる。
- `build.rs` は 1 本で、proto からの生成（prost の型、Connect の service、サーバの handler `client_service.rs`、呼び出し表 `client_calls.rs`、`client_commands.rs`、descriptor）、telemetry の値の埋め込み（`build.rs:3-10`）、`tauri_build::build()`（`:11-12`、desktop 時）を行う。`COMMAND_NAMES` は `#[cfg(test)]` で生成される（`build.rs:42`）。
- シェルのテストの一部は、サーバの内部を Tauri の mock app の state に入れて組み立て、Connect の client API の振る舞いを検査している（`src/adaptor/controller/command/client_test.rs`、`src/desktop_test_support.rs`）。シェルのテストの一部は、サーバの `#[cfg(test)]` の telemetry の部品を使う（`src/desktop_test.rs:79-167`）。
- シェルの統合テスト（`tests/desktop_daemon.rs:53,220,293`、`tests/desktop_status_channel.rs:59`、`tests/desktop_update.rs:139`）は `env!("CARGO_BIN_EXE_releash-backend")` でサーバの実行ファイルを起動している。
- CI は lib テストを `desktop` あり（`rust-test-desktop`、`cargo test --locked`）となし（`rust-test-headless`、`--no-default-features --lib`）で 2 回走らせている（`.github/workflows/ci.yml:216-316`）。`rust-lint` は `--no-default-features --bin releash-backend` の clippy も走らせる（`:212-214`）。`.github/scripts/workflows-test.mjs` がこれらのコマンドを固定で断定している（`:55-96`、`:163-166`、`:210`、`:244-306`、`:435-437`）。
- 版上げ（`.github/workflows/bump-version.yml:38-43`、`stable.yml:201-206`）は、`src-tauri/Cargo.toml` の `version` と、`src-tauri/Cargo.lock` の `name = "releash"` の直後の `version` を書き換える。`workflows-test.mjs:560-567`、`:944-983` がこれを断定している。
- `.app` を作る入口は 4 つ（`package.json:15` の `tauri:build`、`:33` の `build:desktop:acceptance`、`nightly.yml:252-261` と `stable.yml:146-155` の tauri-action）。サーバの実行ファイルは `beforeBundleCommand`（`tauri.conf.json:10`）の `scripts/build-desktop-backend.mjs` が arch ごとにビルドし、universal のときは lipo で 1 つにする。
- GUI の実行ファイル名 `releash` は、`launchagents/com.releash.app.plist:6-7`、`wdio.performance.conf.ts:6`、`wdio.client-streams.conf.ts:9`、`tests/helpers/desktop-bundle.mjs:41,52` に現れる。

# Scope / Non-goals

## Scope

- `src-tauri/` を Cargo workspace にし、root package をサーバのクレート `releash-backend`、その下の 1 段のサブディレクトリに Tauri シェルのクレート `releash-desktop` を置く。版は workspace で 1 か所に持つ。
- `desktop` で gate されたモジュール、Tauri を参照するファイル、シェルだけが使うものをシェルのクレートへ移す。`desktop` feature を削除する。サーバのクレートから `tauri`・`tauri-build`・tauri-plugin-*・objc2 系の依存（dev-dependency を含む）と `staticlib`・`cdylib` を外す。
- サーバの lib に、シェルが使う項目だけを並べる公開の入口を 1 つ作る。シェルのテストだけが使う項目は、サーバの feature `test-support` で gate した区画に並べる。
- サーバの振る舞いを確かめているシェルのテストを、Tauri のアプリなしで動くサーバのクレートのテストにする。シェルのクレートには、シェルの振る舞いを確かめるテストだけを残す。
- `build.rs` をサーバ側（proto からの生成一式と telemetry の値の埋め込み）とシェル側（`tauri_build::build()`）に分ける。
- サーバの実行ファイルを、.app を作るときだけ `bundle.externalBin` で同梱する。
- CI、nightly・stable・版上げ・dependabot、`workflows-test.mjs`、`package.json` の scripts、`scripts/`、wdio、`tests/*.mjs`、`deny.toml`、`.gitignore`、`codecov.yml`、`AGENTS.md`、`docs/architecture/README.md`・`TEST.md` のコマンドとパスを追従させる。

## Non-goals

- クライアント側の共有クレート（#1902 で作る）。
- `releashd` への改名（#1903）。CLI の `releash` の新設（#1902）。
- シェルが `releash.toml` を直接読んでいること（`src/desktop.rs:68-74`）は変えない。所在: シェルがサーバを通さずに設定ファイルを読んでいる。`AGENTS.md` の「ロジックと外部リソースへのアクセスはサーバに置く」に合っていない。受け取る経路の見直しは #1904。
- 実行ファイルの名前が変わる前の版（GUI の実行ファイルが `releash`）から更新したとき、更新後の自動再起動が失敗し、手で起動し直す必要がありうること。今の版の再起動は、自分の実行ファイルのパス（`Contents/MacOS/releash`）を起動し直す（`src/infrastructure/platform/desktop_restart.rs:5-6`、`src/usecase/desktop_update.rs:56-68`）ので、この ISSUE の変更では直せない。未確認の事項: 実行中のプロセスの `current_exe` が .app の差し替え後に何を返すか、tauri-plugin-updater の install が .app を丸ごと置き換えるか、登録済みのログイン項目（`src/infrastructure/platform/login_item.rs:14-16`、plist の `BundleProgram`）が更新後の plist の新しい名前で読み直されるか。
- #1852 で `debug_assertions` で公開したサーバ用の受け入れハーネスの公開の仕方。
- HTTP `/v1` を使う受け入れテストの経路（#1902）。
- ディレクトリ名 `src-tauri/` の整理（#1854）。
- `docs/specs/` の過去の ISSUE の文書。

# Requirements

- R-001: サーバのクレートは、Tauri に依存しない（通常・dev・build のどの依存にも、どの target・どの feature の組み合わせでも、`tauri` が現れない）。
- R-002: コードに `feature = "desktop"` が残っていない。
- R-003: PR の CI で、サーバの lib テストは 1 つの構成で 1 回、シェルの lib テストは 1 つの構成で 1 回だけ走る。nightly の `--features performance` の lib テストは、目的が別なので残す。
- R-004: Tauri アプリは今と同じに動く。PR の CI で走るもの（シェルのクレートの Rust のテスト。既存の `tests/desktop_*.rs` と `tests/daemon_termination.rs` を含む。Playwright）は、断定を変えずに成功する。CI で走らない wdio の設定（`wdio.performance.conf.ts`、`wdio.client-streams.conf.ts`）と、.app を確かめる script（`tests/desktop-bundle.mjs`、`tests/desktop-login.mjs`、`tests/helpers/`）は、断定を変えずに、分けた後のパスと名前に追従している。
- R-005: 依存の向きはシェルのクレートからサーバのクレートへの一方向だけである。サーバのクレートは、シェルのクレートにも Tauri にも依存しない。
- R-006: シェルがサーバのクレートから使える項目は、サーバの lib の公開の入口 1 か所に並んだものだけである。並ぶのは、シェルが実際に使う項目だけである。層のモジュールは公開しない。シェルのテストだけが使う項目は、feature `test-support` を有効にしたときだけ公開される別の区画に並び、シェルの本番のビルドにはこの feature が入らない。
- R-007: シェルだけが使うもの（シェルの domain・usecase・gateway・presenter・controller・infrastructure の部品、`ApplicationQuitIngress`、`usecase::test_helpers`、`RetryBackoff::DESKTOP_POLL` の値）は、シェルのクレートにある。別のクレートからは足せないもの（enum の値、型のメソッド、サーバの型どうしの変換、サーバの非公開の部品を使う処理）と、サーバの domain の規則は、サーバのクレートに残る。
- R-008: サーバの振る舞いを確かめているテストは、サーバのクレートのテストとして Tauri のアプリなしで走る。シェルのクレートのテストは、シェルの振る舞い（Tauri コマンドの登録と受付、監督、ウィンドウ、tray、更新、ログイン項目、設定の反映）を確かめる。どのテストも、断定の式と期待値は変わらない。
- R-009: 作られた .app の `Contents/MacOS/` に、GUI の実行ファイル `releash-desktop` とサーバの実行ファイル `releash-backend` が入り、どちらも署名の対象になる。.app の表示名と identifier は今と同じである。ログイン項目の plist は `releash-desktop` を起動する。
- R-010: `.app` を作る 4 つの入口（`tauri:build`、`build:desktop:acceptance`、nightly と stable の tauri-action）のすべてで、サーバの実行ファイルが .app に入る。universal のビルドでは、サーバの実行ファイルも universal である。
- R-011: サーバのクレートとシェルのクレートの版は、workspace の 1 か所で決まり、同じ値である。版上げ（bump-version と stable の版上げ）は、その 1 か所と Cargo.lock の両方の package の行を書き換える。
- R-012: `tauri dev` で、シェルは今と同じにサーバを起動して接続できる。
- R-013: シェルの統合テストは、先にビルドされたサーバの実行ファイルを使う。見つからないときは、先に実行するコマンドを示して失敗する。テストの中から cargo を呼ばない。
- R-014: サーバのクレートのビルド・テストで、`target/<profile>/releash-backend` がサーバの package の cargo の出力以外のもので置き換わらない。
- R-015: nightly と stable の `vendored-openssl`、`performance`、`performance-wdio` の指定は、今と同じ効果を持つ（`vendored-openssl` と `performance` はサーバのクレートの feature に届く）。
- R-016: `AGENTS.md` のビルド・テスト・Lint のコマンドと実行ファイルの説明、`docs/architecture/README.md`・`TEST.md` のパスとコマンドは、分けた後の構成で正しい。

# Assumptions

なし
