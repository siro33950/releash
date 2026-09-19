## B-001: クライアントからの RPC 呼び出し

GIVEN desktop が起動し、サーバが応答できる
WHEN 利用者が backend の処理を呼ぶ UI 機能を操作する
THEN その処理は Connect プロトコルの RPC としてサーバへ送られる
AND 画面には変更前と同じ結果が反映される

## B-002: desktop の通信が Tauri IPC を経由しない

GIVEN desktop が起動している
WHEN desktop が backend を呼ぶ
THEN その通信は Tauri IPC を経由しない

## B-003: push の受信

GIVEN desktop が起動し、サーバが応答できる
WHEN backend の状態が変化し push が発生する
THEN その push は Connect の server streaming で desktop に届く
AND 画面には変更前と同じ結果が反映される

## B-004: terminal の出力

GIVEN desktop が起動し、terminal が存在する
WHEN terminal が出力する
THEN その出力は Connect の server streaming でクライアントへ届く
AND 画面には変更前と同じ内容が表示される

## B-005: terminal への入力

GIVEN desktop が起動し、terminal が存在する
WHEN 利用者が terminal へ入力する
THEN その入力は unary の RPC としてサーバへ送られる
AND terminal はその入力を受け取る

## B-006: 自前 RPC 層の不在

`proto/client.proto` から生成されないクライアント側の RPC 層は、リポジトリに存在しない。

## B-007: 新しいプラットフォームのクライアントの追加

GIVEN `proto/client.proto` から生成したクライアントコードと、そのプラットフォームの transport アダプタがある
WHEN そのクライアントがサーバの RPC を呼ぶ
THEN desktop と同じ RPC を同じ手順で呼べる
AND 手書きの RPC 層を追加する必要がない

## B-008: token 失効の反映

GIVEN クライアントが有効な token でサーバへの通信を開始している
WHEN その token が失効し、その後クライアントが次の要求を送る
THEN その要求は拒否される

## B-009: 許可しない Origin の拒否

GIVEN サーバが動作している
WHEN 許可しない Origin から、有効な token を伴うクライアント向け RPC の要求が送られる
THEN その要求は拒否される

## B-010: desktop の WebView の Origin

GIVEN desktop が起動している
WHEN desktop の WebView（macOS / Linux は `tauri://localhost`、Windows は `http://tauri.localhost`）からクライアント向け RPC の要求が送られる
THEN その Origin は許可され、要求は Origin を理由に拒否されない

## B-011: master token の非露出

GIVEN desktop が起動している
WHEN renderer が受け取る接続情報を確認する
THEN master token は含まれない
AND master token は daemon の discovery file にだけ書かれている

## B-012: 通信の内部状態を提示しない

GIVEN desktop が起動している
WHEN サーバとの通信が、未送信・結果不明・再接続中のいずれかの状態になる
THEN その状態を提示する表示は画面に現れない

## B-013: 接続断からの回復

GIVEN desktop が起動し、サーバとの接続が切れる
WHEN 接続が回復する
THEN 状態の再取得、push の購読の復旧、terminal の再同期が行われる
AND 画面には現在の状態が反映される

## B-014: CLI と provider hook の HTTP local API

GIVEN daemon が動作している
WHEN CLI または provider hook が HTTP local API を呼ぶ
THEN 変更前と同じ経路・同じ認証で応答が返る

## B-015: daemon 監督の画面

GIVEN desktop が起動する
WHEN daemon の起動・監視・停止の状態が変化する
THEN `DaemonBoundary` の画面は変更前と同じ結果を表示する

## B-016: milestone 77 判断③・判断⑧の内容

milestone 77 の判断③は、クライアント通信を Connect で行うことを定める。判断⑧は、terminal を汎用エンベロープから分離した独立の RPC で扱うことを定める。

## B-018: 起動・切替中の変更要求

GIVEN daemon が起動中または切替中である
WHEN クライアントが変更要求を送る
THEN その要求は、起動中または切替中であることを理由に送信前に拒否されない

## B-019: 応答が返らなかった変更要求

GIVEN クライアントが変更要求を送り、応答が返らないまま接続が切れる
WHEN 接続が回復する
THEN その要求は自動で再送されない
AND 画面には再取得した現在の状態が反映される

## B-020: 正本 Issue が求める spec の改訂

要求の正本である Issue #1829 の本文には、`docs/specs/issues-1201/behavior.md` の B-009〜B-031 を改訂することを求める記述が無い。

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-003 |
| R-003 | B-004, B-005 |
| R-004 | B-006 |
| R-005 | B-007 |
| R-006 | B-008 |
| R-007 | B-009, B-010 |
| R-008 | B-011 |
| R-009 | B-012 |
| R-010 | B-013 |
| R-011 | B-001, B-003, B-004, B-005 |
| R-012 | B-014 |
| R-013 | B-015 |
| R-014 | B-016 |
| R-016 | B-018 |
| R-017 | B-019 |
| R-018 | B-020 |
