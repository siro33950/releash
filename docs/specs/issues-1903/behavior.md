## B-001: サーバの名前

GIVEN Releash をビルドして `.app` を作る
WHEN 作られたサーバの実行ファイルと `.app` の同梱物を見る
THEN サーバの実行ファイルは `releashd` で、`.app` には `releashd` と `releash` が同梱される
AND リポジトリのビルド、CI、テスト、文書に `releash-backend` の名前は無い

## B-002: 引数なしでの単独起動

GIVEN 同じ data dir でサーバが動いていない
WHEN `releashd` を引数なしで起動する
THEN `releashd` は終了せず、data dir に発見ファイル `client-api.json` を書いてサーバとして動き続ける

## B-003: data dir の決め方

GIVEN `RELEASH_DATA_DIR` が設定されている
WHEN `releashd` を `--data-dir` なしで起動する
THEN サーバは `RELEASH_DATA_DIR` の data dir を使う
AND `--data-dir` を指定して起動したときは `RELEASH_DATA_DIR` ではなく `--data-dir` の data dir を使う
AND どちらも無いときは CLI と同じ既定の data dir を使う

## B-004: 多重起動の拒否

GIVEN 同じ data dir でサーバが動いている
WHEN 2 つ目の `releashd` を起動する
THEN 2 つ目は理由を stderr に出して終了コード 1 で終わる
AND 動いているサーバは動き続ける

## B-005: 単独起動したサーバへの CLI の接続

GIVEN `releashd` を引数なしで起動し、サーバが動いている
WHEN 同じ data dir で CLI のコマンド（例: `releash workflow diagnostics`）を実行する
THEN CLI はそのサーバに接続し、コマンドの結果を返す

## B-006: Tauri アプリの動作の維持

GIVEN Releash の Tauri アプリを起動する
WHEN アプリを使い、再起動・更新・Quit を行う
THEN アプリは今と同じにサーバを子プロセスとして起動して接続し、再起動・更新・Quit で今と同じにサーバを止める

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-006 |
