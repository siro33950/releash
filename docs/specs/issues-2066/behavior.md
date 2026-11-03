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

## B-004: サーバが変えた配置を画面の保存で上書きしない

GIVEN 開発者が worktree を開き、Swift.app が配置を受け取っている
AND サーバが Session の増減でタブ配置を変え、Swift.app がまだその配信を受け取っていない
WHEN 開発者がタブを閉じる
THEN サーバはその保存を拒否し、Session の増減で変えた配置を保つ
AND Swift.app はサーバの配置を読み直して表示し、エラーを表示しない

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002, B-003 |
| R-003 | B-002, B-003 |
| R-004 | B-004 |
