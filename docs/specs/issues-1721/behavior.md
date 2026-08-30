## B-001: トップレベルの Sequence 行

GIVEN Workspace ツリーにトップレベルの Sequence 行が存在する
WHEN Workspace ツリーが表示される
THEN すべてのトップレベルの Sequence 行は Node 種別アイコンとして `Waypoints` を表示する

## B-002: 入れ子の Sequence 行

GIVEN Workspace ツリーに別の合成 Node の配下へ入れ子になった Sequence 行が存在する
WHEN Workspace ツリーが表示される
THEN すべての入れ子の Sequence 行は Node 種別アイコンとして `Waypoints` を表示する

## B-003: Sequence 行のアイコン表示規則

GIVEN Workspace ツリーに `active`、`attention`、`failure`、`idle` のいずれかに分類された Sequence 行が存在する
WHEN Sequence 行の `Waypoints` が表示される
THEN `Waypoints` は14pxで表示される
AND `active` は青、`attention` は黄、`failure` は赤、`idle` は緑で表示される
AND `active` と `attention` では pulse する
AND `failure` と `idle` では pulse しない

## B-004: Fanout 行のアイコン維持

GIVEN Workspace ツリーに Fanout 行が存在する
WHEN Workspace ツリーが表示される
THEN Fanout 行は Node 種別アイコンとして既存の `GitFork` を表示する

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-003 |
| R-003 | B-004 |