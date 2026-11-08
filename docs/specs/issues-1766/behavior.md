## B-001: nightly の版上げ

GIVEN 直前の nightly が `0.4.20` を配り、その後 main に新しい commit が入っている
WHEN nightly が実行される
THEN nightly は版を `0.4.21` にした commit を main に入れ、その commit から `Releash.app` を配る
AND 配った `Releash.app` の `CFBundleShortVersionString` と `CFBundleVersion`、同梱の `releashd` と `releash` の版、appcast の nightly の項目の `sparkle:shortVersionString` と `sparkle:version` は、すべて `0.4.21` である

## B-002: 変更が無い日のスキップ

GIVEN 直前の nightly が版上げの commit を main に入れて配っている
WHEN main の HEAD がその commit のまま、翌日の nightly が実行される
THEN nightly は配らず、新しい版も Release も作らない

## B-003: 同じ commit の再試行

GIVEN nightly `0.4.21` が commit A から配られている
WHEN 同じ実行を再試行する
THEN 版は `0.4.21` のままである

## B-004: stable 公開後の版

GIVEN main の版が `0.4.22` で、nightly `0.4.21` の commit を指定して stable を実行する
WHEN stable が公開される
THEN main の版は `0.4.22` のままで、版上げの PR は作られない

## B-005: nightly の成果物

GIVEN main に新しい commit がある
WHEN nightly が成功する
THEN prerelease の Release には署名・公証済みの universal の `Releash.app` の zip が載る
AND Tauri 版の dmg、`app.tar.gz`、`.sig`、`latest.json` は載らない

## B-006: stable の成果物

GIVEN 公開済みの nightly の tag を指定する
WHEN stable を実行する
THEN その nightly の commit から再ビルド、署名、公証した `Releash.app` の zip が latest の Release に載る

## B-007: アプリの名前

GIVEN 配布された `Releash.app` を起動する
WHEN メニューバーのアイコン、About、終了の項目、tooltip を見る
THEN 表示は `Releash` で、`Releash Preview` はどこにも出ない
AND Release の asset 名にも `Releash Preview` は含まれない

## B-008: appcast の内容

GIVEN stable `0.4.20` と nightly `0.4.21` が公開されている
WHEN `https://siro33950.github.io/releash/appcast.xml` を取得する
THEN 項目は 2 つで、`0.4.20` の項目にはチャンネルが無く、`0.4.21` の項目は `sparkle:channel` が `nightly` である
AND 各項目の enclosure は公開済みの Release の asset を指し、その EdDSA 署名はアプリの公開鍵で検証できる

## B-009: 同時公開

GIVEN nightly と stable が同時に公開を進めている
WHEN 両方が終わる
THEN appcast には stable の項目と nightly の項目の両方がある

## B-010: Release の区分

GIVEN nightly と stable が公開されている
WHEN GitHub Release の一覧を見る
THEN nightly は prerelease、stable は latest である

## B-011: 更新中のサーバと workflow

GIVEN `Releash.app` `0.4.20` で workflow が実行中で、`0.4.21` の更新がダウンロード済みである
WHEN 利用者が更新を適用する
THEN アプリは差し替わって `0.4.21` で再起動し、動いていたサーバに接続する
AND サーバと workflow は止まらずに続く
AND フッターのサーバ表示は `0.4.20` のサーバに対して「再起動で更新できます」を出す

## B-012: 更新設定の初期値

GIVEN `Releash.app` を初めて起動する
WHEN Settings の System タブを開く
THEN 「更新」の節に「自動更新」が ON、「nightly アップデートを受け入れる」が OFF で表示される

## B-013: サーバ未接続での更新設定

GIVEN サーバに接続できず、画面が「サーバに接続できません」を表示している
WHEN 利用者が Settings の「更新」の節を操作する
THEN 「自動更新」と「nightly アップデートを受け入れる」は切り替えられ、再起動後も保持される

## B-014: 自動更新の適用

GIVEN 「自動更新」が ON で、新しい版が appcast にある
WHEN アプリが更新を確認する
THEN アプリは利用者の操作なしに更新をダウンロードする
AND 利用中の画面は終了しない

## B-015: チャンネルと更新候補

GIVEN 現在の版が `0.4.20` で、appcast に stable `0.4.20` と nightly `0.4.21` がある
WHEN 「nightly アップデートを受け入れる」が OFF で確認する
THEN 更新は見つからない

## B-016: 戻さない

GIVEN 現在の版が nightly `0.4.21` で、appcast に stable `0.4.20` と nightly `0.4.21` がある
WHEN 「nightly アップデートを受け入れる」を OFF にして確認する
THEN 更新は見つからず、`0.4.20` に戻らない

## B-017: 手動確認

GIVEN `Releash.app` が起動している
WHEN 利用者がメニューバーの「アップデートを確認」を選ぶ
THEN Sparkle の確認が始まり、結果（新しい版がある、または最新である）が Sparkle の画面で表示される

## B-018: フッターの通知

GIVEN 新しい版 `0.4.21` が見つかっている
WHEN フッターを見る
THEN `0.4.21` と「更新」ボタンが表示される

## B-019: Debug のアプリ

GIVEN `./nx run macos:dev` で Debug のアプリを開く
WHEN appcast に Debug のアプリより新しい版がある
THEN Debug のアプリは appcast を読まず、更新を案内しない

## B-020: 規約の記述

GIVEN `AGENTS.md` を読む
WHEN 「サーバがロジックを所有する」の節を見る
THEN アプリ自身の更新設定（自動更新、nightly の受け入れ）はクライアントが所有し、サーバに置かない、と書かれている

## B-021: 既定のデータフォルダ

GIVEN `--data-dir` も `RELEASH_DATA_DIR` も指定しない
WHEN 本番の `releash status` を実行する
THEN `data_dir` は `~/Library/Application Support/releash` である
AND 開発ビルドでは `~/Library/Application Support/releash.dev` である
AND `docs/guide/cli.md` の既定値の記述もこの 2 つである

## B-022: 旧保存先のサーバへの接続

GIVEN `~/Library/Application Support/com.releash.app` でサーバが動いている
WHEN 新しい `Releash.app` か `releash status` を起動する
THEN そのサーバに接続し、`com.releash.app` のデータはそのままである
AND 新しいサーバは起動しない

## B-023: 停止後の移行

GIVEN `com.releash.app` にデータがあり、サーバは止まっている。`releash` には利用者の定義 `workflows/` と `prompts/` だけがある
WHEN `Releash.app` か `releash server start` でサーバを起動する
THEN `com.releash.app` の設定、event store とその関連ファイル、ログ、その他のファイルが `releash` に移り、サーバは `releash` で起動する
AND `releash/workflows/` と `releash/prompts/` はそのまま残る
AND `releash/bin/releash` の wrapper は `releash` のパスを埋め込んでいる
AND `com.releash.app.dev` は影響を受けない

## B-024: 両方にデータがあるとき

GIVEN `com.releash.app` と `releash` の両方に `local-event-store.sqlite3`（中身あり）がある
WHEN サーバを起動する
THEN 移行は行われず、理由を表示して止まる
AND どちらのデータも変わらない

## B-025: 空のファイルは無視する

GIVEN `com.releash.app` にデータがあり、`releash` には 0 バイトの `local-event-store.sqlite3` と `workflows/` だけがある
WHEN サーバを起動する
THEN 移行が行われ、サーバは `releash` で起動する

## B-026: 並行起動と失敗後の再実行

GIVEN `com.releash.app` にデータがあり、サーバは止まっている
WHEN 2 つの起動が同時に走る、または移行の途中で失敗した後にもう一度起動する
THEN サーバは 1 つだけ起動し、データは失われない

## B-027: 停止を確かめられないとき

GIVEN `com.releash.app` の `client-api.json` が残っていて、そのプロセスの生存を確かめられない
WHEN サーバを起動する
THEN 移行は行われない

## B-028: Finder で開ける

GIVEN 移行が終わっている
WHEN Finder で `~/Library/Application Support/releash` をダブルクリックする
THEN 通常のフォルダとして開く

## B-029: 改名後のログイン項目

GIVEN 「ログイン時に起動」が ON で、`Releash Preview.app` を `Releash.app` に置き換えた
WHEN `Releash.app` を一度起動し、その後ログインし直す
THEN ログイン時に `Releash.app` が起動する

## B-030: 今すぐ適用

GIVEN 「自動更新」が ON で、新しい版のダウンロードが終わっている
WHEN 利用者が Sparkle の画面で今すぐ適用を選ぶ
THEN アプリはその場で差し替わり、新しい版で再起動する

## B-031: CI 未成功の commit

GIVEN main の HEAD に成功した main push の CI が無い
WHEN nightly が実行される
THEN nightly は失敗し、版上げも Release も行わない

## B-032: 回帰 E2E の失敗

GIVEN main に新しい commit があり、回帰 E2E が失敗する
WHEN nightly が実行される
THEN Release は作られず、ISSUE が作られる

## B-033: stable への昇格の版

GIVEN nightly `0.4.21` が commit A から配られている
WHEN commit A の nightly を指定して stable を実行する
THEN 版は `0.4.21` のままである
AND stable の `Releash.app`、同梱の `releashd` と `releash`、appcast の stable の項目はすべて `0.4.21` である

## B-034: stable の成果物の欠落

GIVEN Swift の asset か appcast がそろっていない
WHEN stable を実行する
THEN Release は公開されない

## B-035: 昇格後の appcast

GIVEN stable `0.4.20` と nightly `0.4.21` が公開されている
WHEN `0.4.21` を stable に昇格する
THEN appcast の `0.4.21` の項目は stable の 1 つだけになる

## B-036: appcast の公開の失敗と再実行

GIVEN appcast の公開に失敗した workflow がある
WHEN その workflow を再実行する
THEN 失敗した実行は成功にならず、再実行で同じ版の公開が完了する

## B-037: nightly の Release の削除

GIVEN nightly の Release が 14 件を超えている
WHEN 直近 14 件に入らない Release を消す
THEN appcast は消した asset を指さない

## B-038: 更新設定の保持

GIVEN 利用者が更新設定の値を変えている
WHEN アプリを再起動する
THEN 変えた値のままである

## B-039: 終了時の適用

GIVEN 自動ダウンロードが終わった更新がある
WHEN 利用者がアプリを終了する
THEN 更新が適用される

## B-040: nightly の受け入れが ON

GIVEN 現在の版が `0.4.20` で、appcast に stable `0.4.20` と nightly `0.4.21` がある
WHEN 「nightly アップデートを受け入れる」が ON で確認する
THEN `0.4.21` が見つかる

## B-041: 同じ版の昇格

GIVEN 現在の版が nightly `0.4.21` である
WHEN `0.4.21` が stable に昇格した appcast で確認する
THEN 同じ版の再インストールは案内されない

## B-042: 新しい stable

GIVEN 現在の版が nightly `0.4.21` で、「nightly アップデートを受け入れる」が OFF である
WHEN stable `0.4.22` が公開された appcast で確認する
THEN `0.4.22` が見つかる

## B-043: 確認中のメニュー

GIVEN Sparkle がバックグラウンドで更新を確認している、またはダウンロードしている
WHEN メニューバーを開く
THEN 「アップデートを確認」は無効である

## B-044: フッターのボタン

GIVEN 新しい版 `0.4.21` が見つかっていて、自動ダウンロードの途中でない
WHEN フッターの「更新」を押す
THEN Sparkle の標準の更新画面が開き、版だけが表示され、変更内容やリンクは表示されない

## B-045: 自動ダウンロード中のフッターのボタン

GIVEN 新しい版 `0.4.21` を自動でダウンロードしている
WHEN フッターを見る
THEN 「更新」は無効である

## B-046: 手動確認の画面を表示している間のメニュー

GIVEN Sparkle の手動確認の画面を表示している
WHEN メニューバーの「アップデートを確認」を選ぶ
THEN Sparkle の画面が前面に出る

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-001, B-002 |
| R-003 | B-003, B-033 |
| R-004 | B-004 |
| R-005 | B-005, B-031, B-032 |
| R-006 | B-006, B-034 |
| R-007 | B-007 |
| R-008 | B-008, B-035 |
| R-009 | B-009, B-036, B-037 |
| R-010 | B-010 |
| R-011 | B-011 |
| R-012 | B-012, B-013, B-038 |
| R-013 | B-014, B-030, B-039 |
| R-014 | B-015, B-016, B-040, B-041, B-042 |
| R-015 | B-017, B-043, B-046 |
| R-016 | B-018, B-044, B-045 |
| R-017 | B-019 |
| R-018 | B-020 |
| R-019 | B-021 |
| R-020 | B-022 |
| R-021 | B-023 |
| R-022 | B-024, B-025 |
| R-023 | B-026, B-027 |
| R-024 | B-028 |
| R-025 | B-029 |
