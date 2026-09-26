## B-001: 送付中に実行の状態が変わっても注入済みが記録される

GIVEN delegate を持つ親 Session の child が完了し、結果の注入が始まっている
WHEN provider へ結果を送付している間に、同じ実行へ別の事実が記録される
THEN 注入済みの記録が保存される
AND provider の会話へ同じ child の結果が二度届かない

## B-002: 保存の直前に競合しても注入済みが記録される

GIVEN delegate を持つ親 Session の child が完了し、provider への結果の送付が成功している
WHEN 注入済みを保存する直前に、同じ実行へ別の事実が記録される
THEN 注入済みの記録が保存される
AND provider の会話へ同じ child の結果が二度届かない

## B-003: Resume なしで次の Artifact 提出が受理される

GIVEN 親 Session の注入済みが記録されている
WHEN 親 Session が次の Artifact を提出する
THEN 提出が受理される
AND provider のプロセスの終了と Resume を要しない

## B-004: 競合した相手の事実が保たれる

GIVEN 注入済みの保存と競合した別の事実が、同じ実行に記録されている
WHEN 注入済みの記録が保存される
THEN 競合した相手の事実は失われない

## B-005: 既に注入済みなら古い更新を適用しない

GIVEN 対象の注入が既に注入済みとして記録されている
WHEN 同じ注入について注入済みを記録しようとする
THEN 実行の状態は変わらない
AND 保存のやり直しが終わる

## B-006: 対象が変わっていれば古い更新を適用しない

GIVEN 親 Session の待っている注入が、別の注入へ変わっている
WHEN 元の注入について注入済みを記録しようとする
THEN 実行の状態は変わらない
AND 保存のやり直しが終わる

## B-007: 競合が続いても Abort が完了する

GIVEN 注入済みの保存が競合し、やり直しが続いている
WHEN workflow の Abort を要求する
THEN Abort が完了する

## B-008: Abort された後はやり直しを続けない

GIVEN 注入済みの保存のやり直しが続いている
WHEN workflow が Abort される
THEN 実行の状態は変わらない
AND 保存のやり直しが終わる

## B-009: やり直しが続いている間の失敗が観測できる

GIVEN 注入済みの保存が競合し続けている
WHEN 保存のやり直しが続く
THEN その失敗が、回数と最初・最後の時刻を伴う記録として、対象の Node から観測できる

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-003 |
| R-003 | B-004 |
| R-004 | B-005, B-006, B-008 |
| R-005 | B-007 |
| R-006 | B-009 |
