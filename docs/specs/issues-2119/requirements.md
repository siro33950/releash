# Context

- 要求の正本は、ISSUE #2119「Swift.app: provider のプロセスが止まった Session のタブが自動で閉じない」の本文である。
- 調査の時点は main `108316f1` である。
- この文書で「Session が止まる」とは、provider の process が終了し、Session が停止の状態へ移ることを指す。停止の状態にある Session のタブは「Sessionは停止しています」と表示する。

# Outcome

対象者は、Swift.app で Workflow と Session を動かす開発者である。

今は、Session が止まっても、その Session のタブが中央のカラムに残る。
Workflow を回すたびに、終わった Node の Session のタブがたまり、開発者はタブを一つずつ手で閉じる。
タブの並びは Swift.app を再起動しても元に戻るので、閉じ忘れたタブは再起動のあとも残る。

変更後は、Session が止まると、その Session のタブが自動で閉じる。
止まった Session の process が再び動くと、その Session のタブが自動で開く。

# Current Behavior

調査は main `108316f1` のコードを読んで行った。
あわせて、動いている Releash（release 0.4.21）が保存した記録を読み、Node が終わったあともタブ配置が変わらないことを確かめた。
アプリの画面を見ることはしていない。

## Session が止まったときのタブ

- Session が止まっても、その Session のタブは中央のカラムに残る。Workflow の Node が終わったとき、実行を Abort したとき、provider が自分で終了したときのどれでも同じである。
- 止まった Session のタブは「Sessionは停止しています」と表示する。その Session を再開できる場合は、Resume ボタンも表示する。
- Workflow に属さない Session で、provider が会話を始める前に終了すると、その Session は止まらずに Node 面から消える。このときも、その Session のタブは中央のカラムに残る。

## Archive したときのタブ

- 実行を Archive すると、その実行の process が止まり、その実行に属する Session のタブが中央のカラムからすべて閉じる。止まったあとに開発者が開き直したタブも閉じる。

## タブが開く経路

- Session を新しく開始すると、その Session のタブが中央のカラムに開く。
- 開発者が Node 面の ▷ を押すと、その Session のタブが中央のカラムに開く。Session が止まっていても開ける。止まった Session のタブを開いても、process は起動しない。
- 止まった Session の process が再び動く経路は、次の 3 つである。
  - 開発者が Node 面の右クリックメニューで Resume を選ぶ。
  - 開発者がタブの中の Resume ボタンを押す。
  - Workflow が、delegate の child の結果を親の Session へ渡すために、止まっている親の Session を起動し直す。
- どの経路でも、タブは新しく開かない。今はタブが残っているので、再開した Session の端末はそのタブに表示される。

## 再現手順

動いている Releash（release 0.4.21）で、Workflow を実行して確かめた。
この版は、調査の時点の main より古い。
確かめた対象は Releash が保存した記録であり、アプリの画面ではない。

1. worktree `feat/issues/2119` で、Session の Node `spec` と `spec_check_reviewer` を持つ Workflow を実行する。
2. `spec` が動いているあいだに、`spec_check_reviewer` が 2 回実行される。どちらの回も、Node は Artifact を提出して終わり、provider の process が終了する。
3. 保存されたタブ配置を読む。

実際の出力は次のとおりである。時刻は、1 回目の `spec_check_reviewer` が始まった時点からの経過である。

| 経過 | 記録 |
| --- | --- |
| 0.0 秒 | 1 回目の `spec_check_reviewer` が始まる |
| 22.0 秒 | 1 回目の `spec_check_reviewer` の process が終了する |
| 48.7 秒 | 2 回目の `spec_check_reviewer` が始まる。タブ配置が保存される |
| 71.2 秒 | 2 回目の `spec_check_reviewer` の process が終了する |

48.7 秒に保存されたタブ配置は、Session のタブを 3 つ持つ。
`spec` の Session のタブ、1 回目の `spec_check_reviewer` の Session のタブ、2 回目の `spec_check_reviewer` の Session のタブである。
1 回目の `spec_check_reviewer` の process は 22.0 秒に終了しているが、そのタブはタブ配置に残っている。
71.2 秒より後に保存されたタブ配置は無い。
2 回目の `spec_check_reviewer` のタブも、process が終了したあとタブ配置に残っている。

# Scope / Non-goals

変更するもの:

- Session が止まったときのタブ配置。その Session のタブを閉じる。
- 止まった Session の process が再び動いたときのタブ配置。その Session のタブを開く。

Non-goals:

なし

# Requirements

- R-001: Session が止まると、その Session のタブは中央のカラムから閉じる。Workflow の Node が終わったとき、実行を Abort したとき、provider が自分で終了したとき（`/exit`、異常終了）のどれでも閉じる。
- R-002: provider が自分で終了した結果、Session が Node 面から消える場合も、その Session のタブは閉じる。
- R-003: 実行を Archive すると、その実行に属する Session のタブは閉じる。止まったあとに開発者が開き直したタブも閉じる。
- R-004: 開発者は、止まった Session のタブを Node 面の ▷ で開き直せる。開き直したタブは、Session が止まっているあいだも閉じない。そのタブは「Sessionは停止しています」と表示し、再開できる Session では Resume ボタンも表示する。
- R-005: 止まった Session の process が再び動くと、その Session のタブが中央のカラムに開く。開発者が Node 面の右クリックメニューで Resume を選んだときと、Workflow が止まっている Session を起動し直したときのどちらでも開く。
- R-006: R-005 のとき、その Session のタブが既に開いていれば、タブは増えない。
- R-007: R-001、R-002、R-003 で閉じたタブは、Swift.app を再起動しても開かない。

# Assumptions

なし

# Invariants

なし

# Risks

なし
