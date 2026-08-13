## B-001: 過去仕様を名指しする拒絶ガードの不在

GIVEN CLI経路とWorkflow YAML経路がある
WHEN 保守者がそれらのsource、test、fixtureを確認する
THEN 過去のcommand、flag、field、または語彙を名指しで拒否するblacklist、source scan、negative assertion、fixtureが存在しない
AND 過去の具体的な構文が拒否されることを固定するtestが存在しない

## B-002: 未知入力の拒否理由が過去仕様に依存しない

GIVEN Workflow YAMLに、現在仕様で定義されていないfieldまたはkeywordが含まれている
WHEN workflow authorがそのWorkflow YAMLをload、保存、または実行する
THEN そのfieldまたはkeywordが過去仕様のものであるかどうかによらず、同一のDiagnostic codeで拒否される

## B-003: 未知入力の拒否testで使用する名前

GIVEN CLIの未知optionまたはWorkflow YAMLの未知fieldが拒否されることを確認するtestがある
WHEN 保守者がそのtestの入力を確認する
THEN 未知入力には、その時点の仕様に存在しない一般名が使われている
AND 過去に存在した具体的なoption名またはfield名は使われていない

## B-004: CLIの未知入力と引数errorの拒否

GIVEN CLIに、現在定義されていないoption、現在定義されていないsubcommand、必須引数の欠落、または既知引数で受理されない型の値のいずれかが指定されている
WHEN 開発者またはAgentがCLIを実行する
THEN CLIはエラーを返す

## B-005: Workflow YAMLの未知fieldとkeywordの拒否

GIVEN Workflow YAMLのtop-level、node、command・session・fanoutの各kind block、session facets、rule要素、またはschema mapのいずれかに、現在仕様で定義されていないfieldまたはkeywordが追加されている
WHEN workflow authorがそのWorkflow YAMLをload、保存、または実行する
THEN その操作は拒否される

## B-006: 現在仕様の検証契約の維持

GIVEN 現在仕様の必須値、型、相互排他、参照整合性、または状態遷移の制約について、変更前と同じ入力または操作が与えられる
WHEN 対応する検証が行われる
THEN 受理または拒否の判定は変更前と同じである

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-004 |
| R-004 | B-005 |
| R-005 | B-006 |
| R-006 | B-003 |
