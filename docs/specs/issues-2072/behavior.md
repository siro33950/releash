## B-001: 作成した Worktree の表示

GIVEN main Worktree を含む Repository の一覧が展開されている
WHEN GitHub Issue または Notion task から複数の Worktree を作成して Workflow を起動し、その状態が配信される
THEN 作成した各 Worktree が所属する Repository の一覧に表示される

## B-002: 作成後の画面更新の継続

GIVEN main Worktree を含む Repository で GitHub Issue または Notion task から複数の Worktree を作成して Workflow を起動した
WHEN 一覧の更新後にサーバから表示対象の状態変化が配信される
THEN 一覧とフッターを含む画面に、その状態変化が反映され続ける
AND 利用者は表示を更新するためにアプリを再起動する必要がない

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
