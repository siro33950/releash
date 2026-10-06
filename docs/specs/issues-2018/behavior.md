## B-001: 書かないテストが無い

GIVEN リポジトリのプロダクトのテスト（`src-tauri/`・`src/`・ルートの `tests/`）
WHEN 各テストを TEST.md の「書かないテスト」と照らす
THEN どのテストも「書かないテスト」のいずれにも当たらない

## B-002: 種類と置き場所が一致する

GIVEN リポジトリのプロダクトのテスト
WHEN 各テストの種類（関わる仕組みと操作）と置き場所を照らす
THEN すべてのテストが TEST.md の「配置と実行」の表で、その種類に対応する置き場所にある
AND Rust の単体テストは `<impl>_test.rs` にあり、`<impl>.rs` から `#[path]` で取り込まれている

## B-003: 種類ごとのコマンドがその種類だけを実行する

GIVEN TEST.md の「配置と実行」の表の各行
WHEN その行の実行コマンドを実行する
THEN その行の置き場所にあるテストだけが実行され、成功する
AND 対象のテストが0件の行のコマンドも成功する

## B-004: CI のジョブが種類ごとに分かれる

GIVEN main への pull request
WHEN CI が実行される
THEN サーバ・シェル・フロントのそれぞれで、Lint・単体・統合のジョブが `<役割>-<種類>` の名前で並列に実行される
AND 振る舞いのテストは PR 層では実行されず、nightly 層で実行される

## B-005: 計測のためのビルド切り替えが無い

GIVEN `releash-backend` と `releash-desktop`
WHEN `performance` feature を指定してビルドする
THEN その feature は存在せず、ビルドは失敗する
AND フロントには `performance` モードのためのビルドコマンドと分岐が無い

## B-017: 計測テストのためだけの実行時の仕組みが無い

GIVEN 画面とサーバの間の呼び出しの契約（`proto/client.proto`）
WHEN 計測サンプルの収集・取得の呼び出し、端末の計測用 A/B 切り替えの値、描画側の起動段階（first_xterm_parsed・first_paint）を記録する呼び出しを探す
THEN いずれも契約に無い
AND 環境変数 `RELEASH_PERF_DISABLE_*` を設定しても、端末の動作は変わらない

## B-018: 利用者向けの計測は変わらない

GIVEN 「Send anonymous performance metrics」が有効
WHEN 端末を起動する
THEN サーバ側の起動段階の時間が、これまでどおり OTLP へ送られる

## B-006: nightly のリリース関門に計測のジョブが無い

GIVEN nightly.yml
WHEN nightly が起動する
THEN 計測のジョブは実行されず、リリースはそれを待たずに PR 層の検証一式の成功で進む

## B-007: 必須チェックの名前と集約

GIVEN main への pull request
WHEN CI が完了する
THEN 必須チェック `server`・`shell`・`frontend`・`quality` が報告される
AND `server`・`shell`・`frontend` は、その役割の Lint・単体・統合のジョブのどれかが失敗したときに失敗する

## B-008: Rust に関係しない変更で必須チェックが残らない

GIVEN `docs/` 配下とルートの `*.md` だけを変える pull request
WHEN CI が完了する
THEN Rust のテストは実行されず、必須チェック `server` と `shell` は成功として報告される

## B-009: キャッシュの共有と保存

GIVEN ビルド条件が同じ Rust のジョブ
WHEN CI が実行される
THEN それらのジョブは同じ rust-cache を復元する
AND rust-cache を保存するのは main への push のときの1ジョブだけである
AND Playwright のブラウザはキャッシュから復元されない

## B-010: Biome を CI 用のコマンドで実行する

GIVEN main への pull request
WHEN CI の Biome の検査が実行される
THEN `biome ci` で実行される

## B-011: 置き場所の違反で CI が落ちる

GIVEN プロダクトのテストファイルを TEST.md の表の置き場所の外に置くか、`*_test.rs` と `tests/` 以外に `#[test]` を書いた変更
WHEN CI が実行される
THEN 置き場所の検査が違反したファイルを示して失敗する

## B-019: テストの取り込みと実装との対応の違反で CI が落ちる

GIVEN 同じディレクトリに対応する `<impl>.rs` が無い `*_test.rs`、対応する `<impl>.rs` から取り込まれていない `*_test.rs`、一つの実装に二つ以上の test ファイルを取り込む変更、`#[path = "<impl>_test.rs"]` の mod 名を `<impl>_tests` 以外にした変更、または `src-tauri/tests/`・`src-tauri/releash-desktop/tests/` のサブディレクトリにあってテストを持つファイルを、統合テストの入口（`tests/` 直下の `*.rs` と `[[test]]` で登録したファイル）から取り込まない変更
WHEN CI が実行される
THEN 置き場所の検査が違反したファイルを示して失敗する

## B-020: テストヘルパーの置き方の違反で CI が落ちる

GIVEN `src-tauri/src/` または `src-tauri/releash-desktop/src/` に `test_helpers.rs` 以外の名前のテストヘルパー（`test_helpers_<名前>.rs` など）を置く変更
WHEN CI が実行される
THEN 置き場所の検査が違反したファイルを示して失敗する

## B-012: 単体テストが外部の資源を使うと CI が落ちる

GIVEN `src-tauri/src/` または `src-tauri/releash-desktop/src/` の `*_test.rs`・`test_helpers*.rs`・`test_support/` で、`rusqlite::Connection::open*`・`git2::Repository::{init,open}`・`tempfile`・`std::process::Command`・`tokio::process`・`TcpListener`・`TcpStream` を `use` 宣言または完全修飾で使う変更
WHEN CI が実行される
THEN 外部の資源を使わない検査が違反箇所を示して失敗する

## B-013: 検査は許可リスト無しで通る

GIVEN このISSUEの変更を入れた main
WHEN 置き場所の検査と外部の資源を使わない検査を実行する
THEN どちらも許可リストによる除外無しで違反0件で成功する

## B-014: 内部への入口は test-support の中だけ

GIVEN `releash-backend` のライブラリ
WHEN `test-support` feature を有効にせずに外のクレートから adaptor・domain・usecase の型を import する
THEN コンパイルが失敗する
AND `src-tauri/tests/` の統合テストは TEST.md の実行コマンド `cargo test --test '*' -p releash-backend` で、feature を指定せずにコンパイル・実行できる

## B-015: AGENTS.md のコマンドが規約と CI に一致する

GIVEN `AGENTS.md` の「ビルド・テスト・Lint」と「リリース」
WHEN TEST.md の「配置と実行」の表と ci.yml・nightly.yml と照らす
THEN 記載されたコマンドとリリースの関門が一致する

## B-016: 手動で実行するテストが無い

GIVEN リポジトリのプロダクトのテスト
WHEN 各テストの実行のされ方を確かめる
THEN 手動での起動を前提とするテスト（使い捨てのアカウントや手動の段階実行を前提とするもの、手動で起動する前提の `#[ignore]` テスト）は無い

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005, B-006, B-017, B-018 |
| R-006 | B-007, B-008 |
| R-007 | B-009 |
| R-008 | B-010 |
| R-009 | B-011, B-019, B-020 |
| R-010 | B-012 |
| R-011 | B-013 |
| R-012 | B-014 |
| R-013 | B-015 |
| R-014 | B-016 |
