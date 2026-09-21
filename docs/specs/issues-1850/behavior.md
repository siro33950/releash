## B-001: PR と main への push で同じジョブ一式

GIVEN `ci.yml` を持つリポジトリ
WHEN main 宛の PR を出す、または main へ push する
THEN どちらの場合も同じジョブ一式が起動する

## B-002: PR 層が実行する Rust の検証

GIVEN Rust のコードを含む変更の PR
WHEN PR 層が実行される
THEN `cargo fmt --check`、既定 feature の `cargo clippy --locked -- -D warnings`、`cargo clippy --locked --no-default-features --bin releash-backend -- -D warnings`、`cargo deny --locked check`、`cargo test --locked`、`cargo test --locked --no-default-features --lib`、`cargo test --locked --no-default-features --test daemon_smoke` がそれぞれ実行される
AND 同じ feature と profile の組み合わせのビルドまたはテストが 2 回以上実行されることはない

## B-003: Rust の検証の並列と部分失敗

GIVEN Rust のコードを含む変更の PR
WHEN Rust の検証の一つが失敗する
THEN 他の Rust の検証も最後まで実行され、それぞれの結果が得られる

## B-004: 集約チェック `rust`

GIVEN PR
WHEN PR 層が完了する
THEN `rust` という名前のチェックが結果を持つ
AND Rust の検証がすべて成功したときだけ `rust` は成功する
AND Rust の検証を実行しなかったときも `rust` は成功する

## B-005: PR 層が実行しない検証

GIVEN PR
WHEN PR 層が実行される
THEN `performance` feature を有効にした検証は実行されない
AND coverage の計測は実行されない

## B-006: debuginfo

GIVEN PR 層の Rust のジョブ
WHEN Rust のコードをビルドする
THEN debuginfo は生成されない

## B-007: PR ではキャッシュを保存しない

GIVEN PR で起動した PR 層
WHEN Rust のジョブが終わる
THEN Rust のビルドキャッシュは保存されない

## B-008: main への push でキャッシュを保存する

GIVEN main への push で起動した PR 層
WHEN Rust のジョブが終わる
THEN Rust のビルドキャッシュが保存される
AND キャッシュのキーはジョブごとに分かれている

## B-009: PR の古い run を止める

GIVEN 同じ PR の run が実行中である
WHEN その PR へ続けて push する
THEN 実行中の古い run は中断される
AND 新しい run が起動する

## B-010: main への push の run は止めない

GIVEN main への push で起動した run が実行中である
WHEN main へ続けて push する
THEN 実行中の run は中断されない

## B-011: 文書だけの変更で Rust の検証を実行しない

GIVEN 変更が `docs/**` とリポジトリルート直下の `*.md` だけである
WHEN PR または main への push で PR 層が起動する
THEN どちらの場合も Rust の検証は実行されない
AND `rust` のチェックは成功する

## B-012: facet の Markdown はスキップの対象にしない

GIVEN `workflows/facets/**/*.md` の変更を含む PR
WHEN PR 層が起動する
THEN Rust の検証が実行される

## B-014: nightly が対象にする commit

WHEN `nightly.yml` が起動したとき、起動の理由にかかわらず、検証の対象は main の HEAD である

## B-015: 差分が無い日の日次起動

GIVEN 前回成功した nightly の run が起動した commit と main の HEAD が同じである
WHEN `schedule` で `nightly.yml` が起動する
THEN 検証は実行されない

## B-016: 手動起動は常に実行する

GIVEN 前回成功した nightly の run が起動した commit と main の HEAD が同じである
WHEN `workflow_dispatch` で `nightly.yml` が起動する
THEN 検証が実行される

## B-017: nightly の performance 系の検証

GIVEN nightly の検証が実行される
WHEN `performance` feature を有効にした検証が走る
THEN `performance` feature を有効にした lib テストが絞り込みなしで全件実行される
AND `pnpm test:performance:daemon` が release ビルドで実行される

## B-018: nightly の desktop_cli_install

GIVEN nightly の検証が実行される
WHEN Rust のテストが走る
THEN `cargo test --locked --features performance --test desktop_cli_install` が実行される

## B-019: nightly の coverage

GIVEN nightly の検証が実行される
WHEN coverage の計測が走る
THEN `llvm-profdata: no profile can be merged` で失敗しない
AND TypeScript と Rust の結果が Codecov へ送られる

## B-020: AGENTS.md と CI の一致

`AGENTS.md` の「ビルド・テスト・Lint」に載るコマンドと、変更後の CI が実行するコマンドに差が無い

## B-021: キャンセルされた run はテスト結果のコメントを更新しない

GIVEN 統合テストの結果のコメントが投稿されている PR
WHEN その PR へ続けて push し、実行中の run がキャンセルされる
THEN キャンセルされた run は統合テストの結果のコメントを投稿も更新もしない
AND 以前の run が投稿したコメントがそのまま残る

## B-022: 統合テストが失敗したときの報告

GIVEN PR
WHEN 統合テストが失敗する
THEN 統合テストの結果のコメントは失敗として報告する

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-006 |
| R-007 | B-007, B-008 |
| R-008 | B-009, B-010 |
| R-009 | B-011, B-012 |
| R-011 | B-014 |
| R-012 | B-015, B-016 |
| R-013 | B-017 |
| R-014 | B-018 |
| R-015 | B-019 |
| R-016 | B-020 |
| R-017 | B-021, B-022 |
