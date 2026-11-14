## B-001: 中央の「+」のメニューの項目

GIVEN 開発者が worktree を選び、中央のカラムに pane が表示されている
WHEN 開発者がその pane の「+」を押す
THEN メニューは、上から「Session ▸」「Workflow ▸」「Terminal」の順に項目を表示する

## B-002: 中央の「+」の一覧は Node 面の「+」と同じ

GIVEN 開発者が worktree を選んでいる
WHEN 開発者が中央の「+」の「Session ▸」と「Workflow ▸」を開く
THEN 「Session ▸」は、Node 面の「+」の「Session ▸」と同じ provider の一覧を表示する
AND 「Workflow ▸」は、Node 面の「+」の「Workflow ▸」と同じ Workflow の一覧を表示する

## B-003: Node 面の「+」のメニューの項目

GIVEN 開発者が worktree を選び、右カラムに Node 面を表示している
WHEN 開発者が Node 面の「+」を押す
THEN メニューは、上から「Session ▸」「Workflow ▸」の順に項目を表示する
AND 「New」の付いた項目は表示されない

## B-004: 中央の「+」から始めた Session のタブは押した pane に開く

GIVEN 開発者が中央のカラムを 2 つの pane に分割し、先頭の pane にフォーカスを置いている
WHEN 開発者が先頭ではない側の pane の「+」の「Session ▸」から provider を選ぶ
THEN 選択中の worktree で、その provider の Session が始まる
AND その Session のタブは、「+」を押した pane に開く
AND そのタブは、その pane で選択された状態になる

## B-005: 中央の「+」から Workflow を始めると実行木が Node 面に増える

GIVEN 開発者が worktree を選び、右カラムに Node 面以外の面を表示している
WHEN 開発者が中央の「+」の「Workflow ▸」から Workflow を選ぶ
THEN 選択中の worktree で、その Workflow が始まる
AND 右カラムは、表示していた面を表示したままである
AND 開発者が Node 面に切り替えると、その Workflow の実行木が表示されている

## B-006: 中央の「+」から始めた Workflow のタブが開く pane

GIVEN 開発者が中央のカラムを 2 つの pane に分割している
AND Session の Node を持つ Workflow がある
WHEN 開発者が先頭ではない側の pane の「+」の「Workflow ▸」からその Workflow を選ぶ
THEN その Workflow の Node が起動した Session のタブは、Node 面の「+」から同じ Workflow を始めたときと同じ pane に開く

## B-007: 配信を待っている Root がある間は Workflow を選べない

GIVEN 開発者が Workflow を始め、その実行の Root がまだ Node 面に届いていない
WHEN 開発者が同じ worktree で中央の「+」を押す
THEN メニューは、「Workflow ▸」の位置に「Workflow」を押せない状態で表示する
AND 「Workflow」は Workflow の一覧を表示しない

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
