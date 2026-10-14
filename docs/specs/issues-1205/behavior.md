## B-001: 右カラムの面の並び

GIVEN Swift.app で worktree を選んでいる
WHEN 利用者が右カラムを開く
THEN 面の切り替えは、Root の一覧、ファイル、Source Control、Thread の順に並ぶ
AND 「タスク」の面は無い

## B-002: Root の一覧の行

GIVEN 選んでいる worktree に、Session の Root、Command の Root、Workflow の Root（Sequence）と、Archive 済みの Session がある
WHEN 利用者が Root の一覧の面を開く
THEN Archive されていない Root が 1 行ずつ並び、Archive 済みの Session の行は無い
AND 各行は、Root の種類のアイコンを状態の色で塗ってタイトルを並べ、Session の行はタイトルの前に provider の表示名を出す

## B-003: 行からタブを開く

GIVEN Root の一覧に Session、Command、Sequence、Fanout の Root の行がある
WHEN 利用者が行を押す
THEN Session の行なら Session タブ、Command の行なら Command タブ、Sequence と Fanout の行なら Workflow 管理タブが開く

## B-004: New Session と New Workflow

GIVEN Root の一覧を開いている
WHEN 利用者が一覧の上端のメニューの New Session で provider を選ぶ、または New Workflow で定義を選ぶ
THEN 選んでいる worktree で Session または Workflow が開始する
AND 開始した Root について、一覧の行を押したときと同じタブが開く

## B-005: History の Archive 済みの Session

GIVEN 選んでいる worktree に Archive 済みの Session がある
WHEN 利用者が一覧の上端のメニューから History を開く
THEN Archive 済みの Session が並ぶ
AND その Session を選ぶと Restore され、Root の一覧に行が戻り、その Session タブが開く
AND Delete を選ぶと、その Session は削除され History から消える

## B-006: History の provider の履歴

GIVEN 選んでいる worktree に provider の履歴がある
WHEN 利用者が History を開いて provider の履歴を選ぶ
THEN その provider の Session が Resume され、Root の一覧に行が増え、その Session タブが開く
AND 履歴の続きがあるときは、続きを追加で読める

## B-007: 行の右クリックメニュー

GIVEN Root の一覧に行がある
WHEN 利用者が行を右クリックする
THEN 改名、Archive、Resume のうち、サーバがその Root に使えるとする項目だけが出る
AND Restore と Delete は出ない

## B-008: Archive

GIVEN Root の一覧に、実行が中止できる状態の Root と、中止できる状態でない Root がある
WHEN 利用者がそれぞれの行の右クリックメニューで Archive を選ぶ
THEN 中止できる状態でない Root は、確認なしで Archive され、一覧から消える
AND 中止できる状態の Root は、実行を中止して Archive することの確認が出る
AND 確認で了承すると、実行は中止され、その Root は Archive されて一覧から消える
AND 確認で取りやめると、実行は続き、その Root は一覧に残る

## B-009: 改名

GIVEN Session の Root の行と、その Session タブがある
WHEN 利用者が行の右クリックメニューで改名し、新しい名前を入れる
THEN 一覧の行と、その Session タブのタブ名が新しい名前になる

## B-010: 「+」の「開く」

GIVEN pane のタブ列を表示している
WHEN 利用者が「+」の「開く」を押す
THEN 項目は Terminal だけである

## B-011: タブの保存と復元

GIVEN worktree A で Session タブと Command タブを開いている
WHEN 利用者が別の worktree を選んでから A を選び直す、または Swift.app を起動し直して A を選ぶ
THEN A の Session タブと Command タブは、それぞれ前と同じ Node の内容を表示する

## B-012: タブを閉じる

GIVEN 動いている Session の Session タブと、実行中の Command の Command タブを開いている
WHEN 利用者がそれらのタブを閉じる
THEN 確認は出ずにタブが閉じる
AND Session と実行は動き続け、Root の一覧の行も残る

## B-013: Session タブの TUI

GIVEN 動いている Session の Root がある
WHEN 利用者がその行から Session タブを開き、文字を打つ
THEN Session タブに provider の TUI が表示される
AND 打った文字は provider の TUI に送られ、その応答が表示される

## B-014: 止まっている Session

GIVEN provider のプロセスが止まっていて、Resume できる Session がある
WHEN 利用者がその Session タブを開く
THEN Session タブは、TUI の代わりに、Session が止まっていることと Resume の操作を表示する
AND 利用者が Resume を押すと、Session が再開し、Session タブに TUI が表示される

## B-015: 判定できない Session と Archive 済みの Session

GIVEN Session タブを開いている
WHEN サーバが、その Session を開けるかどうかを判定できない状態、またはその Session が Archive された状態を配る
THEN Session タブは、TUI の代わりに、判定できないこと、または Archive されていることを表示する

## B-016: Command タブの COMMAND 行

GIVEN Root が Command の実行が終わっている
WHEN 利用者がその行から Command タブを開く
THEN COMMAND 行に、実行したコマンド、作業ディレクトリ、shell、終了コード、所要時間が表示される
AND 作業ディレクトリは、その Command を実行した worktree の path で、shell は Command を実行した shell である

## B-017: 出力と失敗理由

GIVEN Root が Command の実行が、stdout と stderr に出力して、0 以外の終了コードで失敗している
WHEN 利用者がその Command タブを開く
THEN stdout の内容と stderr の内容が分かれて表示される
AND 失敗理由が表示される

## B-018: Retry と Abort

GIVEN Command タブを開いている
WHEN サーバが、その Node に Retry できること、またはその実行が中止できる状態であることを配る
THEN Retry できるときは COMMAND 行に Retry が、中止できる状態のときは COMMAND 行に Abort が出る
AND Retry を押すと、その Command が再び実行される
AND Abort を押すと、その実行が中止される

## B-019: 過去の試行

GIVEN Root が Command の Node が Retry され、過去の試行がある
WHEN 利用者がその Command タブを開く
THEN 過去の試行が、時刻と失敗理由とともに並ぶ
AND 過去の試行を押すと、その試行の Command タブが開き、その試行の出力と終了コードが表示される

## B-020: サーバとの境界

GIVEN この開発で追加・変更したサーバの呼び出しと購読
WHEN その形と、Swift.app の処理を確かめる
THEN 呼び出しと購読は特定の client に固有の値を持たない
AND 状態は購読で配信され、単発の呼び出しは状態を変える操作か入力に対する計算である
AND Session を開けるかどうか、Command の shell と作業ディレクトリ、Root の一覧の provider、開始した Workflow の Root、Command タブの Abort の可否と対象は、サーバが配る値で決まり、Swift.app は lifecycle や terminalPresence からの判定や、別々に届いた値の突き合わせを行わない

## B-021: proto の互換性

GIVEN この開発の proto の変更
WHEN `buf.yaml` の breaking 検査を main に対して実行する
THEN 検査が通る

## B-022: 回帰の E2E

GIVEN Swift.app の回帰の E2E
WHEN `./nx run macos:e2e` を実行する
THEN Session タブに provider の TUI が表示され、入力できることを確かめるテストが含まれ、成功する
AND Root が Command の実行について、Command タブにコマンドの出力と終了コードが表示されることを確かめるテストが含まれ、成功する

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005, B-006 |
| R-006 | B-007 |
| R-007 | B-008 |
| R-008 | B-009 |
| R-009 | B-010 |
| R-010 | B-011 |
| R-011 | B-012 |
| R-012 | B-013 |
| R-013 | B-014, B-015 |
| R-014 | B-020 |
| R-015 | B-016, B-020 |
| R-016 | B-017 |
| R-017 | B-018 |
| R-018 | B-019 |
| R-019 | B-020 |
| R-020 | B-021 |
| R-021 | B-022 |
| R-022 | B-022 |
