## B-001: 初期の開閉

GIVEN 実行木に、status が idle の子を持つ行と、active または attention の子を持つ行がある
WHEN 開発者が Node 面を開く
THEN idle の行は閉じていて、その子の行は出ない
AND active または attention の行は開いていて、その子の行が出る

## B-002: status の変化への追従

GIVEN Node 面に、status が active または attention で開いている子を持つ行がある
WHEN その行の status が idle に変わる
THEN その行は閉じる
AND その行の status が再び active または attention に変わると、その行は開く

## B-003: 手動の開閉の優先

GIVEN Node 面に、status が active で開いている子を持つ行がある
WHEN 開発者がその行を手で閉じ、その後その行の status が attention に変わる
THEN その行は閉じたままである
AND その行の status が idle に変わると、その行は閉じたままである
AND その行の status が再び active または attention に変わると、その行は開く

## B-004: 手で開いた idle の行の追従

GIVEN Node 面に、status が idle で閉じている子を持つ行がある
WHEN 開発者がその行を手で開き、その後その行の status が active または attention に変わる
THEN その行は開いたままである
AND その行の status が idle に変わると、その行は閉じる

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003, B-004 |
