## B-001: 削除確定でダイアログが待たずに閉じる

GIVEN worktree の削除確認ダイアログが開いている
WHEN 利用者が削除を確定する
THEN 削除処理の完了を待たずに削除ダイアログが閉じる
AND 利用者は画面の操作を続けられる

## B-002: 受理時点で実行木は Archive 済みである

GIVEN 削除対象の worktree に Archive されていない実行木がある
WHEN 利用者が削除を確定し、削除ダイアログが閉じる
THEN その時点で対象 worktree の実行木は Archive 済みである

## B-003: 受理前の失敗はダイアログ上で分かる

GIVEN 削除対象の worktree について、削除前の検証または実行木の Archive が失敗する
WHEN 利用者が削除を確定する
THEN 削除ダイアログは開いたままで、失敗が表示される
AND 対象 worktree は削除されない

## B-006: 受理から削除処理が終わるまでの一覧の扱い

GIVEN worktree の削除が受理され、その削除処理が終わっていない
WHEN 利用者が workspace 一覧を見る
THEN 対象 worktree は削除中と分かる形で一覧に残る

## B-007: 受理から削除処理が終わるまでの状態変更と読み取り

GIVEN worktree の削除が受理され、その削除処理が終わっていない
WHEN 外部の入口から対象 worktree に対する状態変更を要求する
THEN その要求は受理されない
AND 同じ worktree に対する読み取りは引き続き行える

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002, B-003 |
| R-005 | B-006 |
| R-006 | B-007 |
