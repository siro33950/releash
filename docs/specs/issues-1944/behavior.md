## B-001: provider の欄に使用率を出す

GIVEN サーバが provider の使用率を受け取っている
WHEN 開発者がフッターを見る
THEN フッターの左に、その provider の欄が出る
AND 欄には provider のアイコン、使用率のバー、窓ごとの使用率と窓の長さが出る

## B-002: クリックで reset までの時間と plan を出す

GIVEN フッターに provider の欄が出ていて、サーバがその provider の plan を読めている
WHEN 開発者がその欄をクリックする
THEN 窓ごとの reset までの時間と、その provider の plan が出る

## B-003: plan を読めないときも表示を続ける

GIVEN フッターに provider の欄が出ていて、サーバがその provider の plan を読めない
WHEN 開発者がその欄をクリックする
THEN 窓ごとの reset までの時間が出る
AND plan は出ない

## B-004: 80% を超えた窓があると警告する

GIVEN サーバが受け取った provider の使用率のうち、いずれかの窓が 80% を超えている
WHEN 開発者がフッターを見る
THEN その provider の欄に警告が出る

## B-005: hook の警告を provider の欄に出す

GIVEN サーバが provider の hook の警告を持っている
WHEN 開発者がフッターを見る
THEN その provider の欄に hook の警告が出る

## B-006: provider に属さない失敗を一つの警告にまとめる

GIVEN hook の状態の記録を読めなかったか、hook の警告の購読に失敗している
WHEN 開発者がフッターを見る
THEN provider の欄とは別に、フッターに一つの警告が出る

## B-007: 値の無い provider の欄は出さない

GIVEN ある provider について、サーバが使用率も hook の警告も持っていない
WHEN 開発者がフッターを見る
THEN その provider の欄は出ない

## B-008: Claude の使用率を反映する

GIVEN managed settings が statusLine を定めていない環境で、Releash が起動した Claude の Session が動いている
WHEN Claude が statusLine に使用率と reset を渡す
THEN フッターの Claude の欄に、その使用率と reset が出る

## B-009: 利用者の statusLine を引き継ぐ

GIVEN 利用者が Claude の statusLine を設定している
WHEN Releash が Claude の Session を起動する
THEN その Session の Claude の画面に、利用者の statusLine の出力が出る
AND フッターの Claude の欄に使用率が出る

## B-010: Claude の plan を出す

GIVEN `~/.claude.json` の `oauthAccount` に plan の値がある
WHEN 開発者がフッターの Claude の欄をクリックする
THEN その値に基づく plan が出る

## B-011: Codex の使用率と plan を反映する

GIVEN Releash が起動した Codex の Session が動いている
WHEN その Session の rollout に `rate_limits` を持つ `token_count` が追記される
THEN フッターの Codex の欄に、最新の `token_count` の使用率と reset が出る
AND Codex の欄をクリックすると、その `plan_type` に基づく plan が出る

## B-012: 最後に受け取った値を出す

GIVEN ある provider の使用率を、サーバが二度受け取った
WHEN 開発者がフッターを見る
THEN その provider の欄には、後から受け取った値が出る

## B-013: 再起動の後は次の値まで使用率を出さない

GIVEN サーバが provider の使用率を受け取った後に再起動した
WHEN 再起動の後、その provider の使用率をまだ受け取っていない
THEN フッターにその provider の使用率は出ない

## B-014: サーバの状態を出す

GIVEN 画面がサーバに接続している
WHEN 開発者がフッターを見る
THEN フッターの右に、サーバの版、互換、serving status、起動からの時間が出る

## B-015: CLI が版の違いを返す

GIVEN 動いているサーバと CLI の protocol が同じで、版が違う
WHEN `releash status --json` を実行する
THEN 出力に、サーバとクライアントのどちらが古いかが入る

## B-016: サーバが古いときに再起動を案内する

GIVEN 画面がサーバに接続していて、CLI がサーバのほうが古いと判定している
WHEN 開発者がフッターを見る
THEN フッターに、再起動で更新できることが出る

## B-017: クライアントが古いことを出す

GIVEN 画面がサーバに接続していて、CLI がクライアントのほうが古いと判定している
WHEN 開発者がフッターを見る
THEN フッターに、クライアントが古いことが出る

## B-018: サーバを停止する

GIVEN 画面がサーバに接続している
WHEN 開発者がフッターの「サーバを停止」を押す
THEN サーバが止まる
AND 画面はつなぎ直しをせず、接続失敗の画面に理由「サーバは停止しています」を出す

## B-019: サーバを再起動する

GIVEN 画面がサーバに接続している
WHEN 開発者がフッターの「サーバを再起動」を押す
THEN 同梱した CLI の `releash server restart` でサーバが再起動する
AND 画面は新しいサーバに接続し直す

## B-020: フッターの回帰の E2E

GIVEN リポジトリの回帰の E2E
WHEN `./nx run macos:e2e` を実行する
THEN フッターに provider の使用率とサーバの状態が表示されることを確かめるテストが実行され、成功する

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002, B-003 |
| R-003 | B-004 |
| R-004 | B-005 |
| R-005 | B-006 |
| R-006 | B-007 |
| R-007 | B-008 |
| R-008 | B-009 |
| R-009 | B-010 |
| R-010 | B-011 |
| R-011 | B-012, B-013 |
| R-012 | B-014 |
| R-013 | B-015 |
| R-014 | B-016, B-017 |
| R-015 | B-018 |
| R-016 | B-019 |
| R-017 | B-020 |
