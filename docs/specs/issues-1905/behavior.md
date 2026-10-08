## B-001: 動いているサーバの状態の表示

GIVEN 同じ data dir で、CLI と互換のあるサーバが動いている
WHEN `releash status` を実行する
THEN クライアントの release と protocol、data dir、サーバが動いていること、サーバの daemon_id・pid・起動時刻・release・protocol・capability・serving status・起動からの時間、互換ありの判定が表示される
AND サーバの daemon_id・pid・起動時刻・release・protocol・capability・serving status は、同じサーバの `GetServerInfo` の応答と一致する
AND 終了コードは 0 である

## B-002: サーバが動いていないときの状態の表示

GIVEN 同じ data dir でサーバが動いていない
WHEN `releash status` を実行する
THEN クライアントの release と protocol、data dir と、サーバが動いていないことが表示される
AND 終了コードは 0 である

## B-003: 互換の無いサーバの状態の表示

GIVEN 同じ data dir で、CLI より古い protocol のサーバが動いている
WHEN `releash status` を実行する
THEN サーバの情報と、サーバが古いという判定と、`releash server restart` で更新できるという案内が表示される
AND 終了コードは 0 である

## B-004: クライアントが古いときの状態の表示

GIVEN 同じ data dir で、CLI より新しい protocol のサーバが動いている
WHEN `releash status` を実行する
THEN サーバの情報と、クライアントが古いという判定と、クライアントの更新の案内が表示される
AND 終了コードは 0 である

## B-005: 状態の JSON

GIVEN 同じ data dir でサーバが動いている
WHEN `releash status --json` を実行する
THEN stdout に、B-001 の項目と、接続先の host と port、発見ファイルのパスを含む JSON が出る
AND JSON に token の値は含まれない

## B-006: サーバの起動

GIVEN 同じ data dir でサーバが動いていない
WHEN `releash server start` を実行する
THEN CLI と同じディレクトリの `releashd` が起動し、発見ファイルが書かれてから CLI が終了コード 0 で終わる
AND CLI が終わった後も、起動したサーバは動き続ける

## B-007: 動いているときの起動

GIVEN 同じ data dir でサーバが動いている
WHEN `releash server start` を実行する
THEN 新しいサーバは起動されず、動いていたサーバのプロセスはそのまま動き続ける
AND 終了コードは 0 である

## B-008: 起動の失敗

GIVEN 同じ data dir でサーバが動いておらず、起動した `releashd` が stderr に理由を書いて終了する
WHEN `releash server start` を実行する
THEN プロセスが終了したことと stderr の末尾が表示される
AND 終了コードは 1 である

## B-009: サーバの停止

GIVEN 同じ data dir でサーバが動いている
WHEN `releash server stop` を実行する
THEN サーバのプロセスは終了し、発見ファイルは消える
AND CLI はプロセスの終了と発見ファイルの消失の後に、終了コード 0 で終わる

## B-010: 停止を確かめられないとき

GIVEN 同じ data dir でサーバが動いていて、停止の要求の後も `shutdown_timeout_ms` の間に終了しない
WHEN `releash server stop` を実行する
THEN 終了コードは 1 である

## B-011: 動いていないときの停止

GIVEN 同じ data dir でサーバが動いていない
WHEN `releash server stop` を実行する
THEN 終了コードは 1 である

## B-012: 動いているときの再起動

GIVEN 同じ data dir でサーバが動いている
WHEN `releash server restart` を実行する
THEN 動いていたサーバのプロセスは終了し、CLI と同じディレクトリの `releashd` の新しいプロセスがサーバとして動いている
AND 出力は、停止してから起動したことを示す
AND 終了コードは 0 である

## B-013: 動いていないときの再起動

GIVEN 同じ data dir でサーバが動いていない
WHEN `releash server restart` を実行する
THEN CLI と同じディレクトリの `releashd` がサーバとして動いている
AND 出力は、動いていなかったので起動したことを示す
AND 終了コードは 0 である

## B-014: 引数なしの実行（画面がある）

GIVEN CLI の実行ファイルの実体が `.app` の中にあり、CLI が解決した data dir が画面の既定の data dir と同じで、その data dir でサーバが動いていない
WHEN `releash` を引数なしで実行する
THEN 同じディレクトリの `releashd` がサーバとして起動し、その `.app` が開く

## B-015: 引数なしの実行（画面が無い）

GIVEN CLI の実行ファイルの実体が `.app` の中に無く、同じ data dir でサーバが動いていない
WHEN `releash` を引数なしで実行する
THEN 同じディレクトリの `releashd` がサーバとして起動し、`releash status` と同じ表示が出る

## B-024: 引数なしの実行（data dir が画面の既定と違う）

GIVEN CLI の実行ファイルの実体が `.app` の中にあり、CLI が解決した data dir が画面の既定の data dir と違い、その data dir でサーバが動いていない
WHEN `releash` を引数なしで実行する
THEN 同じディレクトリの `releashd` が、CLI が解決した data dir のサーバとして起動する
AND `.app` は開かれず、data dir が画面の既定と違うために開かなかったことと、`releash status` と同じ表示が出る

## B-016: サーバの状態の購読

GIVEN サーバが動いている
WHEN client がサーバの状態を購読する
THEN 最初の状態として、`GetServerInfo` の応答と同じ項目と値（serving status は Serving）が届く

## B-017: 停止に入ったときの購読

GIVEN client がサーバの状態を購読している
WHEN サーバが停止の要求を受理する
THEN client に、serving status が Stopping の状態が届く

## B-018: サーバが古いときの既存コマンド

GIVEN 同じ data dir で、CLI より古い protocol のサーバが動いている
WHEN workflow または review のコマンドを実行する
THEN `failed_precondition` で失敗し、`releash server restart` で更新できるという案内が出る
AND 終了コードは 1 である

## B-019: クライアントが古いときの既存コマンド

GIVEN 同じ data dir で、CLI より新しい protocol のサーバが動いている
WHEN workflow または review のコマンドを実行する
THEN `failed_precondition` で失敗し、クライアントの更新の案内が出る
AND 終了コードは 1 である

## B-020: サーバが RPC を知らないとき

GIVEN 同じ data dir で、CLI のコマンドが呼ぶ RPC を持たないサーバが動いている
WHEN そのコマンドを実行し、サーバが `unimplemented` を返す
THEN `releash server restart` で更新できるという案内が出る

## B-021: 古い発見ファイル

GIVEN data dir に、終了したサーバの発見ファイルが残っている
WHEN workflow または review のコマンドを実行する
THEN サーバが動いていないときと同じく `unavailable` で失敗し、終了コードは 1 である
AND `releash status` は、サーバが動いていないと表示する

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002, B-003, B-004 |
| R-003 | B-005 |
| R-004 | B-006, B-007, B-008 |
| R-005 | B-009, B-010, B-011 |
| R-006 | B-012, B-013 |
| R-007 | B-014, B-015, B-024 |
| R-008 | B-016, B-017 |
| R-009 | B-003, B-004, B-018, B-019 |
| R-010 | B-020 |
| R-011 | B-021 |
| R-012 | なし（レビューで確認） |
| R-013 | なし（レビューで確認） |
