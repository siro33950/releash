## B-001: 切り替えに PR のアイコンが出る

GIVEN 開発者が Worktree を選んでいる
WHEN 右カラムが開いている
THEN 右カラムの切り替えの、Source Control と Thread の間に、PR のアイコンが出る

## B-002: PR のアイコンで PR の面が出る

GIVEN 開発者が Worktree を選んでいる
WHEN 開発者が右カラムの切り替えの PR のアイコンを押す
THEN 右カラムに PR の面が出る

## B-003: Worktree を選び替えると PR の面が替わる

GIVEN PR の面に、選んでいる Worktree のブランチの PR が出ている
AND 別の Worktree のブランチには PR が無い
WHEN 開発者がその別の Worktree を選ぶ
THEN PR の面に、「このブランチの PR はありません」が出る

## B-004: PR が無いときに PR を作る欄が出る

GIVEN 選んでいる Worktree のブランチに PR が無い
WHEN PR の面が出る
THEN 「このブランチの PR はありません」が出る
AND Title の入力欄、Description（任意）の入力欄、base の選択、「draft で作る」のチェック、「PR を作成」ボタンが出る

## B-005: base の選択

GIVEN リポジトリの既定のブランチが `main` であり、origin に `main` と `develop` がある
WHEN PR を作る欄が出る
THEN base には `main` が選ばれている
AND base の選択に `main` と `develop` が並ぶ

## B-006: Title が空のあいだ PR を作成できない

GIVEN PR を作る欄が出ている
WHEN Title の入力欄が空である
THEN 「PR を作成」は押せない

## B-007: PR を作る

GIVEN 選んでいる Worktree のブランチが origin にあり、そのブランチに PR が無い
AND 開発者が Title と Description を書き、base を選んでいる
WHEN 開発者が「PR を作成」を押す
THEN そのブランチを head、選んだ base を base とする PR が GitHub にできる
AND PR のタイトルは書いた Title であり、PR の本文は書いた Description である
AND PR の面は、できた PR の見出し、本文、CHECKS、COMMENTS の表示に替わる

## B-008: draft で PR を作る

GIVEN 選んでいる Worktree のブランチが origin にあり、そのブランチに PR が無い
AND 開発者が Title を書き、「draft で作る」にチェックを入れている
WHEN 開発者が「PR を作成」を押す
THEN PR が draft でできる
AND PR の面の見出しに、Draft の状態のアイコンが出る

## B-009: origin に無いブランチでは PR を作らない

GIVEN 選んでいる Worktree のブランチが origin に無い
AND 開発者が Title と Description を書いている
WHEN 開発者が「PR を作成」を押す
THEN PR はできない
AND ブランチは origin に push されない
AND PR の面に、ブランチの push が要ることが出る
AND 書いた Title と Description は残る

## B-010: PR の作成が失敗する

GIVEN 選んでいる Worktree のブランチが origin にある
AND 開発者が Title と Description を書いている
WHEN 開発者が「PR を作成」を押し、GitHub が PR の作成を拒む
THEN PR の面に、失敗の理由が出る
AND 書いた Title と Description は残る

## B-011: GitHub のものでないリポジトリ

GIVEN 選んでいる Worktree のリポジトリが GitHub のものでない
WHEN PR の面が出る
THEN 「このブランチの PR はありません」と PR を作る欄が出る

## B-012: GitHub のものでないリポジトリで PR を作成する

GIVEN 選んでいる Worktree のリポジトリが GitHub のものでない
AND 開発者が Title を書いている
WHEN 開発者が「PR を作成」を押す
THEN PR の面に、失敗の理由が出る

## B-013: PR があるときの並び

GIVEN 選んでいる Worktree のブランチに PR がある
WHEN PR の面が出る
THEN 上から順に、見出し、本文、CHECKS、COMMENTS が出る
AND 下端に、Comment の入力欄と「Comment」ボタンが出る

## B-014: 見出し

GIVEN 選んでいる Worktree のブランチに、タイトルが「Thread 面を作り直す」で番号が 2128 の PR がある
WHEN PR の面が出る
THEN 見出しに、状態のアイコン、「Thread 面を作り直す」、「#2128」、ブラウザで開くアイコンが出る
AND 状態を表す文字は出ない

## B-015: 状態のアイコンと色

GIVEN 選んでいる Worktree のブランチの PR の状態が、Open、Draft、Merged、Closed のいずれかである
WHEN PR の面が出る
THEN 見出しの状態のアイコンと色は、次の表のとおりである

| 状態 | アイコン（Lucide） | 色 |
| --- | --- | --- |
| Open | `git-pull-request` | 緑 |
| Draft | `git-pull-request-draft` | 灰 |
| Merged | `git-merge` | 紫 |
| Closed | `git-pull-request-closed` | 赤 |

## B-016: ブラウザで開く

GIVEN PR の面に PR が出ている
WHEN 開発者が見出しのブラウザで開くアイコンを押す
THEN その PR の GitHub のページが既定のブラウザで開く

## B-017: 本文が Markdown として整形される

GIVEN PR の本文に、Markdown で書いた箇条書きがある
WHEN PR の面が出る
THEN 本文の箇条書きが、整形された箇条書きとして出る

## B-018: Closed の PR から新しい PR を作る

GIVEN 選んでいる Worktree のブランチの PR が Closed である
WHEN 開発者が、PR の面の新しい PR を作る操作を押す
THEN PR の面に PR を作る欄が出る

## B-019: Closed の PR の後に作った PR に替わる

GIVEN 選んでいる Worktree のブランチの PR が Closed であり、そのブランチが origin にある
AND 開発者が新しい PR を作る操作を押し、PR を作る欄に Title を書いている
WHEN 開発者が「PR を作成」を押す
THEN PR の面は、できた PR の表示に替わる

## B-020: Closed 以外では新しい PR を作る操作が出ない

GIVEN 選んでいる Worktree のブランチの PR の状態が、Open、Draft、Merged のいずれかである
WHEN PR の面が出る
THEN 新しい PR を作る操作は出ない

## B-021: CHECKS の件数

GIVEN PR に、失敗の check が 1 件、取り消しの check が 1 件、実行中の check が 1 件、skip の check が 2 件、通過の check が 7 件ある
WHEN PR の面が出る
THEN CHECKS の見出しの右に、失敗 1、取り消し 1、実行中 1、skip 2、通過 7 の件数が出る

## B-022: 0 件の分類の件数は出ない

GIVEN PR に、通過の check が 18 件あり、ほかの分類の check が無い
WHEN PR の面が出る
THEN CHECKS の見出しの右に、通過 18 の件数だけが出る

## B-023: 失敗、取り消し、実行中の check の行

GIVEN PR に、4 分で失敗した check、取り消された check、6 分前から実行中の check が 1 件ずつある
WHEN PR の面が出る
THEN CHECKS に、3 件の check が 1 行ずつ出る
AND 各行に、分類を表すアイコンと check の名前が出る
AND 失敗の行に「4 分で失敗」が出る
AND 実行中の行に「実行中 6 分」が出る

## B-024: 通過と skip の check はたたまれる

GIVEN PR に、通過の check が 7 件、skip の check が 2 件ある
WHEN PR の面が出る
THEN CHECKS に、「通過した 7 件」の行と「skip した 2 件」の行が出る
AND 通過の check と skip の check の名前は出ない

## B-025: たたまれた行を開閉する

GIVEN CHECKS に「通過した 7 件」の行が出ている
WHEN 開発者がその行を押す
THEN 通過の 7 件の check の名前が並ぶ
AND もう一度その行を押すと、「通過した 7 件」の 1 行に戻る

## B-026: 失敗した GitHub Actions の job のログを開く

GIVEN CHECKS に、失敗した GitHub Actions の job の行が出ている
WHEN 開発者がその行の「ログを開く」を押す
THEN 中央のタブに、その check のログが開く

## B-027: 外部のサービスの check にはログを開く操作が出ない

GIVEN PR に、外部のサービスが報告した失敗の check がある
WHEN PR の面が出る
THEN その check の行に「ログを開く」は出ない

## B-028: COMMENTS の並び

GIVEN PR に、3 時間前に書かれた PR 全体への Comment、2 時間前に提出された本文があるレビュー、1 時間前に書かれた PR 全体への Comment がある
AND 本文が無いレビューと、コードの行へのレビューの Comment もある
WHEN PR の面が出る
THEN COMMENTS に、3 時間前の Comment、2 時間前のレビューの本文、1 時間前の Comment がこの順に並ぶ
AND COMMENTS の見出しの右に 3 が出る

## B-029: 人が書いた Comment

GIVEN PR に、人が 40 分前に書いた、Markdown でコードを書いた Comment がある
WHEN PR の面が出る
THEN その Comment に、書き手の名前の頭文字のアイコン、書き手の名前、「40 分前」、本文が出る
AND 本文のコードは、整形されたコードとして出る
AND 「bot」の印は出ない

## B-030: bot が書いた Comment

GIVEN PR に、bot が 2 時間前に書いた Comment がある
WHEN PR の面が出る
THEN その Comment に、bot を表す絵のアイコン、書き手の名前、「2 時間前」、本文が出る
AND 書き手の名前の右に「bot」の印が出る

## B-031: 入力欄が空のあいだ Comment を投稿できない

GIVEN PR の面に PR が出ている
WHEN Comment の入力欄が空である
THEN 「Comment」は押せない

## B-032: Comment を投稿する

GIVEN PR の面に PR が出ている
AND 開発者が Comment の入力欄に文を書いている
WHEN 開発者が「Comment」を押す
THEN その文が、PR 全体への Comment として GitHub に投稿される
AND その Comment が COMMENTS の最後に出る
AND 入力欄は空に戻る

## B-033: Comment の投稿が失敗する

GIVEN PR の面に PR が出ている
AND 開発者が Comment の入力欄に文を書いている
WHEN 開発者が「Comment」を押し、GitHub が投稿を拒む
THEN PR の面に、失敗の理由が出る
AND 書いた文は入力欄に残る

## B-034: GitHub での変化が反映される

GIVEN PR の面に、実行中の check が 1 件ある PR が出ている
WHEN その check が GitHub で通過になり、開発者が何も操作せずに 30 秒が過ぎる
THEN その check は実行中の行から消え、通過の件数が 1 増える

## B-035: PR の内容を取れない

GIVEN GitHub の認証が切れていて、PR の内容を GitHub から取れない
WHEN PR の面が出る
THEN PR の面に、取れない理由が出る

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-003 |
| R-003 | B-004 |
| R-004 | B-005 |
| R-005 | B-006 |
| R-006 | B-007, B-008 |
| R-007 | B-009 |
| R-008 | B-010 |
| R-009 | B-011, B-012 |
| R-010 | B-013 |
| R-011 | B-014, B-015 |
| R-012 | B-016 |
| R-013 | B-017 |
| R-014 | B-018, B-019, B-020 |
| R-015 | B-021, B-022 |
| R-016 | B-023 |
| R-017 | B-024, B-025 |
| R-018 | B-026, B-027 |
| R-019 | B-028 |
| R-020 | B-029, B-030 |
| R-021 | B-031 |
| R-022 | B-032 |
| R-023 | B-033 |
| R-024 | B-034 |
| R-025 | B-035 |
