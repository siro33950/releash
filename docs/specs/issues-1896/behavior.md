## B-001: 再接続をまたいで画面が残る

GIVEN 画面が出ていて、worktree のタブを開き、選択している
WHEN 画面の接続が切れてつながり直す、またはシェルの phase が Starting・Backoff を経て Ready に戻る
THEN 画面は作り直されず、開いていたタブと選択はそのまま残る

## B-002: 接続が切れている間は「再接続中」と表示する

GIVEN 画面の接続状態が一度 READY になった
WHEN 画面の接続状態が READY でなくなる
THEN 画面を残したまま「再接続中」と表示される
AND 画面の接続状態が READY に戻ると、この表示は消える

## B-003: 最初の接続では「再接続中」を出さない

GIVEN 画面の接続状態が一度も READY になっていない
WHEN 画面の接続状態が CONNECTING である
THEN 「再接続中」は表示されない

## B-004: シェルの phase だけでは「再接続中」を出さない

GIVEN 画面が出ていて、画面の接続状態が READY である
WHEN シェルの phase が Starting または Backoff になる
THEN 「再接続中」は表示されず、画面を覆う表示も出ない
AND 画面の操作はそのまま受け付けられる

## B-005: Failed では覆いを出し Retry と Quit を選べる

GIVEN 画面が出ている
WHEN シェルの phase が Failed になる
THEN 画面を覆う表示が出て、Quit を選べる
AND シェルが Retry できると示しているときは、Retry も選べる
AND 覆いの下の画面は unmount されない

## B-006: 終了と更新の途中は覆いを出す

GIVEN 画面が出ている
WHEN シェルの phase が Stopping、Installing、Stopped のいずれかになる
THEN 画面を覆う表示が出る

## B-007: つながり直すと購読を版から再開する

GIVEN 購読している表示があり、最後に受け取った版がある
WHEN 画面の接続が切れて、READY に戻る
THEN その購読は、最後に受け取った版を付けて開始し直される

## B-008: つながったシェルは復元を待たずに Ready になる

GIVEN シェルの phase が Starting である
WHEN シェルが daemon とつながる
THEN シェルの phase は、画面からの復元の完了を待たずに Ready になる
AND 画面は復元の完了も失敗もシェルに送らない

## B-009: 起動時の処理は最初の 1 回だけ動く

GIVEN 画面を最初に出したときに、cwd の Repository の登録、worktree のタブを開く処理、自動の更新の確認が 1 回ずつ動いた
WHEN 画面の接続が切れてつながり直す、またはシェルの phase が Ready 以外を経て Ready に戻る
THEN これらの処理はもう一度は動かない
AND 利用者が閉じたタブは開き直されない

## B-013: 自動更新を ON にすると更新を確認する

GIVEN 画面が出ていて、自動更新が OFF で、まだ更新を確認していない
WHEN 設定画面で自動更新を ON にする
THEN 更新が確認される

## B-014: 確認が失敗した後に ON にすると確認し直す

GIVEN 画面が出ていて、更新の確認が失敗した
WHEN 設定画面で自動更新を OFF にしてから ON にする
THEN 更新がもう一度確認される

## B-010: Ready に戻るとメニューの有効・無効を合わせ直す

GIVEN 画面が出ていて、シェルの phase が Ready でない間に worktree の選択が変わった
WHEN シェルの phase が Ready に戻る
THEN メニューの有効・無効は、そのときの worktree の選択に合っている

## B-011: 設定を読めていない間は performance metrics を保存しない

GIVEN `desktop-settings` を読めていない
WHEN 設定画面で performance metrics の設定を変えて Save する
THEN performance metrics の設定は daemon に書き込まれない

## B-012: 設定の読み込みの失敗を performance metrics の場所に出す

GIVEN `desktop-settings` の読み込みが失敗している
WHEN 設定画面を開く
THEN performance metrics の設定の場所に、読み込みの失敗が表示される

## B-015: performance metrics の書き込みの失敗をその欄に出し、変えた値を残す

GIVEN `desktop-settings` を読めていて、設定画面で performance metrics の設定を変えた
WHEN Save して、performance metrics の daemon への書き込みが失敗する
THEN performance metrics の設定の場所に、書き込みの失敗が表示される
AND 変えた値は、保存されていないものとして設定画面に残る

## B-016: 他の欄の保存の失敗の後に Save し直すと performance metrics を書き込む

GIVEN `desktop-settings` を読めていて、設定画面で performance metrics の設定と他の設定を変えた
WHEN Save して他の設定の保存が失敗し、performance metrics の書き込みまで届かなかった後に、もう一度 Save する
THEN performance metrics の設定は、変えた値で daemon に書き込まれる

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-005 |
| R-002 | B-002, B-003, B-004 |
| R-003 | B-004 |
| R-004 | B-005 |
| R-005 | B-006 |
| R-006 | B-007 |
| R-007 | B-008 |
| R-008 | B-009, B-013, B-014 |
| R-009 | B-010 |
| R-010 | B-011, B-012, B-015, B-016 |
