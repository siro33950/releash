## B-001: Thread の面の一覧

GIVEN worktree に、ファイルの行に付いた Thread と、ファイルに付かない Thread がある
WHEN 利用者が右カラムの「Thread」の面を開く
THEN 各 Thread が 1 行ずつ出る
AND 行に付いた Thread の行には、ファイルの path と行の範囲が出る
AND ファイルに付かない Thread の行には `General` が出る
AND 各行に、本文（最初の Comment の文）、Comment の数、最後の Comment の発言者と時刻が出る

## B-002: 未解決と解決済みの切替

GIVEN worktree に未解決の Thread と解決済みの Thread がある
WHEN 利用者が「Thread」の面で解決済みに切り替える
THEN 解決済みの Thread だけが出て、各行に解決済みの印が出る
AND 見出しに未解決の件数と解決済みの件数が出る

## B-003: 返信

GIVEN 「Thread」の面に未解決の Thread がある
WHEN 利用者がその行から返信する
THEN その Thread に返信が Comment として加わり、行の Comment の数が増える

## B-004: 解決

GIVEN 「Thread」の面に未解決の Thread がある
WHEN 利用者がその行で解決する
THEN その Thread は未解決の側から消え、解決済みの側に出る

## B-005: markdown の整形表示

GIVEN 本文に見出し、箇条書き、コード、強調を含む Thread がある
WHEN 利用者が「Thread」の面でその本文を見る
THEN 見出し、箇条書き、コード、強調が整形されて出る

## B-006: General の Thread の作成

GIVEN 利用者が「Thread」の面を開いている
WHEN 利用者が「General に追加」から本文を入れて作る
THEN ファイルに付かない Thread が未解決の側に出る

## B-007: チェックと件数

GIVEN 「Thread」の面に未解決の Thread と解決済みの Thread がある
WHEN 利用者が未解決の Thread と解決済みの Thread を一つずつチェックする
THEN 送るボタンにチェックした件数 2 が出る

## B-008: 送り先の Session の指定

GIVEN 選択中の worktree に、生きている Session と、停止している Session がある
WHEN 利用者が送るボタンを押す
THEN 生きている Session だけが一覧に出て、利用者はその中から一つを指定できる

## B-009: 生きている Session が無いとき

GIVEN 選択中の worktree に生きている Session が無い
WHEN 利用者が「Thread」の面を見る
THEN 送るボタンから Session を指定できない

## B-010: Session の terminal への送信

GIVEN 利用者が Thread をチェックし、生きている Session を指定した
WHEN 利用者が送る
THEN その Session の terminal に 1 つのプロンプトが書き込まれ、Session の TUI は 1 回の入力として受ける
AND プロンプトは、冒頭の一文、チェックした Thread ごとの位置、ID、本文、それ以降の Comment を `releash review get --session-id "$RELEASH_SESSION_ID" <id>` で確認するよう書いた一文からなる
AND プロンプトに返信や解決の指示は無い

## B-011: 送信の API

GIVEN 生きている Session と、その worktree の Thread がある
WHEN クライアントが Thread の ID の列とその Session の ID を指定して送信を呼ぶ
THEN その Session の terminal に B-010 のプロンプトが書き込まれる

## B-012: 送信の拒否

GIVEN 生きていない Session、または指定した worktree に無い Thread の ID がある
WHEN クライアントがそれらを指定して送信を呼ぶ
THEN サーバは送信を拒み、terminal には何も書き込まれない

## B-013: 1 Thread 分の引き渡し文

GIVEN worktree に Thread がある
WHEN クライアントがその Thread の引き渡し文を求める
THEN 返る文は B-010 のプロンプトと同じ形で、Thread は 1 件である

## B-014: 変化の反映

GIVEN 利用者が「Thread」の面を開いている
WHEN 別の経路（ファイルタブや CLI）で Thread の作成、返信、解決、削除が起きる
THEN 利用者の操作なしに、その変化が面に出る

## B-015: 回帰の E2E

GIVEN Thread と生きている Session を持つ worktree がある
WHEN 回帰の E2E を実行する
THEN 「Thread」の面に Thread が出て返信と解決ができることと、チェックした Thread を Session の terminal に送れることが確かめられる

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003, B-004 |
| R-004 | B-005 |
| R-005 | B-006 |
| R-006 | B-007 |
| R-007 | B-008, B-009 |
| R-008 | B-010 |
| R-009 | B-011, B-012 |
| R-010 | B-013 |
| R-011 | B-014 |
| R-012 | B-015 |
