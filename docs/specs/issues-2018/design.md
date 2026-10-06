# Design

## 変える部分
- 書かないテストの削除: 削除済み機能の否定・定数の値・パフォーマンス・層の重複・同じ入力区分の重複・単体で証明できる統合・統合で証明できる振る舞いに当たるテストを消す。計測専用の設定・補助ファイル（`playwright.performance.config.ts`、`wdio.performance.conf.ts`、`wdio.client-streams.conf.ts`、`tests/terminal-performance.spec.ts`、`tests/tauri-performance/`、`src/test/performance/`、`tests/helpers/performance-daemon.mjs` と自己テスト、`package.json` の `test:performance*`・`test:client-streams:macos`、Rust の `#[ignore]` 計測）も消す。根拠: R-001「プロダクトのどのテストも、TEST.md の『書かないテスト』に当たらない」。ルート: 委任
- 手動で実行するテストの削除: 手動で実行するテスト（`tests/desktop-bundle.mjs`・`tests/desktop-login.mjs`・`tests/helpers/desktop-bundle.mjs`、手動で起動する前提の `#[ignore]` テスト）と、それだけが使うもの（`package.json` の `build:desktop:acceptance`・`test:desktop:bundle`・`test:desktop:login`、`performance-wdio` feature、`tauri-plugin-wdio`・`tauri-plugin-wdio-webdriver`・`@wdio/*` 依存、`src/main.tsx` の wdio 読み込み、wdio 用の Tauri 設定）も消す。ほかのテストが子プロセスとして起動するための `#[ignore]`（`daemon_test.rs:180`、`local_log_test.rs:118,160`）は手動実行ではないので消さない。根拠: R-014「手動で実行するテストがリポジトリに無い」。ルート: 委任
- `performance` feature の削除: `src-tauri/Cargo.toml` と `src-tauri/releash-desktop/Cargo.toml` の `performance` feature、`cfg(feature = "performance")`・`cfg!(feature = "performance")` が付いたコードと分岐（`daemon.rs:692-700`、`app_data_dir.rs:6-7`、`path_aliases.rs:28`、`cli/common.rs:479`、`cli_install.rs:35,39,66,308`、`desktop_client_acceptance.rs:125`）、環境変数 `RELEASH_PERFORMANCE_DATA_DIR`・`RELEASH_PERFORMANCE_PROVIDER_FIXTURE_EXECUTABLE`、それを確かめるテスト（`desktop_cli_install.rs`、feature で期待値を変える単体テスト）、フロントの `performance` モード（`package.json` の `build:performance`、`src/main.tsx:21` の分岐、`src/main.test.tsx` の該当テスト）、`scripts/build-desktop-backend.mjs` の `--performance`、`tauri.conf.bundle.performance.json` を消す。根拠: R-005「計測のためのビルド切り替え…が無い」。ルート: 委任
- 実行時の計測の仕組みの削除: 計測サンプルの収集・取得の RPC と、それにしか使われない message、サーバのサンプル保持と入力 trace、`TerminalPerformanceSwitches`（環境変数・usecase/gateway/presenter の型と変換・画面への配信・サーバと画面の分岐。分岐は false の側を残す）、`src/lib/terminalPerformanceProbe.ts` と `src/lib/terminalPerformanceSwitches.ts` と `useTerminal.ts` のその分岐、描画側の起動段階の送信（`record_terminal_launch_renderer_phase` の RPC と message、`TerminalLaunch` の `FirstXtermParsed`・`FirstPaint`、画面側の送信処理）を消す。サーバ側の起動段階の OTLP への記録は残し、サンプル保持だけ外す。根拠: R-005「計測テストのためだけの実行時の仕組み…が無い」。ルート: 「固定するルート」3
- 偽サーバ相手の Playwright の移設: ルート `tests/*.spec.ts` のうち書かないテストに当たらないものを、`src/` の vitest の単体テストへ移し、偽の Connect サーバ（`tests/helpers/tauri-mock.ts`）と Playwright の設定を、使われなくなる分だけ消す。根拠: R-002「種類に対応する置き場所にある」。ルート: 委任
- src 内の統合テストの移設: `src-tauri/src/` と `src-tauri/releash-desktop/src/` にある統合テスト（実 SQLite・実 git・`tempfile`・外部プロセス・TCP を使うもの）を、`src-tauri/tests/` と `src-tauri/releash-desktop/tests/` へ移す。根拠: R-002、R-010「単体テスト側のファイルが…を使っても、CI が落ちる」。ルート: 「固定するルート」1
- tests 内の単体テストの移設: `src-tauri/tests/state_subscription_scenarios.rs`（`include!`）と `tests/state_subscription_reads/reads_test.rs`（`#[path]` でライブラリへ取り込み）を、`src-tauri/src/` の `<impl>_test.rs` へ移す。根拠: R-002。ルート: 委任
- `*_test.rs` 以外にある `#[test]` の移設: `<impl>.rs` 内の `#[cfg(test)] mod tests { … }` を `<impl>_test.rs` へ分け、`#[path]` で取り込む。根拠: R-002「Rust の単体テストは `<impl>_test.rs` に置き、`<impl>.rs` から `#[path]` で取り込む」。ルート: 委任
- 内部への入口の一本化: `src-tauri/src/lib.rs` の `#[cfg(debug_assertions)] pub mod *_acceptance` と `acceptance_test_support` を、`test-support` feature の入口へ寄せる。根拠: R-012「統合テストがクレート内部に触れる入口は `test-support` feature の中だけにあり」。ルート: 「固定するルート」1
- 実行コマンド: `package.json` の `test:integration` を `tests/integration/` の Playwright（対象0件で成功）に、`test:behavior` を `tests/behavior/` の実行（対象0件で成功）にする。`lint` の CI での実行を `biome ci` にする。根拠: R-003、R-008。ルート: 委任
- CI の Rust ジョブ: `rust-test-desktop`・`rust-test-backend` を `rust-unit`（`cargo test --lib --bins` と `cargo test --doc`）と `rust-integration`（`cargo build --locked -p releash-backend --bin releash-backend` の後に `cargo test --test '*'`）に置き換え、どちらも両パッケージを対象にする。集約ジョブ `rust` の `needs` を `rust-lint`・`rust-unit`・`rust-integration` にする。変更の検知はステップの `if` のまま。根拠: R-004、R-005、R-006。ルート: ISSUE「CI と Lint の作り方」のジョブ構成
- CI のフロントジョブ: 単体は `frontend` ジョブ、統合は `integration` ジョブのまま分け、`integration` ジョブの `pnpm test:integration` は `tests/integration/` を実行する。`integration` ジョブの Playwright のブラウザのキャッシュを消す。`frontend` ジョブの Biome を `biome ci` にする。根拠: R-004、R-007、R-008。ルート: 委任
- nightly: `performance` ジョブを消し、`release` の `needs` から外す。振る舞いのジョブ（`pnpm test:behavior`）を置く。根拠: R-004、R-005。ルート: 委任
- rust-cache: ビルド条件が同じジョブの `shared-key` を揃え、`save-if` で保存を main への push の1ジョブに絞る。根拠: R-007。ルート: 委任
- 置き場所の検査: `git ls-files` の結果を TEST.md の「配置と実行」の表と突き合わせ、外れたテストファイルと、`*_test.rs` と `tests/` 以外の `#[test]` を検出するスクリプトを CI で実行する。`.github/` 配下と補助ファイルは対象外。根拠: R-009、R-011。ルート: 委任
- 外部の資源を使わない検査: `sgconfig.yml` と規則ディレクトリを置き、ast-grep の規則を qlty の ast-grep プラグインで実行する。根拠: R-010、R-011。ルート: 「固定するルート」2
- CI 構成のテスト: `.github/scripts/workflows-test.mjs` の期待値を、変更後の ci.yml・nightly.yml・AGENTS.md の構成に合わせる。根拠: R-004、R-006、R-007、R-013（このテストは CI と AGENTS.md の構成そのものを検査しており、変更後も `workflow-tests` ジョブが通る必要がある）。ルート: 委任
- AGENTS.md: 「ビルド・テスト・Lint」のコマンド一覧と「リリース」2の関門を、上記の結果に合わせる。根拠: R-013。ルート: 委任

## 固定するルート
1. 統合テストがクレート内部に触れる入口
   - 統合テストが使う内部の型と関数は、`#[cfg(feature = "test-support")]` を付けたテスト用の入口モジュールから、必要なものだけ再公開する。`src-tauri/src/desktop_api.rs:60` の `test_support` と同じ形にする。
   - adaptor・domain・usecase などの層のモジュール自体は `pub` にしない。`desktop_api.rs` の compile_fail doctest が守る「シェルから内部を import できない」性質を保つため。
   - 実行コマンドに feature の指定を足さない。`releash-backend` の `[dev-dependencies]` に自分自身を `features = ["test-support"]` 付きで加える方法を使う（`releash-desktop` の dev-dependencies が backend を test-support 付きで参照しているのと同じ形）。
   - 既存の `#[cfg(debug_assertions)] pub mod *_acceptance` と `acceptance_test_support` は、同じ意味の二つ目の書き方を残さないため、test-support の入口へ寄せる。
2. ast-grep の実行
   - qlty の ast-grep プラグインで実行する。qlty の作業領域から規則ディレクトリが見えない場合は、`ast-grep/action` を別ステップで実行する。
3. proto から消すもの
   - 消す RPC のメソッドと、それにしか使われない message は削除する。
   - 他の message の中のフィールド（切り替えを画面へ配信するフィールドなど）や、enum の値・oneof の分岐を消すときは、番号と名前を `reserved` にする。
4. 「無いこと」を述べる受入条件の確かめ方
   - B-005・B-016・B-017 を確かめるテストは書かない（TEST.md の書かないテスト: 削除済み機能が存在しないことを確かめるテスト）。差分の確認（grep やビルドの確認）で確かめる。

## 変えないもの
- `.github/scripts/` の CI 用スクリプトのテストの置き場所と実行箇所（ci.yml の `workflow-tests`、nightly.yml の coverage）は変えない。プロダクトのテストではなく、TEST.md の表の対象外のため。
- nextest の archive は使わず、ビルドはジョブ間で共有しない。
- Rust に関係しない変更での飛ばし方は、ワークフロー単位ではなくステップの `if` のままにする。ワークフロー単位で飛ばすと必須チェックが Pending のまま残るため。

## 未確定・リスク
- `releash-backend` の `[dev-dependencies]` に自分自身を加える方法が Cargo で成り立たず、実行コマンドに `--features test-support` が必要になる場合、TEST.md の表と食い違う。その場合は実装せずに報告する。
