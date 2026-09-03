## B-001: workflow 実行木の Session Node の表示名

GIVEN workflow 定義から起動された実行木に Session Node が含まれる
WHEN 利用者が Workspace ツリーを表示する
THEN その Session Node の行の表示名は、workflow 定義が与えた Node 名である

## B-002: workflow 実行木の Session Node と provider タイトル

GIVEN workflow 実行木に含まれる Session Node が手動で rename されていない
WHEN その Session Node に対応する provider のセッションタイトルが取得される
THEN その行の表示名は workflow 定義が与えた Node 名のままである
AND provider のセッションタイトルはその行に表示されない

## B-003: 単独 Session 実行木ルート行の既定表示名

GIVEN 単独 Session 実行木が起動され、その provider のセッションタイトルが未取得であり、手動で rename もされていない
WHEN 利用者が Workspace ツリーを表示する
THEN そのルート行の表示名は `session` である

## B-004: 単独 Session 実行木ルート行への provider タイトルの反映

GIVEN 単独 Session 実行木のルート行が手動で rename されていない
WHEN その Session の provider のセッションタイトルが取得される
THEN そのルート行の表示名は provider のセッションタイトルである
AND 既定値 `session` はその行に表示されない

## B-005: provider のセッションタイトルの供給源

GIVEN 活動中の AgentSession の provider が Claude または Codex である
WHEN その AgentSession の provider のセッションタイトルが取得される
THEN Claude では provider が当該 session の transcript に生成したタイトルが、その AgentSession の provider のセッションタイトルとして扱われる
AND Codex では provider が当該 thread に付けた名前が、その AgentSession の provider のセッションタイトルとして扱われる

## B-006: Session Node の手動 rename

GIVEN Session Node が Workspace ツリーに表示されている
WHEN 利用者がその Session Node の表示名を任意の名前へ変更する
THEN その行の表示名は利用者が指定した名前である
AND provider のセッションタイトルおよび既定値はその行に表示されない

## B-007: 手動 rename の対象外の行

GIVEN Workspace ツリーに Sequence 行、Fanout 行、および workflow 実行木のルート行が表示されている
WHEN 利用者がそれらの行の表示名を変更しようとする
THEN いずれの行にも表示名を変更する操作は提供されない

## B-008: 手動 rename した表示名の再起動後の維持

GIVEN Session Node の表示名が手動で rename されている
WHEN アプリケーションを再起動して Workspace ツリーを表示する
THEN その行の表示名は rename で指定された名前のままである

## B-009: 手動 rename と provider タイトルの更新

GIVEN Session Node の表示名が手動で rename されている
WHEN その Session の provider のセッションタイトルが新たに取得または更新される
THEN その行の表示名は rename で指定された名前のままである

## B-010: 手動 rename と provider 側のカスタム名

GIVEN Session Node に対応する provider session に provider 側のカスタム名が付いている
WHEN 利用者がその Session Node の表示名を手動で変更する
THEN その行の表示名に provider 側のカスタム名は現れない
AND provider 側に保持されている名前は変化しない

## B-011: 同名の Node が並ぶ場合の表示名

GIVEN Fanout または retry によって同じ Node 名の Node が複数並んでいる
WHEN 利用者が Workspace ツリーを表示する
THEN 並んだ各行の表示名は互いに同一である
AND 表示名に序数その他の区別は付かない

## B-012: provider session 終了後のタイトル取り込み

GIVEN AgentSession の provider session が終了している
WHEN provider 側でその session のセッションタイトルが変化する
THEN その AgentSession に対応する Session Node の表示名は変化しない

## B-013: paused / archived の AgentSession のタイトル取り込み

GIVEN AgentSession の lifecycle が `paused` または `archived` である
WHEN provider 側でその session のセッションタイトルが変化する
THEN その AgentSession に対応する Session Node の表示名は変化しない

## B-014: タイトル未取得時のタイトルの反映

GIVEN 活動中の AgentSession に対応する、手動で rename されていない単独 Session 実行木のルート行があり、provider のセッションタイトルが未取得である
WHEN provider がその session のセッションタイトルを生成し、その後の取り込みが到来する
THEN そのルート行の表示名は生成されたタイトルへ変わる

## B-015: タイトル取得済み時の更新の反映

GIVEN 活動中の AgentSession に対応する、手動で rename されていない単独 Session 実行木のルート行に provider のセッションタイトルが表示されている
WHEN provider がその session のセッションタイトルを更新し、その後の取り込みが到来する
THEN そのルート行の表示名は更新後のタイトルへ変わる

## B-016: タイトル読み取りの範囲

WHEN 活動中の AgentSession の provider のセッションタイトルを取り込むとき、Releash はその session の transcript 全体を走査しない。

## B-017: Provider history 行のタイトル表示

GIVEN Provider history の候補に provider のセッションタイトルがある
WHEN 利用者が Provider history 一覧を表示する
THEN その候補の行には provider のセッションタイトルが表示される
AND その行に provider 名と provider session id は表示されない

## B-018: Provider history 行のフォールバック表示

GIVEN Provider history の候補に provider のセッションタイトルが無く、最初のユーザープロンプトがある
WHEN 利用者が Provider history 一覧を表示する
THEN その候補の行には最初のユーザープロンプトの冒頭が、改行を含まない1行として表示される
AND その行に provider 名と provider session id は表示されない

GIVEN Provider history の候補に provider のセッションタイトルも最初のユーザープロンプトも無い
WHEN 利用者が Provider history 一覧を表示する
THEN その候補の行には provider 名と短縮された provider session id が表示される
AND provider session id が短縮されない形で表示されることはない

## B-019: bind される前の Session Node の rename

GIVEN Session Node が Workspace ツリーに表示されており、AgentSession がまだ bind されていない
WHEN 利用者がその行の表示名を変更しようとする
THEN その行に表示名を変更する操作は提供されない
AND AgentSession が bind された後は、同じ行に表示名を変更する操作が提供される

## B-020: bind される前の Session Node の状態表示

GIVEN workflow 実行木の Session Node が起動され、AgentSession がまだ bind されていない
WHEN 利用者が Workspace ツリーを表示する
THEN その行の状態表示は、bind 後の状態表示および他のどの状態表示とも区別できる
AND AgentSession が bind された後は、その行の状態表示は bind 前とは異なる

## B-021: bind 待ちの Node が並ぶ Sequence 行と Fanout 行の状態表示

GIVEN Sequence 行または Fanout 行の子に、bind 待ちの状態に分類された Session Node とそれ以外の状態の Node が並んでいる
WHEN 利用者が Workspace ツリーを表示する
THEN bind 待ちの状態に分類された Session Node は、その親行の状態を決める集約対象の子から除外される
AND bind 前に終了して Failure または Idle に分類された Session Node は除外されない
AND 除外後に子が残る場合は、親自身と残った子に既存の集約規則が適用される
AND 除外後に集約対象の子が残らない場合に限り、その行は bind 前の状態表示になる

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-002, B-004, B-006 |
| R-002 | B-001, B-002 |
| R-003 | B-004 |
| R-004 | B-003 |
| R-005 | B-011 |
| R-006 | B-006 |
| R-007 | B-007 |
| R-008 | B-008, B-009 |
| R-009 | B-010 |
| R-010 | B-005 |
| R-011 | B-012, B-013 |
| R-012 | B-014, B-015 |
| R-013 | B-016 |
| R-014 | B-017, B-018 |
| R-015 | B-019 |
| R-016 | B-020 |
| R-017 | B-021 |
