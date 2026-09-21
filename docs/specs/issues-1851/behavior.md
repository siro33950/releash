## B-001: 関門が緑のとき nightly の prerelease が上がる

GIVEN nightly が main の HEAD を対象に起動している
WHEN `performance` と PR 層と同じ検証一式がすべて成功する
THEN その commit を指す `v{X.Y.Z}-nightly.{YYYYMMDD}.{N}` 形式のタグを持つ prerelease の GitHub Release が作られる
AND その Release は署名・公証済みの macOS universal ビルドを asset に持つ
AND そのビルドが名乗る版は 3 つの整数からなり、プレリリース識別子を含まない
AND そのビルドには Tauri updater の署名と telemetry の送信先が注入されている

## B-002: 関門が赤のとき nightly の Release が上がらない

GIVEN nightly が main の HEAD を対象に起動している
WHEN `performance` と PR 層と同じ検証一式のいずれかが失敗する
THEN nightly の Release は作られない

## B-003: main に差分が無い日の schedule 起動はスキップされる

GIVEN 直近の nightly の Release のタグが指す commit と main の HEAD が同じ
WHEN nightly が `schedule` で起動する
THEN 検証、ビルド、Release 作成のいずれも行われない

## B-004: main に差分がある日の schedule 起動は実行される

GIVEN 直近の nightly の Release のタグが指す commit と main の HEAD が異なる
WHEN nightly が `schedule` で起動する
THEN main の HEAD を対象に関門の検証が実行される

## B-005: 手動起動は差分の有無によらず実行される

GIVEN 直近の nightly の Release のタグが指す commit と main の HEAD が同じ
WHEN nightly が `workflow_dispatch` で起動する
THEN main の HEAD を対象に関門の検証が実行される
AND 関門がすべて成功すれば prerelease の Release が作られる

## B-006: 古い nightly の Release は残らない

GIVEN nightly の Release が 14 件ある
WHEN 新しい nightly の Release が作られる
THEN nightly の Release は新しいものから 14 件だけが残る
AND それより古い nightly の Release は取得できない

## B-007: nightly を指定して stable の Release を作れる

GIVEN nightly の Release のタグが存在する
WHEN そのタグを入力として stable を `workflow_dispatch` で起動する
THEN そのタグが指す commit からビルド・署名・公証をやり直した asset を持つ Release が作られる
AND その Release のタグ名は `v` + その commit のリポジトリに書かれた版である
AND その Release は prerelease ではない
AND そのビルドには Tauri updater の署名と telemetry の送信先が注入されている

## B-008: 既存利用者は stable へ更新できる

GIVEN 一つ前の stable を使っている利用者がいる
WHEN 新しい stable の Release が公開される
THEN 利用者は `plugins.updater.endpoints` を変えずに Tauri updater で新しい stable へ更新できる
AND nightly の prerelease は Tauri updater の更新対象にならない

## B-009: stable の後に patch を上げる PR ができる

GIVEN stable の Release が作られた
WHEN stable の workflow が完了する
THEN `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` の版を、patch を 1 つ上げた版へ揃えて更新する PR が存在する

## B-010: 旧リリース経路が動かない

WHEN `v*` 形式のタグが push されたとき、Release は作られない

## B-011: main への版変更の push でタグが作られない

WHEN `package.json` の版を変更した commit が main へ push されたとき、タグは作られない

## B-012: AGENTS.md がリリースと版の扱いを示す

GIVEN `AGENTS.md` を読む
WHEN 変更後のリリース手順と版を上げるコミットの扱いを確認する
THEN 「リリース」節に nightly の起動とスキップ、stable の起動と入力、版を上げる PR、`Bump Version` による版の変更を含む流れが記述されている
AND 「コミット・PR」節の `release: vX.Y.Z` が、リリースではなく版を上げるコミットを表すものとして記述されている

## B-013: coverage の失敗は nightly の Release を止めない

GIVEN nightly が main の HEAD を対象に起動している
WHEN 関門がすべて成功し、`coverage` ジョブが失敗する
THEN prerelease の GitHub Release が作られる

## B-014: 上げ幅を指定して版を変えられる

GIVEN main の版が `X.Y.Z` である
WHEN `Bump Version` を上げ幅を指定して `workflow_dispatch` で起動する
THEN 指定した上げ幅で計算した版へ、`package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` の 4 箇所を揃えて更新する PR が存在する

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002, B-013 |
| R-002 | B-003, B-004 |
| R-003 | B-005 |
| R-004 | B-006 |
| R-005 | B-007 |
| R-006 | B-007, B-008 |
| R-007 | B-009 |
| R-008 | B-001, B-007 |
| R-009 | B-010, B-011, B-014 |
| R-010 | B-012 |
| R-011 | B-001, B-003 |
