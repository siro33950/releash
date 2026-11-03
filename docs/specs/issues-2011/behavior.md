## B-001: 定義と Facet のタブを開く

GIVEN Swift.app がサーバに接続し、Settings が開いている
WHEN 開発者が定義と Facet のタブを選ぶ
THEN 定義の一覧が表示され、Facet の一覧へ切り替えられる

## B-002: 定義の一覧の表示

GIVEN 定義のフォルダに利用者の定義があり、そのうち一つに error の診断がある
WHEN 開発者が定義の一覧を見る
THEN builtin の定義と利用者の定義の両方が並ぶ
AND 各定義に名前、説明、builtin であるか、実行中であるか、形式（yaml / lua）が表示される
AND error の診断がある定義には error の件数が表示される

## B-003: 読み取りエラーのある定義の表示

GIVEN 定義のフォルダに読み取りエラーのある定義がある
WHEN 開発者が定義の一覧を見る
THEN その定義の行に読み取りエラーの内容が表示される

## B-004: 定義の詳細の表示

GIVEN 定義の一覧が表示されている
WHEN 開発者が解釈できる定義を選ぶ
THEN その定義の説明と、その定義に対する診断の一覧が表示される
AND Node の一覧が表示され、各 Node の名前と kind と、定義が持つ内容（command、session の provider / model / permission、参照する Facet を枠ごとに、Fanout と Sequence の子 Node の名前、artifact、input、completion、worktree）が見られる

## B-005: 解釈できない定義の詳細

GIVEN 定義の一覧に解釈できない定義がある
WHEN 開発者がその定義を選ぶ
THEN その定義に対する診断の一覧だけが表示される

## B-006: Facet の一覧の表示

GIVEN 定義のフォルダに利用者の Facet があり、そのうち一つに error の診断がある
WHEN 開発者が Facet の一覧で種類（policy / knowledge / instruction）を切り替える
THEN 選んだ種類の builtin の Facet と利用者の Facet の両方が並ぶ
AND 各 Facet に key、説明、builtin であるかが表示される
AND error の診断がある Facet には error の件数が表示される

## B-007: Facet の詳細の表示

GIVEN 二つの定義の Node が同じ Facet を参照している
WHEN 開発者がその Facet を選ぶ
THEN その Facet の key、種類、説明と、その Facet に対する診断の一覧が表示される
AND 参照元の一覧に、二つの定義それぞれの定義名、Node 名、枠が表示される
AND 本文は表示されない

## B-022: Facet の本文に誤りがあるときの参照元の表示

GIVEN builtin の定義が参照する Facet の本文に、参照検証に失敗する誤りがある
WHEN 開発者がその Facet を選ぶ
THEN 参照元の一覧に、その builtin 定義の定義名、Node 名、枠が表示される
AND その参照元を選ぶと、その定義の詳細へ移れる

## B-009: 診断の項目の表示

GIVEN 選んだ定義または Facet に、位置を持つ診断と位置を持たない診断がある
WHEN 開発者が診断の一覧を見る
THEN 各項目に code、severity（error / info）、stage、message が表示される
AND 位置を持つ項目にはソース名と行・列が表示され、位置を持たない項目には位置が表示されない

## B-010: 定義を外部エディタで開く

GIVEN 外部エディタが選択され、利用者の定義が選ばれている
WHEN 開発者がその定義を外部エディタで開く
THEN サーバが選択中の外部エディタを、定義のフォルダとその定義のファイルの 2 つのパスを渡して起動する

## B-011: Facet を外部エディタで開く

GIVEN 外部エディタが選択され、利用者の Facet が選ばれている
WHEN 開発者がその Facet を外部エディタで開く
THEN サーバが選択中の外部エディタを、定義のフォルダとその Facet のファイルの 2 つのパスを渡して起動する

## B-012: builtin を外部エディタで開く操作

GIVEN builtin の定義または Facet が選ばれている
WHEN 開発者が詳細を見る
THEN 外部エディタで開く操作が無い

## B-013: builtin を外部エディタで開く要求の拒否

GIVEN サーバが起動している
WHEN クライアントが builtin の定義または Facet を外部エディタで開く RPC を呼ぶ
THEN サーバはエディタを起動せず、builtin は開けない旨の理由を返す

## B-014: 定義のフォルダを外部エディタで開く

GIVEN 外部エディタが選択され、定義のフォルダが存在する
WHEN 開発者が定義のフォルダを外部エディタで開く
THEN サーバが選択中の外部エディタを、定義のフォルダのパスを渡して起動する

## B-015: 存在しない定義のフォルダを開く

GIVEN 外部エディタが選択され、定義のフォルダが存在しない
WHEN 開発者が定義のフォルダを外部エディタで開く
THEN サーバが定義のフォルダを作り、選択中の外部エディタをそのパスを渡して起動する

## B-016: 外部での変更への追従

GIVEN 定義と Facet のタブで、ある定義の詳細が表示されている
WHEN その定義のファイルが外部で書き換えられて保存される
THEN 一覧と詳細と診断の表示が、書き換え後の内容に変わる

## B-017: 既存のクライアントからの呼び出し

GIVEN React の画面がサーバに接続している
WHEN 開発者が React の画面から利用者の定義または Facet を外部エディタで開く
THEN サーバが選択中の外部エディタを、定義のフォルダとそのファイルの 2 つのパスを渡して起動する
AND React の画面は変更前と同じ引数で RPC を呼んでいる

## B-018: Node から Facet への移動

GIVEN 定義の詳細が表示され、ある Node が Facet を参照している
WHEN 開発者がその Node の参照する Facet を選ぶ
THEN その Facet の詳細が表示される

## B-019: Facet から定義への移動

GIVEN Facet の詳細が表示され、参照元の一覧に定義がある
WHEN 開発者がその参照元を選ぶ
THEN その定義の詳細が表示される

## B-020: builtin と同名の利用者ファイルの表示

GIVEN builtin と同名の定義または Facet が利用者のファイルから取得されている
WHEN 開発者が一覧または詳細を見る
THEN 利用者のものとして表示され、builtin の印は表示されない

## B-021: builtin と同名の利用者ファイルへの操作

GIVEN builtin と同名の定義または Facet が利用者のファイルから取得されている
WHEN 開発者が外部編集・保存・削除のいずれかを要求する
THEN builtin と同名であることだけを理由に操作は拒否されない
AND 取得元とは別の規則による検証は適用される

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002、B-003 |
| R-003 | B-004、B-005、B-018 |
| R-004 | B-006 |
| R-005 | B-007、B-019、B-022 |
| R-006 | B-009 |
| R-007 | B-010、B-011、B-012、B-013 |
| R-008 | B-014、B-015 |
| R-009 | B-016 |
| R-010 | B-017 |
| R-011 | B-020、B-021 |
