## B-001: 配信する状態の型

購読で配信する状態の型は、Output Data だけでできている。どの状態の型も、そのフィールドと、フィールドの中身の型を含めて、domain の型を含まない。

## B-002: presenter の依存先

presenter の本番コードは gateway の型と関数を参照しない。presenter が転送の形とステータスコードに変える元の型は、usecase と domain の型だけである。

## B-003: 保存の入力

GIVEN 画面が workflow の設定を保存する
WHEN `update_workflow_config` が呼ばれる
THEN controller は受け取った値を Input Data に変えて Usecase に渡す
AND controller は gateway の型と変換を使わない

## B-004: gateway の失敗

GIVEN gateway が記録の読み取りで失敗する
WHEN その失敗が gateway の外へ出る
THEN 失敗は domain の失敗として presenter に届く
AND presenter はその domain の失敗をステータスコードに対応付ける

## B-005: provider の Output Data

usecase には、provider を表す Output Data の型が 1 つだけあり、domain の provider からそれへの写し方が 1 つだけある。provider を Output Data として返す読み取り、購読の配信、QueryService は、すべてその型と写し方を使う。

## B-006: provider の CLI を変える操作の応答

GIVEN provider の設定の画面を開いている
WHEN `update_provider_executable`・`reset_provider_executable`・`refresh_provider_availability` のいずれかが成功する
THEN 応答は状態を含まない
AND 画面は、購読で届いた変更後の状態を表示する

## B-007: reset で他の provider の入力が消えない

GIVEN provider の設定の画面で、二つの provider が表示されている
AND 一方の provider に保存していない入力がある
WHEN もう一方の provider を reset する
THEN 保存していない入力は、reset の後も表示されたまま残る

## B-008: refresh の失敗

GIVEN provider の設定の画面に provider の一覧が表示されている
WHEN refresh が失敗する
THEN 画面は直前の一覧を表示したまま、エラーを表示する

## B-009: 画面が受け取る値

GIVEN 画面が `Failures`・`Terminal`・`Workflows`・`Workflow`・`NotionConfig`・`ProviderAvailability`・`WorkflowConfig`・`ProviderHookHealth` のいずれかを購読している
WHEN daemon がその状態を配信する
THEN 画面が受け取る値は、この変更の前に同じ状態から受け取る値と同じである

## B-010: 使われなくなるコード

この変更で使われなくなる型、関数、変換、proto のメッセージは残らない。

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-006 |
| R-007 | B-007 |
| R-008 | B-008 |
| R-009 | B-009 |
| R-010 | B-010 |
