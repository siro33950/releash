## B-001: 画面がサーバを起動して接続する

GIVEN サーバが動いていない
WHEN 利用者が新しい配置からビルドした Releash.app を開く
THEN 画面はサーバを起動して接続し、今と同じ画面を表示する

## B-002: 画面が動いているサーバに接続する

GIVEN サーバが動いている
WHEN 利用者が新しい配置からビルドした Releash.app を開く
THEN 画面はそのサーバに接続し、新しいサーバを起動しない

## B-003: CLI でサーバを起動・確認・停止する

GIVEN サーバが動いていない
WHEN 利用者が新しい配置からビルドした CLI で `releash server start`、`releash status`、`releash server stop` を順に実行する
THEN サーバが起動し、`releash status` は動いているサーバの状態を表示し、`releash server stop` の後にサーバは終了する

## B-004: hook がサーバに信号を届ける

GIVEN 新しい配置からビルドしたサーバが動いており、Session の provider が起動している
WHEN provider の hook が CLI を通して信号を送る
THEN サーバは今と同じにその信号を受け取り、Session の状態に反映する

## B-005: client-api.json の識別子のキー

GIVEN サーバが動いていない
WHEN サーバが起動する
THEN `client-api.json` は識別子をキー `daemon_id` で持ち、その値は `ServerInfo` の `daemon_id` と同じである
AND `client-api.json` はキー `instance_id` を持たない

## B-006: 古いサーバが書いた client-api.json

GIVEN 動いているサーバの `client-api.json` が、識別子をキー `instance_id` で持ち、キー `daemon_id` を持たない
WHEN 利用者が CLI の `releash status` を実行する、または Releash.app を開く
THEN CLI と画面は `instance_id` の値をサーバの識別子として照合し、そのサーバに接続する

## B-007: data dir の選び方

GIVEN `RELEASH_DATA_DIR` を設定していない
WHEN development ビルドのサーバ・CLI・画面を起動する
THEN どれも `com.releash.app.dev` の data dir を使う

## B-008: data dir の指定

GIVEN `RELEASH_DATA_DIR` に data dir を設定している
WHEN 利用者が CLI で `releash server start` を実行し、続けて `--data-dir` を付けずに `releash status` を実行する
THEN サーバは `RELEASH_DATA_DIR` の data dir に `client-api.json` を書き、`releash status` はそのサーバの状態を表示する

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-003, B-004 |
| R-003 | B-005 |
| R-004 | B-006 |
| R-005 | B-007, B-008 |
| R-006 | なし（CI で確認） |
