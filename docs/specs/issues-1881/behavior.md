## B-001: 読み込みが async の 1 つの入口を通る

GIVEN local event store に保存された事実を読む処理がある
WHEN その処理が store から読む
THEN 読み込みは async の入口を通る
AND 処理を止めて結果を待つ読み込みの入口は無い

## B-002: 読み込みの待ちが他の呼び出しを止めない

GIVEN ある読み込みが結果を返すまで時間がかかっている
WHEN その間に別の呼び出しが daemon へ届く
THEN 別の呼び出しは、先の読み込みの完了を待たずに応答を返す

## B-003: CLI の読み込みも同じ入口を通る

GIVEN daemon が動いていない
WHEN CLI が保存された事実を読む command を実行する
THEN CLI は daemon と同じ読み込みの入口を通って結果を出力する

## B-004: 混雑による読み込みの失敗が `UNAVAILABLE` になる

GIVEN store の読み込みが混雑のため受け付けられない
WHEN 呼び出し元がその失敗を受け取る
THEN 失敗は `UNAVAILABLE` として観測される

## B-005: 期限切れによる読み込みの失敗が `DEADLINE_EXCEEDED` になる

GIVEN store の読み込みが期限切れで打ち切られる
WHEN 呼び出し元がその失敗を受け取る
THEN 失敗は `DEADLINE_EXCEEDED` として観測される

## B-006: SQLite のエラーによる読み込みの失敗が経路によらず同じ分類になる

GIVEN 保存された事実を読む処理が複数の経路にある
WHEN いずれの経路でも、データベースが他から使用中であることにより読み込みが失敗する
THEN どの経路でも失敗は `UNAVAILABLE` として観測される

## B-007: 実行環境に起因する読み込みの失敗が `FAILED_PRECONDITION` になる

GIVEN store のあるディスクが I/O エラーを返す
WHEN 保存された事実を読む処理がその失敗を受け取る
THEN 失敗は `FAILED_PRECONDITION` として観測される

## B-008: store を開くときの失敗の分類が変わらない

GIVEN store を開く処理が SQLite のエラーで失敗する
WHEN そのエラーが権限・容量・I/O・ロック・使用中のいずれかに起因する
THEN 失敗は storage が使えないこととして観測される

## B-009: 保存データの破損による読み込みの失敗が `DATA_LOSS` になる

GIVEN 保存された事実が破損している
WHEN 保存された事実を読む処理がその失敗を受け取る
THEN 失敗は `DATA_LOSS` として観測される

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-003 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004, B-005 |
| R-005 | B-006, B-007, B-009 |
| R-006 | B-008 |
