## B-001: 設定画面はナビゲーションとページを左右に並べる

GIVEN Swift.app が起動している
WHEN 開発者が設定画面を開く
THEN 設定画面の大きさは 1024×800 である
AND 設定画面の左に、General・Repositories・Providers・Workflows・System が、この順にアイコン付きで並ぶ
AND 設定画面の右に、選ばれている項目のページが出る

## B-002: ナビゲーションの項目を選ぶとページが替わる

GIVEN 設定画面が開いている
WHEN 開発者がナビゲーションで System を選ぶ
THEN 設定画面の右に System のページが出る
AND ウィンドウの上部に「System」の名前は出ない

## B-003: General は 3 つのまとまりに分かれる

GIVEN 設定画面が開いている
WHEN 開発者がナビゲーションで General を選ぶ
THEN 「エディタ」のまとまりに、外部エディタと Diff base の既定の行が並ぶ
AND 「Terminal」のまとまりに、Terminal の起動コマンドの行がある
AND 「Workflow」のまとまりに、承認の自動化の行がある
AND 各行は、左に項目名、右に値か操作を置く

## B-004: Repositories は一覧に名前とパスを出す

GIVEN Repository が登録されていて、設定画面が開いている
WHEN 開発者がナビゲーションで Repositories を選ぶ
THEN 登録されている Repository の一覧が出る
AND 一覧の各行は、Repository の名前とパスを出す

## B-005: origin が GitHub の Repository は GitHub 上のリポジトリ名を名前にする

GIVEN origin が GitHub のリポジトリ `siro33950/releash` である Repository が、フォルダ `work` に登録されている
WHEN 開発者が設定画面の Repositories の一覧を見る
THEN その Repository の行は、名前として `releash` を出す

## B-006: origin が GitHub でない Repository はフォルダ名を名前にする

GIVEN origin が GitHub のリポジトリでない Repository が、フォルダ `articles` に登録されている
WHEN 開発者が設定画面の Repositories の一覧を見る
THEN その Repository の行は、名前として `articles` を出す

## B-007: 同じ名前の Repository が並ぶときは owner を付ける

GIVEN origin が GitHub のリポジトリ `siro33950/releash` である Repository と、origin が GitHub のリポジトリ `example/releash` である Repository が登録されている
WHEN 開発者が設定画面の Repositories の一覧を見る
THEN 2 つの行は、名前として `siro33950/releash` と `example/releash` を出す

## B-008: Repository の行を選ぶと詳細のページに入り、一覧に戻れる

GIVEN 設定画面の Repositories の一覧が出ている
WHEN 開発者が一覧の行を選ぶ
THEN その Repository の詳細のページが出て、上部にその Repository の名前が出る
AND 詳細のページに、パスと Base branch のまとまり、Notion の連携のまとまり、一覧からの削除のまとまりが並ぶ
AND 開発者が左上のボタンを押すと、Repositories の一覧に戻る

## B-009: Notion の接続テストに成功すると Labels のトグルがプロパティごとに出る

GIVEN Repository の詳細のページが出ている
WHEN 開発者が Notion の接続テストを行い、成功する
THEN Notion の連携のまとまりに、「Labels」のトグルが Notion のプロパティごとに出る

## B-010: Providers は provider ごとのまとまりと再確認を出す

GIVEN 設定画面が開いている
WHEN 開発者がナビゲーションで Providers を選ぶ
THEN provider ごとに 1 つのまとまりが出る
AND ページのいちばん下に「可用性を再確認」がある

## B-011: Workflows は一覧のページを出す

GIVEN 実行中の定義、builtin の定義、診断に error がある定義が存在し、設定画面が開いている
WHEN 開発者がナビゲーションで Workflows を選ぶ
THEN 「定義 / Facet」の切り替え、定義の一覧、「定義のフォルダを外部エディタで開く」のボタン、診断の error と info の件数が出る
AND 一覧の各行は、定義の名前を出す
AND 実行中の定義の行は実行中の印を、builtin の定義の行は builtin の印を、診断に error がある定義の行はその error の件数の印を出す

## B-012: 定義の行を選ぶと定義の詳細のページに入り、一覧に戻れる

GIVEN 設定画面の Workflows の一覧で「定義」が選ばれている
WHEN 開発者が builtin でない定義の行を選ぶ
THEN 定義の詳細のページが出て、上部にその定義の名前が出る
AND ページの右上に「外部エディタで開く」がある
AND 開発者が左上のボタンを押すと、Workflows の一覧に戻る

## B-013: Facet の行を選ぶと Facet の詳細のページに入り、一覧に戻れる

GIVEN 設定画面の Workflows の一覧で「Facet」が選ばれている
WHEN 開発者が builtin でない Facet の行を選ぶ
THEN Facet の詳細のページが出て、上部にその Facet の種類と key が出る
AND ページの右上に「外部エディタで開く」がある
AND 開発者が左上のボタンを押すと、Workflows の一覧に戻る

## B-014: System は 4 つのまとまりに分かれる

GIVEN 設定画面が開いている
WHEN 開発者がナビゲーションで System を選ぶ
THEN 「更新」「ログイン項目」「CLI の設置」「Telemetry」の 4 つのまとまりが出る

## B-015: General の項目は説明の文を出す

GIVEN 設定画面が開いている
WHEN 開発者がナビゲーションで General を選ぶ
THEN 「Diff base の既定」の項目名の下に、灰色で「ファイルタブで差分を開いたときに、最初に比べる相手です。」と出る
AND 「Terminal の起動コマンド」の項目名の下に、灰色で「新しい Terminal のタブで shell が起動した直後に、このコマンドを入力して実行します。空なら何もしません。」と出る
AND 「承認の自動化」の項目名の下に、灰色で「承認を求める Node を、人の確認なしに完了させます。」と出る

## B-016: 確定した設定は設定画面を開き直しても保たれる

GIVEN 設定画面の General のページが出ている
WHEN 開発者が Terminal の起動コマンドを変えて確定する
THEN 設定画面を閉じて開き直しても、Terminal の起動コマンドの欄は確定した値を出す

## B-017: Repository の詳細の項目は説明の文を出す

GIVEN Repository の詳細のページが出ていて、Notion の接続テストに成功している
WHEN 開発者がそのページを見る
THEN 「Base branch」の項目名の下に、灰色で「ファイルタブで「base と比較」を選んだときに、比べる相手になるブランチです。」と出る
AND 「Labels」の項目名の下に、灰色で「新しい Worktree のダイアログで、Notion task を絞り込むのに使います。」と出る

## B-018: Providers の項目は説明の文を出す

GIVEN 利用できる provider があり、設定画面が開いている
WHEN 開発者がナビゲーションで Providers を選ぶ
THEN 「有効な実行ファイル」の項目名の下に、灰色で「上書きがあればその値、無ければ既定の実行ファイルです。」と出る
AND 「解決された実行ファイル」の項目名の下に、灰色で「有効な実行ファイルを PATH から探した結果です。」と出る
AND 「実行ファイルの上書き」の項目名の下に、灰色で「既定と別の実行ファイルを使うときに指定します。」と出る

## B-019: System の項目は説明の文を出す

GIVEN 設定画面が開いている
WHEN 開発者がナビゲーションで System を選ぶ
THEN 「nightly アップデートを受け入れる」の項目名の下に、灰色で「毎日作られる開発中の版も、更新の対象にします。」と出る
AND 「最小化状態で起動」の項目名の下に、灰色で「ログイン時に起動したときは、ウィンドウを開きません。」と出る
AND 「Performance telemetry」の項目名の下に、灰色で「サーバの動作の計測値（metrics と traces）を送ります。」と出る
AND 「Crash reporting」の項目名の下に、灰色で「異常終了したときに、その記録を送ります。」と出る

## B-020: builtin の定義と Facet の詳細に「外部エディタで開く」は出ない

GIVEN 設定画面の Workflows の一覧が出ている
WHEN 開発者が builtin の定義の行、または builtin の Facet の行を選ぶ
THEN その詳細のページが出る
AND そのページに「外部エディタで開く」は出ない

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-001 |
| R-003 | B-002 |
| R-004 | B-002 |
| R-005 | B-003 |
| R-006 | B-003 |
| R-007 | B-004 |
| R-008 | B-005, B-006, B-007 |
| R-009 | B-008 |
| R-010 | B-008, B-009 |
| R-011 | B-010 |
| R-012 | B-011 |
| R-013 | B-012, B-013, B-020 |
| R-014 | B-014 |
| R-015 | B-015, B-017, B-018, B-019 |
| R-016 | B-016 |
