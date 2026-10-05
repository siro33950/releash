## B-001: サーバ情報の取得

GIVEN 要求を受けられる状態のサーバが動いている
WHEN クライアントが `GetServerInfo` を呼ぶ
THEN 応答に、サーバが発行する ID、pid、起動時刻、release、protocol、capability の集合、serving status が含まれる
AND serving status は Serving である
AND `launch_id` と `release` は今と同じ値である

## B-002: サーバ情報と発見ファイルの一致

GIVEN 要求を受けられる状態のサーバが、発見ファイルを書いている
WHEN クライアントが `GetServerInfo` を呼ぶ
THEN 応答の ID・pid・起動時刻は、発見ファイルの `instance_id`・`pid`・`process_started_at` と一致する

## B-003: protocol と capability

GIVEN 要求を受けられる状態のサーバが動いている
WHEN クライアントが `GetServerInfo` を呼ぶ
THEN protocol は、サーバがコンパイルした proto package のメジャー版（今は 1）である
AND capability の集合は空である

## B-004: 発見ファイルを書く時点

GIVEN サーバが起動している途中である
WHEN 2 つの発見ファイル（`local-api.json` と `client-api.json`）が書かれる
THEN その時点で、サーバは要求を受けられる状態にある
AND 2 つのファイルの項目、ファイル名、権限、token の分離は今と同じである

## B-005: 停止時の発見ファイル

GIVEN 要求を受けられる状態のサーバが、2 つの発見ファイルを書いている
WHEN サーバが停止要求を受けて終了する
THEN 終了した後、2 つの発見ファイルはどちらも残っていない

## B-006: 要求を受けられる状態での受付

GIVEN serving status が Serving のサーバが動いている
WHEN クライアントが Connect の任意の RPC を呼ぶ
THEN 要求は受け付けられる

## B-007: 停止に入った後の受付

GIVEN サーバが停止要求を受理し、停止手順を進めている
WHEN クライアントが `GetServerInfo` と停止要求以外の Connect の RPC を新しく呼ぶ
THEN 要求は、分類とコード `APPLICATION_UNAVAILABLE` のエラーで拒否される
AND 停止要求を受理する前に開いた stream と、進行中の呼び出しは、この判定では切られない

## B-008: 停止に入った後の情報取得と停止要求

GIVEN サーバが停止要求を受理し、停止手順を進めている
WHEN クライアントが `GetServerInfo` または停止要求を呼ぶ
THEN 要求は受け付けられる
AND `GetServerInfo` の serving status は Stopping である

## B-009: HTTP local API の受付

GIVEN サーバが動いている
WHEN CLI または hook が HTTP local API を呼ぶ
THEN 受け付けるかどうかは今と同じである

## B-010: 停止後の workflow の Command

GIVEN サーバが停止要求を受理した
WHEN workflow が Command を開始しようとする、または Command の結果が届く
THEN Command は開始されず、結果は workflow に取り込まれない

## B-011: 停止要求の exit code

GIVEN 要求を受けられる状態のサーバが動いている
WHEN クライアントが、exit code を付けた Exit または Restart の停止要求を送る
THEN サーバは停止手順を実行して終了する
AND プロセスの終了コードは、要求が運んだ exit code である

## B-012: 停止の deadline

GIVEN サーバが停止手順を進めている
WHEN 停止手順が停止の deadline（15 秒）を過ぎても終わらない
THEN サーバは停止手順を打ち切って終了する
AND deadline の値は proto の service option に 1 か所だけ定義されている

## B-013: 起動失敗

GIVEN サーバが保存先を開けない
WHEN サーバを起動する
THEN サーバは発見ファイルを書かずに、プロセスを終了する
AND 失敗の説明と相関 ID が、今と同じ形でログと stderr に出る

## B-014: 起動失敗の経路の削除

GIVEN 画面とサーバが、この変更を含む版である
WHEN 画面がサーバに接続する
THEN 購読 `startup-outcome` と `QuitAfterStartupFailure` は存在しない
AND proto から消した項目の field 番号と名前は reserved である
AND 画面は、監督が ready になるまで中身を描かない

## B-015: 発見ファイルからの接続先の確認

GIVEN 発見ファイルがある
WHEN CLI またはシェルが、発見ファイルからサーバへの接続先を確かめる
THEN 記録の妥当性、記録の pid と起動時刻が生きているプロセスと一致すること、到達したサーバが記録と同じであることの判定は、今と同じ結果を返す

## B-016: シェルの受付が拒否したときの形

GIVEN 画面側の監督が、Tauri コマンドを受け付けない状態である
WHEN 画面がその Tauri コマンドを呼ぶ
THEN 呼び出しは `{"type":"application_unavailable"}` で reject される

## B-017: OS の終了要求

GIVEN 画面のシェルが動いている
WHEN OS がアプリの終了を要求する
THEN 監督は今と同じ exit code で停止を進め、シェルは終了する

## B-018: 文書の正本

GIVEN この変更を含む版である
WHEN `docs/architecture/README.md` のドメイン一覧と `docs/glossary/DOMAIN.md` を読む
THEN ドメイン一覧に `daemon` があり、`local_api_discovery` は無い
AND glossary に、Daemon、DaemonInfo、serving status、StopRequest の正規語がある

## B-019: 停止中に届いた停止要求

GIVEN サーバが、exit code を付けた停止要求を受理し、停止手順を進めている
WHEN クライアントが、別の exit code を付けた停止要求を送る
THEN 要求は成功（受理済み）として返る
AND プロセスの終了コードは、最初に受理した停止要求の exit code である

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-008 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004, B-005 |
| R-005 | B-006, B-007, B-008, B-009 |
| R-006 | B-010 |
| R-007 | B-011 |
| R-008 | B-012 |
| R-009 | B-013 |
| R-010 | B-014 |
| R-011 | B-015 |
| R-012 | B-016, B-017 |
| R-013 | B-018 |
| R-014 | B-019 |
