## B-001: Node 面の表示

GIVEN worktree を選んでいる
WHEN 開発者が右カラムの面の切り替えを見る
THEN 「Node」の面があり、「Root の一覧」の面は無い

## B-002: Root の並び

GIVEN worktree に Session の Root、Command の Root、Workflow の実行の Root がある
WHEN 開発者が Node 面を開く
THEN 各 Root が 1 行ずつ並び、各行に状態の色とタイトルが出る

## B-003: Sequence と Fanout の開閉

GIVEN Node 面に Sequence と Fanout を含む実行木がある
WHEN 開発者が Sequence または Fanout の行を押す
THEN その行の子の Node・Sequence・Fanout が、親より一段字下げされて出る、または隠れる
AND 下段に出ている Node の詳細は変わらない

## B-004: 行の種別と情報

GIVEN Node 面に Sequence、Fanout、Session、Command の行が出ている
WHEN 開発者が各行を見る
THEN Sequence の行に `SEQ`、Fanout の行に `FANOUT` が出る
AND 失敗した Session と Command の行に失敗理由の一文が出る

## B-005: MODEL

GIVEN model を指定した Session、model を指定しない Session、Command が実行木にある
WHEN 開発者が各行の MODEL を見る
THEN model を指定した Session の行には、その model が出る
AND model を指定しない Session の行と Command の行の MODEL は空欄である

## B-006: Artifact の有無

GIVEN Artifact を提出した Node と提出していない Node が実行木にある
WHEN 開発者が ART 列を見る
THEN 提出した Node の行にだけ `◆` が出る

## B-007: 過去の試行と隔離 worktree

GIVEN Retry された Node と、隔離 worktree を持つ Sequence が実行木にある
WHEN 開発者が Node の行の過去の試行を開く
THEN その Node の過去の試行が行として並ぶ
AND 隔離 worktree を持つ Sequence の行に、その branch と path が出る

## B-008: 並列・深さ・周回の再入

GIVEN 12 並列の Fanout、深さ 4 の入れ子、同じ名前の Node へ 2 回戻った実行を含む実行木がある
WHEN 開発者がすべての行を開く
THEN 12 個の子、4 段の字下げ、周回ごとの同じ名前の Node が、それぞれ別の行として並ぶ

## B-009: 行の選択

GIVEN Node 面に Session の行がある
WHEN 開発者がその行を押す
THEN その行が選択され、下段にその Node の名前、種別、MODEL が出る
AND 中央に新しいタブは開かない

## B-010: タブを開く

GIVEN Node 面に Session の行と Command の行がある
WHEN 開発者が Session の行の `▷` を押す
THEN 中央にその Session の Session タブが開く
AND Command の行の `▷` を押すと、中央にその Command の Command タブが開く

## B-011: 失敗理由

GIVEN 失敗した Node を選択している
WHEN 開発者が下段を見る
THEN 下段にその Node の失敗理由が出る

## B-012: Artifact の表

GIVEN object と配列を含む Artifact を提出した Node を選択している
WHEN 開発者が下段を見る
THEN Artifact の中身が FIELD / VALUE の 2 列の表で出る
AND object と配列の中身は字下げして展開される

## B-013: 周回ごとの Artifact

GIVEN 同じ名前の Node が 2 周し、周ごとに異なる Artifact を提出している
WHEN 開発者が 1 周目の行と 2 周目の行を順に選択する
THEN 下段には、それぞれの周で提出した Artifact が出る

## B-014: JSON 表示

GIVEN Artifact を提出した Node を選択している
WHEN 開発者が JSON 表示に切り替える
THEN Artifact の中身が JSON で出る

## B-015: Retry / Abort

GIVEN Retry できる Node と、Abort できる実行の Node がある
WHEN 開発者がそれぞれを選択し、下段の Retry、Abort を押す
THEN Retry できる Node は再実行され、実行は Abort される
AND サーバがその操作を可としない Node では、その操作は出ない

## B-016: 開始と History のメニュー

GIVEN worktree を選んでいる
WHEN 開発者が Node 面の上端を見る
THEN New Session と New Workflow を含む「+」メニューと、History を含む「…」メニューがある

## B-017: Root の右クリックメニュー

GIVEN Rename、Resume、Archive をサーバが可とする Root がある
WHEN 開発者が Root の行を右クリックする
THEN 可とされた Rename、Resume、Archive がメニューに出て、選ぶとその操作が実行される

## B-018: archive 済みの実行の復元

GIVEN archive 済みの Workflow の実行がある
WHEN 開発者が History からその実行を選んで復元する
THEN その実行の Root が Node 面の実行木に戻る

## B-019: 右カラムの幅

GIVEN 右カラムを開いている
WHEN 開発者が右カラムの境界を左へ動かす
THEN 右カラムは 400 より広がる
AND 中央のカラムと右カラムは、ウィンドウの最小幅から左のサイドバーの最小幅を引いた残りの半分より狭くならない
AND 左のサイドバーは 400 より広げられない

## B-020: Workflow 管理タブが無い

GIVEN 実行木に Sequence と Fanout の行がある
WHEN 開発者が Node 面と中央のタブを操作する
THEN 「Workflow 管理」タブは開かない
AND クライアントがタブの種類 workflow を含むレイアウトを保存しようとすると、サーバは受け付けない

## B-021: 保存済みの Workflow 管理タブ

GIVEN 保存済みのペインのレイアウトに「Workflow 管理」タブと Terminal タブが含まれている
WHEN 開発者が Swift.app でその worktree を選ぶ
THEN Terminal タブを含む残りのレイアウトが読み込まれ、「Workflow 管理」タブは出ない

## B-022: 子の Command の過去の試行

GIVEN Sequence の子の Command が失敗し、Retry されている
WHEN 開発者がその Command の Command タブを開く
THEN Command タブにその Command の過去の試行が出る

## B-023: Node の実行ごとの Artifact の購読

GIVEN Artifact を提出した Node の実行がある
WHEN クライアントがその Node の実行を指定して Artifact を購読する
THEN その Node の実行が提出した Artifact だけが届く

## B-024: 行の Approve

GIVEN 承認待ちの Node と、承認待ちでない Node が実行木にある
WHEN 開発者が承認待ちの Node の行の Approve ボタンを押す
THEN その Node が承認され、承認待ちではなくなる
AND 承認待ちでない Node の行には Approve ボタンが出ない

## B-025: 中央と右のカラムの初期の幅

GIVEN 右カラムを開いている
WHEN 開発者がウィンドウを初期の幅で開く
THEN 中央のカラムと右カラムの幅は、ウィンドウの幅から左のサイドバーの初期の幅を引いた残りの半分ずつである
AND 左のサイドバーの幅は今の初期の幅のままである

## B-026: delegate の子と過去の試行の開閉

GIVEN delegate の子を持ち、Retry された Session が実行木にある
WHEN 開発者がその行の開閉の矢印を押す
THEN delegate の子だけが開閉し、過去の試行の開閉は変わらない
AND その行の過去の試行を開閉する操作では、過去の試行だけが開閉し、delegate の子の開閉は変わらない

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002, B-003 |
| R-003 | B-002, B-004, B-005, B-006 |
| R-004 | B-005 |
| R-005 | B-006 |
| R-006 | B-007, B-026 |
| R-007 | B-008 |
| R-008 | B-003, B-009 |
| R-009 | B-010 |
| R-010 | B-009, B-011 |
| R-011 | B-012, B-013 |
| R-012 | B-014 |
| R-013 | B-015 |
| R-014 | B-016, B-017 |
| R-015 | B-018 |
| R-016 | B-019 |
| R-017 | B-019, B-025 |
| R-018 | B-020 |
| R-019 | B-021 |
| R-020 | B-022 |
| R-021 | B-023 |
| R-022 | B-024 |
