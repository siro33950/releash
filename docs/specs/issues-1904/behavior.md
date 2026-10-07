## B-001: 動いているサーバへの接続

GIVEN 同じ data dir で、画面と互換のあるサーバが動いている
WHEN 画面を起動する
THEN 画面はそのサーバに接続して通常の窓を出す
AND サーバのプロセスは画面の起動の前と同じである

## B-002: サーバが居ないときの起動

GIVEN 同じ data dir でサーバが動いていない
WHEN 画面を起動する
THEN 画面は同じディレクトリの `releashd` を起動し、そのサーバに接続して通常の窓を出す
AND 起動したサーバは画面と別の session で動く

## B-003: サーバを起動できなかったときの表示

GIVEN 同じ data dir でサーバが動いておらず、起動した `releashd` が stderr に理由を書いて終了する
WHEN 画面を起動する
THEN 画面は失敗の窓に、プロセスが終了したことと stderr の末尾を表示する

## B-004: サーバが古いとき

GIVEN 同じ data dir で、画面より古い protocol のサーバが動いている
WHEN 画面を起動する
THEN 画面は接続せず、失敗の窓に「サーバが古い」ことと、サーバと画面の release を表示する
AND 窓には、サーバを停止して起動し直す操作がある

## B-005: 古いサーバの入れ替え

GIVEN B-004 の失敗の窓が出ている
WHEN 利用者が、サーバを停止して起動し直す操作を行う
THEN 古いサーバは終了し、画面は同じディレクトリの `releashd` を起動して接続し、通常の窓を出す

## B-006: 画面が古いとき

GIVEN 同じ data dir で、画面より新しい protocol のサーバが動いている
WHEN 画面を起動する
THEN 画面は接続せず、失敗の窓に「画面が古い」ことと、サーバと画面の release を表示する

## B-007: Quit は画面だけを閉じる

GIVEN 画面がサーバに接続していて、agent の Session が動いている
WHEN 利用者がトレイの Quit、OS の終了操作（メニュー・Cmd+Q）、または失敗の窓の Quit で画面を終了する
THEN 画面のプロセスは終了する
AND サーバのプロセスと agent の Session は動き続け、CLI からそのサーバに接続できる

## B-008: サーバを停止

GIVEN 画面がサーバに接続している
WHEN 利用者がトレイの「サーバを停止」を選ぶ
THEN 画面は、動いている agent の Session も止まることを示して確認を求める
AND 利用者が確認すると、サーバのプロセスは終了コード 0 で終了し、発見ファイル `client-api.json` は消える
AND 利用者が確認しなければ、サーバは動き続ける

## B-009: サーバが居なくなったとき

GIVEN 画面がサーバに接続している
WHEN サーバのプロセスが終了する
THEN 画面は「動いていない」ことと、サーバを起動する操作を表示する
AND 画面は自分から新しいサーバを起動しない

## B-010: 画面からの起動

GIVEN B-009 の「動いていない」表示が出ている
WHEN 利用者がサーバを起動する操作を行う
THEN 画面は同じディレクトリの `releashd` を起動して接続し、通常の表示に戻る

## B-011: StopDaemon

GIVEN サーバが動いている
WHEN operator の token で `StopDaemon` を呼ぶ
THEN 呼び出しは受理され、サーバのプロセスは終了コード 0 で終了し、発見ファイル `client-api.json` は消える

## B-012: proto の reserved

GIVEN `proto/client.proto`
WHEN 停止の RPC と `ServerInfo` を見る
THEN `RequestApplicationQuit` は無く、その oneof の番号と名前は reserved である
AND `ServerInfo` に `launch_id` は無く、その番号と名前は reserved である

## B-013: 更新でサーバを止めない

GIVEN 画面がサーバに接続していて、画面の更新がある
WHEN 利用者が更新をインストールする
THEN 画面は新しい版で起動し直す
AND 更新の前から動いていたサーバのプロセスは動き続け、起動し直した画面はそのサーバに接続する

## B-014: サーバの設定に従う

GIVEN サーバの設定で close_to_tray・`start_minimized`・crash reporting・performance telemetry・ログイン項目の希望が決まっている
WHEN 画面がサーバに接続する
THEN 画面は窓を閉じたときの動き、ログイン項目からの起動での窓の表示、telemetry、ログイン項目の登録をその値に従わせる
AND サーバの設定が変わると、画面は変わった値に従う

## B-015: サーバの設定ファイルを読まない

GIVEN サーバが動いておらず、data dir に `releash.toml` がある
WHEN 画面をログイン項目から（`--hidden` で）起動し、サーバを起動できない
THEN 画面は `releash.toml` の値を使わず、ログイン項目の登録を変えない
AND 画面は失敗の窓を出す

## B-016: 親プロセスとの pipe での終了が無い

GIVEN `releashd` を起動する
WHEN サーバの stdin を閉じる、または `RELEASH_DAEMON_PARENT_PIPE` を設定して起動する
THEN サーバは動き続ける

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004, B-005, B-006 |
| R-005 | B-007 |
| R-006 | B-008 |
| R-007 | B-009, B-010 |
| R-008 | B-011, B-012 |
| R-009 | B-013 |
| R-010 | B-014, B-015 |
| R-011 | B-016 |
