## B-001: サーバのクレートは Tauri に依存しない

GIVEN workspace にサーバのクレート `releash-backend` とシェルのクレート `releash-desktop` がある
WHEN サーバのクレートの依存を、通常・dev・build の辺、すべての target、すべての feature について `tauri` から逆にたどる
THEN `tauri` に依存する経路は 1 つも無い
AND サーバのクレートはシェルのクレートに依存していない

## B-002: desktop feature が無い

GIVEN 変更後のリポジトリ
WHEN コード（`src-tauri/` の Rust のソース、build script、Cargo.toml）を `feature = "desktop"` と `desktop` feature の定義で検索する
THEN 1 件も見つからない

## B-003: lib テストは 1 構成で 1 回

GIVEN Rust の変更を含む PR
WHEN PR の CI が走る
THEN サーバの lib テストは 1 つの構成で 1 回だけ走る
AND シェルの lib テストは 1 つの構成で 1 回だけ走る
AND 必須チェック `rust` は、サーバとシェルのジョブの結果から成否を決める

## B-004: CI で走る Tauri アプリのテストが同じ断定で成功する

GIVEN 変更前からあるシェルの Rust のテスト（`tests/desktop_*.rs`、`tests/daemon_termination.rs` を含む）と Playwright のテスト
WHEN PR の CI で実行する
THEN 断定の式と期待値を変えずに、すべて成功する

## B-016: CI で走らない確認の設定と script の追従

GIVEN wdio の設定（`wdio.performance.conf.ts`、`wdio.client-streams.conf.ts`）と、.app を確かめる script（`tests/desktop-bundle.mjs`、`tests/desktop-login.mjs`、`tests/helpers/`）
WHEN そこに書かれた実行ファイルと .app と設定ファイルのパスと名前を読む
THEN 分けた後の構成のパスと名前（`releash-desktop`、`releash-backend`、`src-tauri/releash-desktop/`）を指している
AND 断定の式と期待値は変わっていない

## B-005: シェルがサーバから使える項目

GIVEN シェルのクレート
WHEN シェルの本番のコードがサーバのクレートの項目を使う
THEN 使える項目は、サーバの lib の公開の入口 1 か所に並んだものだけである
AND 入口に並ぶ項目は、どれもシェルのどこかが使っている

## B-006: テスト用の項目はテストのときだけ

GIVEN サーバのクレートの feature `test-support`
WHEN シェルの本番のビルド（`cargo build -p releash-desktop`、`tauri build`）の feature を調べる
THEN サーバのクレートに `test-support` は有効になっていない
AND シェルのテストのビルドでだけ、`test-support` の区画の項目が使える

## B-007: サーバの振る舞いのテストは Tauri なしで走る

GIVEN 変更前にシェルの側にあり、サーバの Connect の client API の振る舞いを断定していたテスト
WHEN サーバのクレートのテストを走らせる
THEN そのテストは Tauri のアプリを作らずに、同じ断定のまま成功する

## B-008: .app の中身

GIVEN `.app` を作る 4 つの入口（`tauri:build`、`build:desktop:acceptance`、nightly と stable の tauri-action）のどれか
WHEN .app を作る
THEN `Contents/MacOS/` に `releash-desktop` と `releash-backend` が入る
AND どちらも署名の対象になる
AND .app の表示名と identifier は今と同じである
AND universal のビルドでは `releash-backend` も universal である

## B-009: .app からの起動

GIVEN 分けた後の構成で作った .app
WHEN `releash-desktop` を起動する
THEN シェルは同じ .app の `releash-backend` を起動して接続し、画面を表示する
AND ログイン項目から起動したときも、`releash-desktop` が `--hidden` 付きで起動する

## B-010: 版は 1 か所

GIVEN workspace の版が `X.Y.Z` である
WHEN 版上げ（bump-version または stable の版上げ）が走る
THEN workspace の版と、Cargo.lock の `releash-backend` と `releash-desktop` の行の版が、新しい同じ値になる
AND `package.json` と `tauri.conf.json` の版も同じ値になる

## B-011: tauri dev

GIVEN 開発者のマシン
WHEN `pnpm tauri:dev` を実行する
THEN シェルはサーバを起動して接続し、画面を表示する

## B-012: シェルの統合テストとサーバの実行ファイル

GIVEN サーバの実行ファイルを先にビルドしていない
WHEN シェルの統合テストを走らせる
THEN テストは、先に実行するコマンドを示して失敗する
AND テストの中から cargo は呼ばれない

## B-013: サーバの実行ファイルは上書きされない

GIVEN サーバの package を cargo でビルドした `target/<profile>/releash-backend` がある
WHEN シェルのクレートを `tauri build` 以外（cargo の build・clippy・test、`tauri dev`）でビルドする
THEN `target/<profile>/releash-backend` は、サーバの package の cargo の出力のままである

## B-014: feature の届け先

GIVEN nightly・stable の tauri-action が `--features vendored-openssl` を渡す
WHEN .app を作る
THEN GUI とサーバの両方の実行ファイルで、git2 は vendored の OpenSSL でビルドされる
AND `performance` を渡したビルドでは、サーバのクレートの `performance` が有効になる

## B-015: 文書のコマンド

GIVEN `AGENTS.md` のビルド・テスト・Lint の節と、`docs/architecture/README.md`・`TEST.md`
WHEN 書かれたコマンドを、書かれたディレクトリで実行する
THEN 分けた後の構成で、意図したクレートに対して実行される

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004, B-016 |
| R-005 | B-001 |
| R-006 | B-005, B-006 |
| R-007 | B-005 |
| R-008 | B-004, B-007 |
| R-009 | B-008, B-009 |
| R-010 | B-008 |
| R-011 | B-010 |
| R-012 | B-011 |
| R-013 | B-012 |
| R-014 | B-013 |
| R-015 | B-014 |
| R-016 | B-015 |
