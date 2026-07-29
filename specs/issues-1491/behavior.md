## B-001: Workspace tree表示情報の一括取得

GIVEN 表示可能なWorkspace treeが存在する
WHEN clientがWorkspace treeをqueryする
THEN treeの表示に必要な情報が一度のresponseで返る

## B-002: 無関係な蓄積があるWorkspace treeとNodeの取得

GIVEN 対象のWorkspace tree、Node、Sessionと無関係なSession、event、workflow executionがWorkspaceに蓄積している
WHEN clientがWorkspace tree、Node detail、またはSession IDに対応するNodeをqueryする
THEN それぞれの対象の結果が返る
AND 応答に要する時間が無関係な蓄積量の増加に伴って増えない

## B-003: 無関係な蓄積があるSession一覧とworkflow execution一覧の取得

GIVEN 一覧の対象外であるSession、event、workflow executionが蓄積している
WHEN clientがSession一覧またはworkflow execution一覧をqueryする
THEN 対象の一覧が返る
AND 応答に要する時間が対象外の蓄積量の増加に伴って増えない

## B-004: 同一対象への繰り返し取得

GIVEN backendが起動し、対象が取得可能である
WHEN clientが同じ対象を繰り返しqueryする
THEN 各回で対象の結果が返る
AND 応答に要する時間が実行回数の増加に伴って増えない

## B-005: 再起動をまたいだWorkspace treeの内容

GIVEN Workspace treeを構成する状態が変わらないまま、アプリケーションが再起動する
WHEN clientがWorkspace treeをqueryする
THEN 再起動前と同じ観測可能な内容が返る

## B-006: client surface間で共通のquery契約

GIVEN Tauri、loopback API、または将来のclient surfaceが同じbackend状態を参照する
WHEN それぞれが同じ対象をqueryする
THEN すべてのclient surfaceが同じbackend queryの契約に基づく同じ内容を取得できる

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002, B-003 |
| R-003 | B-004 |
| R-004 | B-005 |
| R-005 | B-006 |
