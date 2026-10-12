## B-001: 回帰の E2E を全部回す

GIVEN リポジトリに回帰の E2E がある
WHEN 開発者が `./nx run macos:e2e` を実行する
THEN `macos:generate` と `macos:helpers` が先に実行される
AND リポジトリにある回帰の E2E がすべて実行される
AND すべて通れば成功、一つでも落ちれば失敗で終わる

## B-002: 隔離した環境で本番と同じ経路を通る

GIVEN 回帰の E2E が実行されている
WHEN テストがアプリを起動する
THEN アプリは、そのテスト用に隔離したデータディレクトリを使い、同梱の CLI でサーバを探して接続する
AND 本番のデータディレクトリと、そこを使うサーバに読み書きしない

## B-003: 本番の Releash を動かしたまま回す

GIVEN 本番の Releash のアプリとサーバが動いている
WHEN 開発者が `./nx run macos:e2e` を実行する
THEN 回帰の E2E が実行される
AND 実行中も実行後も、本番のアプリとサーバは動き続けている

## B-004: build configuration ごとの bundle ID

GIVEN macOS のアプリをビルドする
WHEN Debug でビルドする
THEN アプリの bundle ID は `com.releash.macos.dev` である

## B-005: Release の bundle ID

GIVEN macOS のアプリをビルドする
WHEN Release でビルドする
THEN アプリの bundle ID は `com.releash.macos` である

## B-006: テストごとに状態を用意する

GIVEN 回帰の E2E が、repository、worktree、実行中の workflow、Thread のうち必要なものを宣言している
WHEN そのテストが始まる
THEN 宣言した repository、worktree、実行中の workflow、Thread が、そのテストの隔離したデータディレクトリのサーバに存在する状態で、アプリの操作を始められる

## B-007: provider の代わりの TUI を E2E で起動する

GIVEN 隔離したデータディレクトリの `releash.toml` の `agents.claude.cli_path` に、provider の代わりの TUI の実行ファイルが指定されている
WHEN 回帰の E2E が Claude の Session を起動する
THEN サーバはその実行ファイルを provider として起動する

## B-008: provider の代わりの TUI の実行ファイル

GIVEN サーバのクレートをビルドする
WHEN `test-support` feature を付けてビルドする
THEN provider の代わりの TUI の実行ファイルが作られる
AND `test-support` feature を付けないビルドでは、この実行ファイルは作られない

## B-009: サーバの統合テストが provider の代わりの TUI を使う

GIVEN provider の代わりの TUI を使うサーバの統合テストがある
WHEN そのテストが provider を起動する
THEN 起動されるのは、provider の代わりの TUI の実行ファイルである

## B-010: サーバが居ない状態から起動して接続する

GIVEN 隔離したデータディレクトリでサーバが動いていない
WHEN アプリを起動する
THEN サーバが起動する
AND アプリはサーバに接続し、サイドバーを表示する

## B-011: repository ごとの worktree のカード

GIVEN repository が登録され、その repository に worktree がある
WHEN アプリがサーバに接続する
THEN サイドバーに、その repository のまとまりの中に、worktree のカードが表示される

## B-012: 2 つの pane とタブの移動

GIVEN worktree を選び、中央にタブが開いている
WHEN 中央を 2 つの pane に分割し、タブをもう一方の pane へ移す
THEN 中央に 2 つの pane が表示される
AND 移したタブは、移した先の pane に表示される

## B-013: 作成ダイアログから作成と起動

GIVEN 作成ダイアログで、repository、新しい branch、作成後に起動するものを指定している
WHEN 「作成」を押す
THEN 新しい worktree のカードがサイドバーに表示される
AND 指定したものが、その worktree で起動している

## B-014: 証跡を PR に載せる

GIVEN 今のブランチに PR がある
WHEN 開発者がテストを指定して `./nx run macos:e2e-evidence` を実行する
THEN 今の commit でビルドされたアプリに対して、指定したテストが実行される
AND 今のブランチの PR に、commit の ID、回したテストのソース、各テストの成否を含むコメントが 1 つ載る

## B-015: 落ちたテストも証跡に載る

GIVEN 今のブランチに PR があり、指定したテストの一つが落ちる
WHEN 開発者が `./nx run macos:e2e-evidence` を実行する
THEN PR のコメントに、そのテストが落ちたことが載る

## B-016: 回帰の E2E を全部回した結果を証跡に載せる

GIVEN 今のブランチに PR がある
WHEN 開発者が、回帰の E2E を全部回す指定で `./nx run macos:e2e-evidence` を実行する
THEN 回帰の E2E がすべて実行される
AND PR のコメントに、commit の ID、回したテストのソース、各テストの成否が載る

## B-017: 証跡は同時に 1 つだけ回る

GIVEN 同じ Mac で `macos:e2e-evidence` が実行中である
WHEN 開発者が `./nx run macos:e2e-evidence` をもう一度実行する
THEN 後から呼んだほうのテストは、先の実行が終わってから始まる

## B-018: PR が無いときの証跡

GIVEN 今のブランチに PR が無い
WHEN 開発者が `./nx run macos:e2e-evidence` を実行する
THEN 失敗で終わる
AND どの PR にもコメントは載らない

## B-019: nightly は macos の e2e を回さない

GIVEN nightly が `nx run-many -t e2e coverage` を実行する
WHEN 実行の対象のプロジェクトが決まる
THEN macos の `e2e` は対象に含まれない

## B-020: TEST.md の macOS の行

GIVEN 開発者が `docs/architecture/TEST.md` を読む
WHEN 「種類」の表と「配置と実行」の表を見る
THEN 「種類」の表に振る舞い（macOS）の行がある
AND 「配置と実行」の表に、単体（macOS）と振る舞い（macOS）の行があり、それぞれの置き場所、実行、CI が書かれている

## B-021: AGENTS.md の E2E と証跡

GIVEN 開発者が `AGENTS.md` の「ビルド・テスト・Lint」を読む
WHEN macOS の E2E と証跡の扱いを探す
THEN `macos:e2e` と `macos:e2e-evidence` の呼び方が書かれている
AND 証跡の決まりが、マイルストーン #78 の「テストと CI の方針」と同じ内容で書かれている

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004, B-005 |
| R-005 | B-006 |
| R-006 | B-007, B-008, B-009 |
| R-007 | B-010 |
| R-008 | B-011 |
| R-009 | B-012 |
| R-010 | B-013 |
| R-011 | B-014, B-015 |
| R-012 | B-016 |
| R-013 | B-017 |
| R-014 | B-018 |
| R-015 | B-019 |
| R-016 | B-020 |
| R-017 | B-021 |
