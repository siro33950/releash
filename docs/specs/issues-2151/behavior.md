## B-001: 期限を過ぎた読み取りのあとの読み取りの成功

GIVEN event store の読み取りが 1 件、期限を過ぎて失敗した
WHEN サーバを再起動せずに、後続の event store の読み取りを繰り返し行う
THEN 後続の読み取りは、すべて成功する

## B-002: 期限を過ぎた読み取りのあとに書き込まれた記録の読み取り

GIVEN event store の読み取りが 1 件、期限を過ぎて失敗した
AND その失敗のあとに、event store へ記録が書き込まれた
WHEN サーバを再起動せずに、後続の event store の読み取りを繰り返し行う
THEN 後続の読み取りは、すべてその記録を返す

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
