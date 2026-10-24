## B-001: delegate の各発火を集計する

GIVEN workflow の Session が同じ delegate child を複数回起動し、各発火の部分木に Session と Command が存在する
AND その部分木には、さらに delegate によって起動した Node があり、retry は行われていない
WHEN Workspaces 一覧にその workflow 行が表示される
THEN N は、親 Session と各発火の部分木にある起動済みの Session と Command をすべて含む数になる
AND Sequence と Fanout 自身は N に含まれない
AND 集計対象の Session 一つにつき、その Session の状態の色の四角が一つ表示される

## B-002: 通常のループ再訪を集計する

GIVEN workflow が通常のループで同じ Node を再訪し、各回の Session と Command が実行木に存在する
AND retry は行われていない
WHEN Workspaces 一覧にその workflow 行が表示される
THEN N は、再訪前の実行とその部分木を含む、起動済みの Session と Command の総数になる
AND 集計対象の Session 一つにつき、その Session の状態の色の四角が一つ表示される

## B-003: retry の履歴とその部分木を集計から除く

GIVEN workflow の実行木に、retry で置き換えられた過去の試行とその配下の部分木、および置き換え後の実行が存在する
WHEN Workspaces 一覧にその workflow 行が表示される
THEN N は、retry の履歴とその部分木を除いた実行木の Session と Command の数に一致する
AND retry の履歴とその部分木にある Session の四角は表示されない
AND 集計対象の Session 一つにつき、その Session の状態の色の四角が一つ表示される

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002, B-003 |
| R-002 | B-001, B-002, B-003 |
