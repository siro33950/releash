## B-001: 指定した directory に対する CLI 診断の実行

GIVEN Releash アプリが起動している
AND 利用者が指定する directory に workflow 定義が置かれている
WHEN 利用者が CLI の workflow diagnostics command にその directory を指定して実行する
THEN その directory を対象とした診断結果が出力される
AND 対象 directory が repository 内の custom workflow directory であっても診断結果が出力される

## B-002: 適用済み config directory に対する CLI 診断の実行

GIVEN Releash アプリが起動している
AND 適用済み Workflow の config directory に workflow 定義が置かれている
WHEN 利用者が CLI の workflow diagnostics command で適用済み Workflow の config directory を診断対象として実行する
THEN その config directory を対象とした診断結果が出力される

## B-003: 指定 directory を workflow source と Facet base の双方として扱う

GIVEN Releash アプリが起動している
AND 指定した directory に workflow source が置かれ、同じ directory 配下に Facet が置かれている
WHEN 利用者が CLI の workflow diagnostics command にその directory を指定して実行する
THEN 診断結果には、その directory 上の workflow source に対する diagnostic が含まれる
AND 診断結果には、その directory 配下の Facet に対する diagnostic が含まれる
AND workflow 定義が参照する Facet key は、その directory 配下の Facet を対象として解決される

## B-004: config directory へ未適用の workflow 定義の診断

GIVEN Releash アプリが起動している
AND 指定した directory 上の workflow 定義と Facet が、適用済み Workflow の config directory へ適用されていない
WHEN 利用者が CLI の workflow diagnostics command にその directory を指定して実行する
THEN 適用済み Workflow の config directory の内容ではなく、指定した directory 上の workflow 定義と Facet に対する診断結果が出力される

## B-005: UI 経路と CLI 経路の診断結果の一致

GIVEN Releash アプリが起動している
AND 同一の対象 directory に workflow 定義と Facet が置かれている
WHEN 同じ対象 directory に対して UI から診断を実行し、CLI からも診断を実行する
THEN 双方の診断結果は、検出された diagnostic の code が一致する
AND 双方の診断結果は、各 diagnostic の severity、message、発生位置（対象 source、開始行、開始列、終了行、終了列）が一致する
AND 双方の診断結果は、diagnostic の並び順が一致する
AND 双方の診断結果は、workflow ごとの集計、Facet ごとの集計、Facet の参照元情報が一致する

## B-006: JSON 出力の内容

GIVEN Releash アプリが起動している
AND 診断対象 directory が指定されている
WHEN 利用者が CLI の workflow diagnostics command を JSON 出力を選んで実行する
THEN 出力される JSON は、UI が受け取る診断結果と同じ構造（診断項目の一覧、workflow ごとの集計、Facet ごとの集計、Facet の参照元情報）を持つ
AND 各診断項目は UI が受け取るものと同じ field（code、severity、stage、発生位置、message、対象 workflow 名、対象 node 名、対象 Facet key、対象 Facet 種別、対象 field）を持つ
AND CLI でのみ付与される field は追加されない
AND UI が受け取る field の削除や入れ子構造の再構成は行われない

## B-007: JSON 以外の出力形式

GIVEN Releash アプリが起動している
AND 診断対象 directory が指定されている
WHEN 利用者が CLI の workflow diagnostics command を JSON 出力を選ばずに実行する
THEN 診断結果が JSON ではない人間が読める形式で出力される
AND 検出された diagnostic がその出力に含まれる

## B-008: severity error を検出したときの終了コード

GIVEN Releash アプリが起動している
AND 診断対象 directory に、severity が error の diagnostic を 1 件以上生じさせる workflow 定義または Facet が置かれている
WHEN 利用者が CLI の workflow diagnostics command をその directory に対して実行する
THEN CLI process は non-zero の終了コードで終了する

## B-009: severity error が無いときの終了コード

GIVEN Releash アプリが起動している
AND 診断対象 directory に対する診断で severity が error の diagnostic が 1 件も生じない（severity が info の diagnostic だけが生じる場合、および diagnostic が 1 件も生じない場合を含む）
WHEN 利用者が CLI の workflow diagnostics command をその directory に対して実行する
THEN CLI process は終了コード 0 で終了する

## B-010: --help の記載内容

GIVEN CLI に workflow diagnostics command が存在する
WHEN 利用者がその command の `--help` を表示する
THEN 診断対象 directory の指定方法が記載されている
AND 終了コードの意味が記載されている
AND 出力形式が記載されている

## B-011: 既存 diagnostic 規則と UI 経路の結果の維持

GIVEN 本変更の前に、ある対象 directory に対して UI 経路で得られていた診断結果がある
WHEN 本変更の後に、同じ対象 directory に対して UI から診断を実行する
THEN 検出される diagnostic の code、severity、message は本変更の前と同一である
AND UI が受け取る診断結果の構造と field は本変更の前と同一である

## B-012: Releash アプリが起動していないときの CLI 診断

GIVEN Releash アプリが起動しておらず local API へ到達できない
WHEN 利用者が CLI の workflow diagnostics command を実行する
THEN 診断結果は出力されない
AND local API の起動を要する既存 CLI command と同じ失敗表示になる
AND local API の起動を要する既存 CLI command の失敗時と同じ終了コードで終了する

## B-013: 指定 directory を対象にしたときの診断範囲

GIVEN Releash アプリが起動している
AND 指定した directory 配下に workflow 定義が置かれている
WHEN 利用者が CLI の workflow diagnostics command にその directory を指定して実行する
THEN 診断結果には、指定した directory 配下の workflow 定義に対する diagnostic が含まれる
AND 指定した directory 配下の workflow が参照する Facet は、その実体が指定 directory 配下に無くても判定対象に含まれる
AND 指定した directory 配下の workflow から参照されない workflow および Facet について diagnostic は出ない
AND 終了コードは、この判定範囲で生じた diagnostic だけで決まる

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-001, B-002 |
| R-003 | B-003 |
| R-004 | B-003, B-004 |
| R-005 | B-005 |
| R-006 | B-005 |
| R-007 | B-006 |
| R-008 | B-007 |
| R-009 | B-008, B-009 |
| R-010 | B-010 |
| R-011 | B-011 |
| R-012 | B-012 |
| R-013 | B-013 |
