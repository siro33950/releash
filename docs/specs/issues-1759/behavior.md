## B-001: Terminal タブを開く

GIVEN worktree を選択し、pane のタブ列を表示している
WHEN 利用者が「+」の「開く」から Terminal を選ぶ
THEN Terminal タブが開き、shell のプロンプトが表示される
AND その shell の作業ディレクトリは、選択中の worktree の path である

## B-002: タブごとに別の shell

GIVEN 同じ worktree に Terminal タブを 2 つ開いた
WHEN 利用者が一方のタブでコマンドを打つ
THEN そのコマンドと出力は打ったタブにだけ表示され、もう一方のタブには表示されない
AND 2 つのタブは、タブ名で区別できる

## B-003: 帯の作業ディレクトリと branch

GIVEN branch `feat/login` の worktree で Terminal タブを開いた
WHEN 利用者が shell で別のディレクトリへ移る
THEN 帯の作業ディレクトリは、移った先のディレクトリに変わる
AND 帯の branch は `feat/login` のままである

## B-004: 入力と出力

GIVEN Terminal タブを開いた
WHEN 利用者がコマンドを打つ
THEN コマンドは shell に送られ、その出力がタブに表示される

## B-005: resize

GIVEN Terminal タブを開いた
WHEN 利用者がウィンドウまたは pane の大きさを変える
THEN shell の端末の行数と列数が、タブの表示領域に合わせて変わる

## B-006: 大量の出力

GIVEN Terminal タブを開いた
WHEN shell が、サーバが出力を止める未報告の量を大きく超える量の出力を続ける
THEN 出力は止まらずに最後までタブに表示される

## B-007: 購読の開始前の入力

GIVEN Terminal タブを開いた直後で、購読がまだ始まっていない
WHEN 利用者が文字を打つ
THEN 購読の開始後に、打った文字が打った順で shell に送られる

## B-008: 接続断からの回復

GIVEN Terminal タブを開いている
WHEN サーバとの接続が切れ、切断中に shell が出力し、その後接続が回復する
THEN タブには切断中の出力を含む shell の今の画面が表示される
AND 回復後に打った入力は shell に送られる

## B-009: 選び直しと起動し直し

GIVEN worktree A で Terminal タブを開き、コマンドを実行した
WHEN 利用者が別の worktree を選んでから A を選び直す、または Swift.app を起動し直して A を選ぶ
THEN A の Terminal タブは同じ shell に繋がり、それまでの出力を表示する

## B-010: サーバを起動し直した後の復元

GIVEN worktree A で Terminal タブを開き、コマンドを実行した後に、サーバが起動し直した
WHEN 利用者が A を選ぶ
THEN A の Terminal タブには、サーバが保存した画面が表示される

## B-011: 処理の無いタブを閉じる

GIVEN Terminal タブの shell の前面で、shell 以外の処理が動いていない
WHEN 利用者がそのタブを閉じる
THEN 確認は出ずにタブが閉じ、その shell は止まる

## B-012: 処理の動いているタブを閉じる

GIVEN Terminal タブの shell の前面で、shell 以外の処理が動いている
WHEN 利用者がそのタブを閉じる
THEN 閉じる前に確認が出る
AND 確認で閉じることを選ぶと、タブが閉じ、その shell は止まる
AND 確認で取りやめると、タブは残り、shell と処理は動き続ける

## B-013: リンク、Clear、終了の表示

GIVEN Terminal タブを開いた
WHEN shell の出力に URL が含まれ、利用者がそのリンクを開く
THEN そのリンクが開く
AND 利用者が Clear を選ぶと、タブの表示が消去される
AND shell が終了すると、終了したことと終了コードがタブに表示される

## B-014: アクセシビリティ

GIVEN Terminal タブにコマンドの出力が表示されている
WHEN アクセシビリティでタブの画面の文字を読む
THEN 表示されている出力の文字が読める

## B-015: 起動コマンド

GIVEN サーバの設定に起動コマンドが保存されている
WHEN Terminal タブを開いて新しい shell が起動する
THEN その shell で起動コマンドが実行される
AND サーバが保存した画面から復元した shell では、起動コマンドは実行されない

## B-016: 起動コマンドの変更

GIVEN React の設定画面を開いている
WHEN 利用者が起動コマンドの欄を変えて保存する
THEN 値はサーバの設定に保存され、起動コマンドの購読に新しい値が配信される
AND 設定画面を開き直すと、欄にはサーバの設定の値が表示される

## B-017: サーバとの境界

GIVEN この開発で追加・変更したサーバの呼び出しと購読
WHEN その形と、Swift.app と React の処理を確かめる
THEN 呼び出しと購読は特定の client に固有の値を持たない
AND 状態は購読で配信され、単発の呼び出しは状態を変える操作か入力に対する計算である
AND terminal を起こす要求は起動コマンドを持たない
AND shell の作業ディレクトリと、shell 以外の処理が動いているかは、client ではなくサーバが求める

## B-018: proto の互換性

GIVEN この開発の proto の変更
WHEN `buf.yaml` の breaking 検査を main に対して実行する
THEN 検査が通る

## B-019: 回帰の E2E

GIVEN Swift.app の回帰の E2E
WHEN `./nx run macos:e2e` を実行する
THEN Terminal タブを開き、打ったコマンドの出力が表示されることを確かめるテストが含まれ、成功する

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
| R-008 | B-008 |
| R-009 | B-009, B-010 |
| R-010 | B-011, B-012 |
| R-011 | B-013 |
| R-012 | B-014 |
| R-013 | B-015, B-017 |
| R-014 | B-016 |
| R-015 | B-017 |
| R-016 | B-018 |
| R-017 | B-019 |
