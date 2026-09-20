## B-001: 終了時に行う処理

GIVEN workflow が起動した command が実行中であり、AgentSession の provider CLI を含むターミナルが開いている
WHEN 利用者の終了操作、更新を伴わない再起動、または更新の適用によって Releash が終了する
THEN command のプロセスは停止する
AND ターミナルの状態が保存される
AND 終了のためにターミナルを止めたことは、AgentSession のプロセス終了として記録されない
AND Releash のプロセスが終了する
AND 終了手続きの記録、止める対象ごとの進捗、終了要求の呼び出し記録は保存されない

## B-002: 段階の失敗

GIVEN 終了処理の途中で、command の停止またはターミナルの状態の保存が失敗する
WHEN 終了処理が進む
THEN 失敗がログに出力される
AND 残りの段階が行われる
AND Releash のプロセスが終了する

## B-003: 時間の上限による打ち切り

GIVEN 終了処理が 15 秒以内に終わらない
WHEN 終了処理の開始から 15 秒が経過する
THEN 残りの処理は打ち切られ、Releash のプロセスが終了する

## B-004: 途中で途切れた終了の後の起動

GIVEN 前回の終了処理が途中で途切れた
WHEN Releash を起動して Session を open または新規起動する
THEN 操作は前回の終了を理由に拒否されない
AND 利用者の操作や store の直接操作を必要としない

## B-005: 未完了の記録が残った store の継続利用

GIVEN この変更より前の版で終了処理が途中で途切れ、未完了の終了手続きの記録が残った store がある
WHEN この変更を含む版で Releash を起動して Session を open または新規起動する
THEN 操作は前回の終了を理由に拒否されない
AND 利用者の操作や store の直接操作を必要としない

## B-006: 終了処理に関する画面と分岐が無い

GIVEN 前回の終了処理が途中で途切れた、または未完了の終了手続きの記録が残った store がある
WHEN Releash を起動し、その後に終了・再起動・更新を要求する
THEN `Retry same effect`、`Retry quit`、outcome unknown の表示は現れない
AND `Previous shutdown requires a decision before switching.` は表示されず、終了・再起動・更新は前回の終了の解決を求めて止まらない

## B-007: workflow の状態は次回起動時に記録される

GIVEN workflow の Session または Command の Node がプロセスを起動して実行中である
WHEN Releash が終了し、その後に起動する
THEN 終了処理はその NodeExecution の状態を記録しない
AND 起動時に既存の処理が、その NodeExecution にプロセスの喪失を記録する

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-001 |
| R-005 | B-004, B-005 |
| R-006 | B-006 |
| R-007 | B-007 |
