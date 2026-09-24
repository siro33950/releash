## B-001: 書き込みが async の 1 つの入口を通る

GIVEN local event store へ事実を保存する処理がある
WHEN その処理が store へ書き込む
THEN 書き込みは async の入口を通る
AND 処理を止めて結果を待つ書き込みの入口は無い

## B-002: 書き込みの待ちが他の呼び出しを止めない

GIVEN ある書き込みが結果を返すまで時間がかかっている
WHEN その間に別の呼び出しが daemon へ届く
THEN 別の呼び出しは、先の書き込みの完了を待たずに応答を返す

## B-003: 待ち行列が溢れた書き込みの失敗が入口によらず `UNAVAILABLE` になる

GIVEN 事実を保存する処理が複数の経路にある
WHEN いずれの経路でも、writer の待ち行列が溢れて書き込みが受け付けられない
THEN どの経路でも失敗は `UNAVAILABLE` として観測される

## B-004: 書き込みの失敗が理由に対応した分類で届く

GIVEN store への書き込みが SQLite のエラーで失敗する
WHEN 呼び出し元がその失敗を受け取る
THEN 失敗はそのエラーの種類に対応した分類を伴って届く
AND 分類は書き込みの経路によって変わらない

## B-005: batch の大きさ上限を超えた書き込みの失敗が `RESOURCE_EXHAUSTED` になる

GIVEN store への書き込みが batch の大きさ上限を超えている
WHEN 呼び出し元がその失敗を受け取る
THEN 失敗は `RESOURCE_EXHAUSTED` として観測される

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
