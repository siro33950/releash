# Context

- 要求の正本は、ISSUE #2114「Swift.app: 中央のタブ列の「+」から Session と Workflow を始められるようにする」の本文である。
- 調査の時点は main `373fb61b` である。
- この文書で「中央の「+」」とは、中央のカラムの pane ごとのタブ列にある「+」を指す。「Node 面の「+」」とは、右カラムの Node 面の右上にある「+」を指す。
- この文書で「配信を待っている Root」とは、開発者が Workflow を始めてから、その実行の Root が Node 面に届くまでの間の Root を指す。Node 面は、この間「Rootの配信を待っています」と表示する。
- この文書で「先頭の pane」とは、中央のカラムを分割していないときはその pane を、分割しているときは分割の最初の側をたどった先の pane を指す。
# Outcome

対象者は、Swift.app で Session と Workflow を動かす開発者である。

今は、Session と Workflow を Node 面の「+」からしか始められない。
開発者は、中央のカラムで作業している途中でも、右カラムを開いて Node 面に切り替えてから始める。
中央のカラムを分割しているときは、始めた Session のタブが開く pane を選べない。

変更後は、開発者が中央の「+」から Session と Workflow を始められる。
中央の「+」から始めた Session のタブは、その「+」を押した pane に開く。

# Current Behavior

調査は main `373fb61b` のコードを読んで行った。
アプリの画面を見ることはしていない。

ISSUE の「現状」の記述のうち、次の 2 点は調査したコードと合わなかった。
以下の各節には、コードから確かめた挙動を書く。

- ISSUE は、始めた Session のタブが「フォーカスのある pane」に開くと書いている。コードでは、先頭の pane に開く。
- ISSUE は、Workflow を始めても開くタブは無いと書いている。コードでは、タブが開く場合がある。

## 中央の「+」のメニュー

- 中央の「+」のメニューは、Terminal の 1 項目だけを持つ。
- Terminal を選ぶと、その「+」を押した pane に Terminal のタブが開く。
- 中央のカラムを分割しているときは、pane ごとに「+」がある。

## Node 面の「+」のメニュー

- Node 面の「+」のメニューは、「New Session ▸」「New Workflow ▸」の 2 項目を持つ。
- 「New Session ▸」は provider の一覧を表示する。「New Workflow ▸」は Workflow の一覧を表示する。
- 「New Workflow ▸」は、選択中の worktree に配信を待っている Root がある間、押せない。

## Session を始めたときのタブ

- Node 面の「+」から Session を始めると、その Session のタブが先頭の pane に開く。フォーカスが別の pane にあっても、先頭の pane に開く。
- 開いたタブは、その pane で選択された状態になる。

## Workflow を始めたときの動き

- Node 面の「+」から Workflow を始めると、その実行木が Node 面に増える。右カラムの表示は切り替わらない。
- 始めた Workflow の Root が単独の Node（Session または command）のときは、Root が Node 面に届いた時点で、その Root のタブがフォーカスのある pane に開く。
- Workflow の Node が Session を起動すると、その Session のタブが先頭の pane に開く。

## 再現手順

コードから読み取った手順である。アプリの画面では実行していない。

1. Swift.app で worktree を選び、中央のカラムを 2 つの pane に分割する。
2. 先頭ではない側の pane の「+」を押す。メニューには Terminal だけが表示され、Session と Workflow を始める項目は無い。
3. 先頭ではない側の pane にフォーカスを置いたまま、右カラムの Node 面を開き、「+」の「New Session ▸」から provider を選ぶ。
4. 始まった Session のタブは、先頭の pane に開く。

# Scope / Non-goals

変更するもの:

- 中央の「+」のメニューの項目。Session と Workflow を始める項目を足す。
- 中央の「+」から始めた Session のタブが開く pane。
- Node 面の「+」のメニューの項目名。

Non-goals:

- Node 面の「+」から始めた Session のタブが開く pane。今のまま、先頭の pane に開く。
# Requirements

- R-001: 中央の「+」のメニューは、上から「Session ▸」「Workflow ▸」「Terminal」の順に項目を持つ。
- R-002: 中央の「+」の「Session ▸」は provider の一覧を、「Workflow ▸」は Workflow の一覧を表示する。それぞれの一覧は、Node 面の「+」の同じ名前の項目が表示する一覧と同じである。
- R-003: Node 面の「+」のメニューは、上から「Session ▸」「Workflow ▸」の順に項目を持つ。項目名に「New」は付かない。
- R-004: 開発者が中央の「+」の「Session ▸」から provider を選ぶと、選択中の worktree でその provider の Session が始まる。その Session のタブは、その「+」を押した pane に開き、その pane で選択された状態になる。フォーカスが別の pane にあっても、「+」を押した pane に開く。
- R-005: 開発者が中央の「+」の「Workflow ▸」から Workflow を選ぶと、選択中の worktree でその Workflow が始まり、その実行木が Node 面に増える。右カラムの表示は切り替わらない。
- R-006: 中央の「+」から始めた Workflow でタブが開く pane は、Node 面の「+」から同じ Workflow を始めたときと同じである。「+」を押した pane がどれであっても変わらない。
- R-007: 中央の「+」の「Workflow ▸」は、選択中の worktree に配信を待っている Root がある間、押せない。

# Assumptions

なし

# Invariants

なし

# Risks

なし
