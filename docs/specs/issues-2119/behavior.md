## B-001: Node が終わると Session のタブが閉じる

GIVEN Workflow の Session の Node が動いていて、その Session のタブが中央のカラムに開いている
WHEN その Node が終わり、Session が止まる
THEN その Session のタブは中央のカラムから閉じる

## B-002: 実行を Abort すると Session のタブが閉じる

GIVEN 実行の Session が動いていて、その Session のタブが中央のカラムに開いている
WHEN 開発者がその実行を Abort し、Session が止まる
THEN その Session のタブは中央のカラムから閉じる

## B-003: provider が自分で終了すると Session のタブが閉じる

GIVEN provider が会話を始めている Session が動いていて、その Session のタブが中央のカラムに開いている
WHEN provider が `/exit` または異常終了で終了し、Session が止まる
THEN その Session のタブは中央のカラムから閉じる

## B-004: Session が Node 面から消える場合もタブが閉じる

GIVEN Workflow に属さない Session を開始し、provider がまだ会話を始めておらず、その Session のタブが中央のカラムに開いている
WHEN provider が自分で終了し、その Session が Node 面から消える
THEN その Session のタブは中央のカラムから閉じる

## B-005: 実行を Archive すると Session のタブが閉じる

GIVEN 実行の Session が動いていて、その Session のタブが中央のカラムに開いている
WHEN 開発者がその実行を Archive する
THEN その実行に属する Session のタブは中央のカラムから閉じる

## B-006: 開き直したタブも Archive で閉じる

GIVEN 止まった Session のタブを、開発者が Node 面の ▷ で開き直している
WHEN 開発者がその Session の属する実行を Archive する
THEN その Session のタブは中央のカラムから閉じる

## B-007: 止まった Session のタブを開き直せる

GIVEN Session が止まり、その Session のタブが閉じている
WHEN 開発者が Node 面でその Session の ▷ を押す
THEN その Session のタブが中央のカラムに開く
AND そのタブは「Sessionは停止しています」と表示する
AND 再開できる Session では、そのタブは Resume ボタンを表示する
AND Session が止まっているあいだ、そのタブは閉じない

## B-008: Resume を選ぶと Session のタブが開く

GIVEN Session が止まり、その Session のタブが閉じている
WHEN 開発者が Node 面の右クリックメニューでその Session の Resume を選び、process が再び動く
THEN その Session のタブが中央のカラムに開く

## B-009: Workflow が Session を起動し直すとタブが開く

GIVEN Workflow の Session が止まり、その Session のタブが閉じている
WHEN Workflow がその Session を起動し直し、process が再び動く
THEN その Session のタブが中央のカラムに開く

## B-010: 既に開いているタブは増えない

GIVEN 止まった Session のタブを、開発者が Node 面の ▷ で開き直している
WHEN その Session の process が再び動く
THEN 中央のカラムにある、その Session のタブは一つのままである

## B-011: 閉じたタブは再起動しても開かない

GIVEN Session が止まり、その Session のタブが閉じている
WHEN 開発者が Swift.app を再起動する
THEN その Session のタブは中央のカラムに開かない

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002, B-003 |
| R-002 | B-004 |
| R-003 | B-005, B-006 |
| R-004 | B-007 |
| R-005 | B-008, B-009 |
| R-006 | B-010 |
| R-007 | B-011 |
