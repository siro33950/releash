## B-001: 接続の状態を 1 か所から読める

GIVEN 画面が daemon とつながっている
WHEN 接続が確立、切断、つなぎ直しを経る
THEN 接続の状態は CONNECTING・READY・TRANSIENT_FAILURE・IDLE・SHUTDOWN のいずれかとして client の 1 か所から読め、gRPC Connectivity Semantics で許された遷移だけを経る
AND 単発の呼び出し、terminal、`DaemonBoundary` は同じ接続の状態を読む

## B-002: 単発の呼び出しの失敗で接続は変わらない

GIVEN 接続の状態が READY である
WHEN 単発の呼び出しが失敗する
THEN 接続の状態は READY のままである
AND 購読の stream は切られず、購読している表示は更新され続ける

## B-003: 購読の stream が切れるとつなぎ直す

GIVEN 接続の状態が READY である
WHEN 購読の stream が切れる、または bookmark を含めて何も届かない時間が規則の時間を超える
THEN 接続の状態は TRANSIENT_FAILURE になる
AND つなぎ直しの待ちの後に CONNECTING になり、接続先を Tauri のシェルから受け取り直す

## B-004: 接続の確立に期限がある

GIVEN 接続の状態が CONNECTING である
WHEN 接続先の受け取りから検証までが 20 秒以内に終わらない
THEN 接続の状態は TRANSIENT_FAILURE になる

## B-006: IDLE 中の呼び出しは接続を始める

GIVEN 接続の状態が IDLE である
WHEN 単発の呼び出しを発行する
THEN 接続の状態は CONNECTING になる
AND 呼び出しは READY になってから送られる

## B-007: CONNECTING 中の呼び出しは期限まで待つ

GIVEN 接続の状態が CONNECTING である
WHEN 単発の呼び出しを発行する
THEN 呼び出しは、READY になれば送られ、呼び出しの期限までに READY にならなければ失敗する

## B-008: TRANSIENT_FAILURE・SHUTDOWN 中の呼び出しは即座に失敗する

GIVEN 接続の状態が TRANSIENT_FAILURE または SHUTDOWN である
WHEN 単発の呼び出しを発行する、または terminal に入力する
THEN 呼び出しは待たずに失敗する

## B-009: 利用者の操作の失敗が画面に出る

GIVEN Current Behavior に挙げた、利用者の操作、または経路で分かれる呼び出し元がある
WHEN その呼び出しが失敗する
THEN 失敗が画面に表示される

## B-010: 表示用の計算の失敗が「結果が無い」と区別して出る

GIVEN 言語の判定、差分の折りたたみ範囲、ファイルの移動先のいずれかを表示している
WHEN その計算の呼び出しが失敗する
THEN 失敗が画面に表示され、計算の結果が無い場合と区別できる

## B-011: 計測の失敗はログに残る

GIVEN 計測の呼び出しを発行する
WHEN その呼び出しが失敗する
THEN 失敗はログに残る
AND 画面には表示されず、アプリの操作は続けられる

## B-012: つなぎ直しの前後で terminal の入力が失われない

GIVEN terminal に入力している
WHEN 購読の stream が切れてつなぎ直し、その前後にも入力を続ける
THEN 各入力は、送った順番どおりに terminal に届くか、失敗として画面に表示される
AND つなぎ直しの後の入力は、表示の無いまま保留され続けない

## B-013: 接続の状態による入力の失敗では attachment を張り直さない

GIVEN terminal に入力している
WHEN 入力の送信が、接続が READY でないこと、または UNAVAILABLE で失敗する
THEN 失敗が画面に表示される
AND attachment は張り直されない

## B-014: シェルの状態を変化の通知で受け取る

GIVEN 画面が表示されている
WHEN Tauri のシェルの状態が変わる
THEN `DaemonBoundary` の表示は、定期の読み直しを待たずに、変化の通知で更新される
AND client の接続の状態は、シェルの状態とは別の値として読める

## B-015: GetServerInfo は接続の確立にだけ使う

GIVEN 画面が daemon とつながっている
WHEN 接続の確立、READY の間の通信、つなぎ直しを経る
THEN `GetServerInfo` は接続の確立のときにだけ呼ばれる

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-001, B-013 |
| R-003 | B-002 |
| R-004 | B-003 |
| R-005 | B-004 |
| R-007 | B-015 |
| R-008 | B-006, B-007, B-008 |
| R-009 | B-009, B-010 |
| R-010 | B-011 |
| R-011 | B-012 |
| R-012 | B-013 |
| R-013 | B-014 |
