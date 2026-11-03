## B-001: Session が消えたらタブを消す

GIVEN 開発者が中央のカラムに Session のタブを開いている
WHEN 開発者が Node 面でその Session を Archive する
THEN Swift.app はその Session のタブを消す

## B-002: Workflow が Session を起動したらタブを増やす

GIVEN 開発者が worktree を開き、あるタブを選択している
WHEN その worktree で動いている Workflow の Node が Session を起動する
THEN Swift.app はその Session のタブを中央のカラムに増やす
AND Swift.app は選択中のタブと、各 pane が表示しているタブを変えない

## B-003: Restore したらタブを増やす

GIVEN 開発者が worktree を開き、あるタブを選択している
WHEN 開発者が History から Archive 済みの Session を Restore する
THEN Swift.app はその Session のタブを中央のカラムに増やす
AND Swift.app は選択中のタブと、各 pane が表示しているタブを変えない

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002, B-003 |
| R-003 | B-002, B-003 |
