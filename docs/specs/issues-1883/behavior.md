## B-001: 期限を付けない呼び出しが既定の期限で終わる

GIVEN client が期限を付けずに単発の呼び出しを送る
WHEN daemon の既定の期限を超えても処理が終わらない
THEN その呼び出しは `DEADLINE_EXCEEDED` で終わる

## B-002: client が付けた期限で呼び出しが終わる

GIVEN client が期限を付けて単発の呼び出しを送る
WHEN その期限を超えても処理が終わらない
THEN その呼び出しは `DEADLINE_EXCEEDED` で終わる

## B-003: client の中断で呼び出しが終わる

GIVEN 処理中の単発の呼び出しがある
WHEN client がその呼び出しをやめる
THEN その呼び出しは `CANCELLED` として終わる

## B-004: 期限切れと中断で同時実行の枠が解放される

GIVEN 同時実行の枠を確保した単発の呼び出しが処理中である
WHEN その呼び出しが期限切れまたは client の中断で終わる
THEN その呼び出しが確保していた枠は解放される
AND 以後の単発の呼び出しは、その枠を使える

## B-005: 取り消しが処理の先へ伝わる

GIVEN 単発の呼び出しが、入口より先の処理を実行している
WHEN 期限切れまたは client の中断が起きる
THEN 取り消しを受け取った処理は実行を止める

## B-006: 購読の stream は期限で打ち切られない

GIVEN client が購読の stream を開いている
WHEN 単発の呼び出しの既定の期限より長い時間、状態の変化が無い
THEN stream は期限切れで終わらない
AND bookmark が届き続ける

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-006 |
