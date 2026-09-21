## B-001: 通し番号の振り直し後も未コミット差分が更新される

GIVEN Review パネルが worktree W の未コミット差分を表示している
AND backend との接続が張り直され、W の通し番号が、表示へ反映済みの番号より小さい値から振り直されている
WHEN W で commit が行われ、Review パネルが未コミット差分を取り直す
THEN Staged / Changes の一覧は取り直した結果へ更新される

## B-002: 反映されるのは最後に投げた取得要求の応答だけである

GIVEN Review パネルが同じ worktree と diff base に対して取得要求を続けて投げている
WHEN 応答が投げた順と異なる順で返る
THEN 表示へ反映されるのは、最後に投げた取得要求の応答だけである

## B-003: 番号が一致しない間は差分が stale として返り hunk 単位の操作ができない

GIVEN 表示中の一覧の通し番号が、backend の現在の通し番号と異なる
WHEN その通し番号を添えて差分を取得する
THEN 差分は stale として返る
AND その間、hunk 単位の stage / unstage は操作できない

## B-004: 番号が一致しない画像取得は受け付けられない

GIVEN 表示中の一覧の通し番号が、backend の現在の通し番号と異なる
WHEN その通し番号を添えて画像を取得する
THEN 取得は番号の不一致として失敗する

## B-005: スキャンの開始と完了で repository-snapshot-changed が送られない

GIVEN client が backend の通知を購読している
WHEN worktree のスキャンが開始または完了する
THEN client へ `repository-snapshot-changed` 通知は届かない

## B-006: snapshot は打ち切りフラグを公開しない

GIVEN client が worktree の状態を表す snapshot（status、diff stats、branch cards、head diff file tree、review snapshot）を取得する
WHEN 取得が成功する
THEN 取得内容に打ち切りフラグ（`limited`）は含まれない

## B-007: 通し番号の振り直し後も hunk 単位の操作ができる

GIVEN backend との接続が張り直され、worktree W の通し番号が振り直されている
AND Review パネルが振り直し後に取り直した未コミット差分を表示している
WHEN 表示中のファイルの hunk に対して stage / unstage を行う
THEN その hunk が stage / unstage される

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003, B-004 |
| R-004 | B-005 |
| R-005 | B-006 |
| R-006 | B-007 |
