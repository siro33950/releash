## B-001: 実行木の完了記録

GIVEN 実行中の実行木が存在する
WHEN 実行木の完了条件が成立する
THEN 実行木が完了した事実が事実ログへ記録される
AND 実行の状態は `Completed` になる

## B-002: 完了事実による終端状態の維持

GIVEN 実行木が完了した事実が事実ログに存在する
AND 保存定義を現行コードで解釈できない
WHEN 実行の状態を読み取る
THEN WorkflowExecutionは `Completed` と判定される
AND WorkflowExecutionは `Running` または `Aborted` に変化しない

## B-003: 未完了で定義を解釈できない実行の起動時Abort

GIVEN 完了またはAbortの事実を持たない実行が存在する
AND その保存定義を現行コードで解釈できない
WHEN 起動時の処理がその実行を扱う
THEN 定義を解釈できない理由を伴うAbortの事実が記録される
AND 以後の読み取りで実行は `Aborted` と判定される
AND Nodeは `Unresolved` として公開されない

## B-004: 既にRunningへ戻った実行の収束

GIVEN 過去に自然完了したが完了の事実を持たない実行が存在する
AND 保存定義を現行コードで解釈できないため、その実行が `Running` と判定されている
WHEN 起動時の処理がその実行を扱う
THEN 定義を解釈できない理由を伴うAbortの事実が記録される
AND 以後の読み取りで実行は `Aborted` と判定される

## B-005: 既存終端実行のNode公開情報の互換維持

GIVEN 完了またはAbortの終端事実を持つ既存実行が存在する
AND その保存定義を現行コードで解釈できる
WHEN 実行の状態を読み取る
THEN NodeExecutionの公開情報は、同じ事実列と保存定義から変更前に得られていた値を維持する
AND WorkflowExecutionの終端状態は終端事実から判定される

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-003, B-004 |
| R-006 | B-005 |
