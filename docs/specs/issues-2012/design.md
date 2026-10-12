# Design

## 変える部分
- XCUITest のターゲット: `clients/macos/project.yml` に UI テストのターゲットを足し、回帰の E2E を置く。根拠: R-001「リポジトリにある回帰の E2E を全部実行し」。ルート: 委任
- `macos:e2e` target: `clients/macos/project.json` に、UI テストのターゲットを実行する `e2e` target を足し、`generate` と `helpers` に依存させる。根拠: R-001「実行の前に `macos:generate` と `macos:helpers` が実行される」。ルート: 委任
- E2E の隔離: テストごとに一時的なデータディレクトリを作り、`launchEnvironment` の `RELEASH_DATA_DIR` でアプリに渡す。テストの後に、そのデータディレクトリのサーバを止める。根拠: R-002「テストごとに隔離したデータディレクトリを使い、本番と同じ…経路でサーバに接続する」。ルート: 委任
- E2E の隔離（workflow 定義と Facet）: workflow 定義と Facet のディレクトリ（既定はデータディレクトリの外の `~/Library/Application Support/releash/workflows`）も、テストごとの一時ディレクトリに分ける。根拠: R-002「本番のデータディレクトリと、そこを使うサーバには読み書きしない」。ルート: 委任
- bundle ID: `clients/macos/project.yml` の `PRODUCT_BUNDLE_IDENTIFIER` を build configuration ごとに分ける。根拠: R-004、R-003「本番のアプリとサーバは終了しない」。ルート: 「固定するルート」1
- 状態を用意する仕組み: 回帰の E2E が、テストの始まりに repository、worktree、実行中の workflow、Thread を用意できるようにする。根拠: R-005。ルート: 委任
- provider の代わりの TUI の実行ファイル: `server/tests/agent_tui_fixture.rs` の TUI の部分を、`test-support` feature 付きでだけビルドされる bin にする。根拠: R-006「`test-support` feature を付けたビルドでだけ作られる専用の実行ファイル」。ルート: 「固定するルート」2
- サーバの統合テストの provider: TUI を取り込む `server/tests/` の 6 本（`internal_integration.rs`、`workflow_control_plane_acceptance_test.rs`、`provider_lifecycle_acceptance_test.rs`、`terminal_surface_acceptance.rs`、`agent_tui_harness.rs`、`agent_session_tui_acceptance.rs`）を、統合テストのバイナリを起動し直す形から、専用の実行ファイルを起動する形に変える。根拠: R-006「TUI を使うサーバの統合テストも、この実行ファイルを provider として起動する」。ルート: 「固定するルート」2
- E2E の provider: 隔離したデータディレクトリの `releash.toml` の `agents.claude.cli_path`・`agents.codex.cli_path` に専用の実行ファイルを書き、`macos:e2e` はその実行ファイルのビルドを先に行う。根拠: R-006「E2E は…この実行ファイルを指定して、provider として起動させる」。ルート: 「固定するルート」2
- 器の回帰の E2E: R-007〜R-010 の 4 つの動作を確かめるテストを書く。根拠: R-007、R-008、R-009、R-010。ルート: 委任
- E2E の排他: `macos:e2e` の同じ Mac での同時実行を排他にし、後から呼んだほうを待たせる。根拠: R-013。ルート: 委任
- nightly: `.github/workflows/nightly.yml` の `nx run-many -t e2e coverage` から macos の `e2e` を外す。根拠: R-015。ルート: 委任
- TEST.md: 「種類」の表に振る舞い（macOS）を、「配置と実行」の表に単体（macOS）と振る舞い（macOS）を足す。根拠: R-016。ルート: 「固定するルート」4
- AGENTS.md: 「ビルド・テスト・Lint」に `macos:e2e` の呼び方を足す。根拠: R-017。ルート: 委任
- E2E のビルドの署名: E2E のビルドを ad-hoc 署名から、Apple Development の証明書による署名に変える。根拠: R-018「ビルドのたびに求め直さない」、B-022「designated requirement は、cdhash ではなく、bundle ID と署名の証明書に基づく」。ルート: 「固定するルート」5

## 固定するルート
1. bundle ID
   - Debug は `com.releash.macos.dev`、Release は `com.releash.macos` にする。E2E 用のビルドだけ別の ID にする形はとらない。
   - 理由: 開発版を配布版と共存させる一般的なやり方で、desktop も開発版を `com.releash.app.dev` にしている（`clients/desktop/tauri.conf.dev.json:4`）。E2E と `macos:dev` は Debug でビルドするので、どちらも本番と別の ID になる。
2. provider の代わりの TUI
   - `test-support` feature を付けたときだけビルドされる専用の実行ファイル（例: `releash-agent-tui-fixture`）にする。統合テストのバイナリを起動し直す今の形は残さない。
   - E2E は、隔離したデータディレクトリの `releash.toml` の `agents.*.cli_path` にこの実行ファイルを指定する。
   - サーバの統合テスト 6 本も、この実行ファイルを provider として起動する形に変える。
3. 証跡
   - 証跡を残す仕組み（PR へ証跡を載せる target、録画）は作らない（利用者の決定）。
4. TEST.md
   - 「種類」の表は、対象を分けているのが「振る舞い（desktop）」だけなので、「振る舞い（macOS）」だけを足す。
   - 「配置と実行」の表は、行を対象ごとに分けているので、「単体（macOS）」と「振る舞い（macOS）」を足す。
5. E2E のビルドの署名
   - ad-hoc 署名（`CODE_SIGN_IDENTITY=-`）をやめ、チーム `T6QN7PY57N` の Apple Development の証明書で署名する。
   - `clients/macos/project.yml` は `CODE_SIGN_STYLE: Automatic` なので、`DEVELOPMENT_TEAM` に `T6QN7PY57N` を指定する。
   - 理由: Apple の DTS が日々の開発に勧める方法である（https://developer.apple.com/forums/thread/776479 ）。証明書に基づく designated requirement は、ビルドし直しても変わらない。

## 変えないもの
- Release の bundle ID は `com.releash.macos` のままにする。配布するアプリの ID を変えないため。

## 未確定・リスク
- 状態の用意（R-005）と provider の設定（R-006）は、XCUITest の側からファイルの作成やプロセスの起動を行えることを前提にしている。UI テストの実行プロセスにこれらを禁じる制限があると、テストの中で用意できない。その場合は、`macos:e2e` の前段など、テストの外で用意する経路が必要になる。
