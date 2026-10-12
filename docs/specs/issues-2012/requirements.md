# Context

- 入力: https://github.com/siro33950/releash/issues/2012
- テストと CI の方針の正本は、マイルストーン #78（https://github.com/siro33950/releash/milestone/78 ）の説明文の「テストと CI の方針」である。
  - 回帰の E2E は、機能ごとに主たる動作だけを確かめる。リポジトリに入る XCUITest は回帰の E2E だけである。
  - CI で回帰の E2E を回すのは #1766 からである。それまでは、画面の機能を足す・変える PR で、手元で回帰の E2E を全部回す。
- テストの種類・置き場所・実行の正本は `docs/architecture/TEST.md` である。Nx の `test` target は単体と統合を、`e2e` target は振る舞いを実行する。
- 依存する #1204（器）と #2037（Nx の target）は main に入っている（#2037 は PR #2047、merge commit `b1c16576`）。
- XCUITest の制約:
  - 対象のアプリを前面に出す必要があり、画面がロックされた状態では動かない（https://developer.apple.com/forums/thread/840965 ）。同じ Mac で同時に複数は回せない。
  - `XCUIApplication.launch()` は、bundle ID で対象のアプリを特定する。同じ bundle ID のアプリが動いていれば、終了させてから起動する。
- macOS のプライバシーの許可（TCC）は、アプリの署名の designated requirement で、同じアプリかどうかを判断する。ad-hoc 署名のアプリは designated requirement が cdhash になり、ビルドのたびに変わるので、ビルドのたびに別のアプリとして許可を求められる。Apple の DTS は、日々の開発には Apple Development の証明書で署名するよう勧めている（https://developer.apple.com/forums/thread/776479 ）。
- ISSUE 本文との食い違い（利用者の決定）:
  - 証跡を残す仕組みはすべて作らない。PR へ証跡を載せる target、録画を残す設定、`.xcresult` からの録画の取り出し、録画の圧縮、PR への動画の添付が対象である。
  - ISSUE の「最初に確かめること」（録画が残ること、録画の大きさ、添付の上限を超えた場合の扱い）は、録画を作らないので行わない。
  - ISSUE は TEST.md の「種類の表」に単体（macOS）と振る舞い（macOS）の行を足すとしている。しかし TEST.md の「種類」の表で対象を分けているのは「振る舞い（desktop）」だけである。そこで、「種類」の表には振る舞い（macOS）だけを足し、「配置と実行」の表に単体（macOS）と振る舞い（macOS）を足す。

# Outcome

対象者は、Releash のネイティブ UI（SwiftUI クライアント）を開発する人と agent である。

今は、macOS のアプリの機能を、ユーザと同じ操作で確かめる手段がない。テストのターゲットは単体だけで、隔離した環境でアプリを起動する仕組みもない。このため、マイルストーン #78 の [03] 以降の ISSUE は、方針どおりに回帰の E2E を足すことができない。

変更後は、開発者が次のことをできる。
- 回帰の E2E を 1 つのコマンドで全部回す。本番の Releash が動いている Mac でも回せる。

あわせて、器（#1204）の主たる動作に回帰の E2E が付く。

# Current Behavior

開発開始時点（main `b1c16576`）で確認した状態。

macOS のアプリのテスト:
- テストのターゲットは単体の `ReleashTests` だけである。scheme の test もこれだけを回す（`clients/macos/project.yml:63`、`:74-75`）。UI テストのターゲットと test plan は無い。
- bundle ID は `com.releash.macos` の一つだけで、Debug と Release で同じである（`clients/macos/project.yml:56`）。desktop は開発版を `com.releash.app.dev` にして本番と分けている（`clients/desktop/tauri.conf.dev.json:4`）。
- macos の Nx target は `generate`、`helpers`、`build`、`test`、`dev` で、`e2e` の target は無い（`clients/macos/project.json`）。`dev` は `RELEASH_DATA_DIR` を外して Debug のアプリを開く（`clients/macos/project.json:122`）。

サーバの起動と隔離:
- アプリは、同梱した `Contents/Helpers/releash` でサーバを探し、居なければ `server start` で起動する（`clients/macos/Sources/AppModel.swift:60-68`、`clients/macos/Sources/CLI.swift:53-63`）。CLI は、アプリの環境変数を変えずに起動する（`clients/macos/Sources/CLI.swift:37-45`）。
- CLI は daemon を `--data-dir` 付きで起動し、環境変数を消さない（`clients/cli/src/daemon.rs:93-98`）。データディレクトリは `RELEASH_DATA_DIR` から決まる（`clients/cli/src/data_dir.rs`、`server/src/infrastructure/platform/data_dir.rs:58-67`）。
- daemon は、`{data_dir}/releash.toml` から provider の実行ファイル（`agents.claude.cli_path`、`agents.codex.cli_path`）と repository の一覧（`app.last_repo_paths`）を読む（`server/src/adaptor/controller/daemon.rs:237-255`、`server/src/adaptor/gateway/app_config/repository_impl.rs:157-178`、`server/src/adaptor/gateway/app_config/config_models.rs:143`）。
- CLI に workflow を開始するコマンドは無い（`clients/cli/src/workflow.rs` のサブコマンドは `Diagnostics`、`Status`、`Output`）。

provider の代わりの TUI:
- `server/tests/agent_tui_fixture.rs` の `test_agent_tui_fixture_process`（`:221`）が TUI として動く。単独の実行ファイルではない。統合テストのバイナリに `#[path]` で取り込まれ、環境変数 `RELEASH_AGENT_TUI_FIXTURE_PLAN` があるときだけ動く。
- 取り込んでいる統合テストは `server/tests/` の 6 本である（`internal_integration.rs`、`workflow_control_plane_acceptance_test.rs`、`provider_lifecycle_acceptance_test.rs`、`terminal_surface_acceptance.rs`、`agent_tui_harness.rs`、`agent_session_tui_acceptance.rs`）。これらのテストは、統合テストのバイナリを `--exact` で起動し直す sh スクリプトを書き出し、provider の実行ファイルとして使う（`server/tests/workflow_control_plane_acceptance_test.rs:28-75`、`server/tests/agent_tui_fixture.rs:495-508`）。

CI と文書:
- PR の CI の `macos-checks` job は、macOS の runner で `./nx run macos:test` を実行する（`.github/workflows/ci.yml:105-134`）。E2E を回す job は無い。
- nightly は、ubuntu の runner で `./nx run-many -t e2e coverage --parallel=1` を実行する（`.github/workflows/nightly.yml:99`）。
- `docs/architecture/TEST.md` の「種類」の表（`:12`）と「配置と実行」の表（`:37`）に、macOS の行は無い。
- `AGENTS.md` の「ビルド・テスト・Lint」に、`macos:e2e` は無い。

# Scope / Non-goals

Scope:
- macOS のアプリの XCUITest のターゲットと、回帰の E2E を全部回す `macos:e2e` target
- E2E の隔離（データディレクトリ、bundle ID）と、テストごとに状態を用意する仕組み
- provider の代わりの TUI の専用の実行ファイル化と、それを使う形へのサーバの統合テスト 6 本の変更
- 器（#1204）の回帰の E2E 4 本
- nightly の `run-many` の対象から macos の `e2e` を外すこと
- `docs/architecture/TEST.md` と `AGENTS.md` の追記

Non-goals:
- 証跡を残す仕組み（PR へ証跡を載せる target、録画を残す設定、`.xcresult` からの録画の取り出し、録画の圧縮、PR への動画の添付）
- ISSUE の「最初に確かめること」（録画の実測と、添付の上限を超えた場合の扱いの決定）
- CI で回帰の E2E を回すこと（#1766 で扱う）
- 器以外の機能の回帰の E2E（[03] 以降の各 ISSUE で扱う）

# Requirements

- R-001: `./nx run macos:e2e` は、リポジトリにある回帰の E2E を全部実行し、すべて通れば成功、一つでも落ちれば失敗で終わる。実行の前に `macos:generate` と `macos:helpers` が実行される。
- R-002: E2E で起動したアプリは、テストごとに隔離したデータディレクトリを使い、本番と同じ「同梱の CLI でサーバを探し、居なければ起動する」経路でサーバに接続する。本番のデータディレクトリと、そこを使うサーバには読み書きしない。
- R-003: 本番の Releash（アプリとサーバ）が動いている Mac で `macos:e2e` を回せる。回している間も、本番のアプリとサーバは終了しない。
- R-004: macOS のアプリの bundle ID は、Debug ビルドが `com.releash.macos.dev`、Release ビルドが `com.releash.macos` である。
- R-005: 回帰の E2E は、テストごとに、repository、worktree、実行中の workflow、Thread を必要な分だけ用意した状態から始められる。
- R-006: provider の代わりの TUI は、`test-support` feature を付けたビルドでだけ作られる専用の実行ファイルである。E2E は、隔離したデータディレクトリの `releash.toml` の `agents.claude.cli_path`・`agents.codex.cli_path` にこの実行ファイルを指定して、provider として起動させる。TUI を使うサーバの統合テストも、この実行ファイルを provider として起動する。
- R-007: 回帰の E2E（器）: サーバが居ない状態からアプリを起動すると、アプリがサーバを起動して接続する。
- R-008: 回帰の E2E（器）: サイドバーに、repository ごとに worktree のカードが表示される。
- R-009: 回帰の E2E（器）: 中央を 2 つの pane に分割し、タブを別の pane へ移せる。
- R-010: 回帰の E2E（器）: 作成ダイアログの 1 回の操作で、worktree が作られ、指定したものが起動する。
- R-013: `macos:e2e` は、同じ Mac で同時に 1 つしか回らない。実行中に後から呼んだほうは、先の実行が終わるまで待ってから回る。
- R-015: nightly の `nx run-many -t e2e coverage` は、macos の `e2e` を実行しない。
- R-016: `docs/architecture/TEST.md` の「種類」の表に振る舞い（macOS）の行が、「配置と実行」の表に単体（macOS）と振る舞い（macOS）の行がある。
- R-017: `AGENTS.md` の「ビルド・テスト・Lint」に、`macos:e2e` の呼び方が書かれている。
- R-018: E2E を繰り返し実行しても、macOS は、E2E のためにビルドしたアプリへのアクセスの許可を、ビルドのたびに求め直さない。

# Assumptions

- XCUITest は、画面がロックされておらず、アプリを前面に出せる Mac で実行する。同じ Mac で XCUITest を同時に複数回さない。
