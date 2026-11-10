## B-001: サイドバーのアイコン

GIVEN サイドバーに、main の worktree と、PR の状態が open、draft、merged、closed のブランチの worktree があり、Merged の worktree と、Session と Workflow を実行中の worktree がある
WHEN 開発者がサイドバーを見る
THEN リポジトリを追加、再読み込み、Worktree を作成の操作に、`folder-plus`、`refresh-cw`、`plus` のアイコンが出る
AND main の worktree のカードに `house`、ブランチの worktree のカードに `git-branch` のアイコンが出る
AND PR の状態が open、draft、merged、closed のカードに、`git-pull-request`、`git-pull-request-draft`、`git-merge`、`git-pull-request-closed` のアイコンが出る
AND Merged の印に `circle-check` のアイコンが出る
AND Session の行に `bot`、Workflow の行に `waypoints` のアイコンが出る

## B-002: Node 面の行の種別のアイコン

GIVEN 実行木に Session、Command、Sequence、Fanout の行がある
WHEN 開発者が Node 面を開く
THEN Session の行に `bot`、Command の行に `terminal`、Sequence の行に `waypoints`、Fanout の行に `git-fork` のアイコンが出る
AND 種別のアイコンの大きさは 8 ポイントである
AND どの行にも状態の丸は出ない

## B-003: Node 面の Session の詳細の種別のアイコン

GIVEN Node 面に Session の行がある
WHEN 開発者が Session の行を選ぶ
THEN 詳細に `bot` のアイコンが出る
AND そのアイコンの大きさは 8 ポイントである
AND そのアイコンは、選んだ行の状態の色である
AND 詳細に状態の丸は出ない

## B-004: 種別のアイコンの色

GIVEN Node 面に、状態が attention、active、idle の行がある
WHEN 開発者が Node 面を見る
THEN 種別のアイコンは、attention の行で黄、active の行で青、idle の行で緑である

## B-005: サイドバーとフッターのアイコンの色

GIVEN サイドバーに状態が active の Session の行があり、フッターに hook の状態を確認できないときの警告が出ている
WHEN 開発者が画面を見る
THEN Session の行の `bot` は青である
AND フッターの警告の `triangle-alert` は黄である

## B-006: 中央のタブのアイコン

GIVEN 中央に Terminal、ファイル、Session、Command のタブが開いている
WHEN 開発者が中央のタブの並びを見る
THEN 4 つのタブのアイコンは、どれも ● である

## B-007: 中央のタブのアイコンの色

GIVEN 中央に、Node の状態が active の Session のタブと、Terminal、ファイル、Command のタブが開いている
WHEN 開発者が中央のタブの並びを見る
THEN Session のタブの ● は青である
AND Terminal、ファイル、Command のタブの ● には、状態の色が付いていない

## B-008: Lucide のアイコンの形

アプリが画面に出す次の 32 個のアイコンの形は、`lucide-static` 1.71.0 の同じ名前のアイコンと同じである。
`bot`、`waypoints`、`terminal`、`git-fork`、`folder-plus`、`refresh-cw`、`plus`、`house`、`git-branch`、`circle-check`、`git-pull-request`、`git-pull-request-draft`、`git-merge`、`git-pull-request-closed`、`list`、`file`、`message-square`、`panel-right`、`history`、`ellipsis`、`diff`、`square-arrow-out-up-right`、`unfold-vertical`、`git-commit-horizontal`、`rows-2`、`columns-2`、`triangle-alert`、`sparkles`、`server`、`settings`、`folder`、`monitor`。

## B-009: Lucide のライセンスの表記

アプリの配布物は Lucide のライセンスの表記を含み、その内容は `lucide-static` 1.71.0 の `LICENSE` と同じである。

## B-010: Node 面の Command の詳細の種別のアイコン

GIVEN Node 面に Command の行がある
WHEN 開発者が Command の行を選ぶ
THEN 詳細に `terminal` のアイコンが出る
AND そのアイコンの大きさは 8 ポイントである
AND そのアイコンは、選んだ行の状態の色である
AND 詳細に状態の丸は出ない

## B-012: 右カラムの上の並びのアイコン

GIVEN 開発者が worktree を選んでいて、右カラムが開いている
WHEN 開発者が右カラムの上の並びを見る
THEN Node、ファイル、Source Control、Thread の切り替えに、`list`、`file`、`git-branch`、`message-square` のアイコンが出る
AND 右カラムの開閉に `panel-right` のアイコンが出る

## B-013: Node 面のメニューと過去の試行のアイコン

GIVEN Node 面に、過去の試行を持つ行がある
WHEN 開発者が Node 面を見る
THEN 「その他」のメニューに `ellipsis`、「開始」のメニューに `plus` のアイコンが出る
AND 過去の試行のボタンに `history` のアイコンが出る

## B-014: Thread の解決済みのアイコン

GIVEN Thread 面に、解決済みの Thread がある
WHEN 開発者が Thread 面を見る
THEN 解決済みの表示に `circle-check` のアイコンが出る

## B-015: 「開く」メニューのアイコン

GIVEN 中央にタブの並びが出ている
WHEN 開発者が、タブの並びの「開く」メニューを開く
THEN 「開く」メニューのボタンに `plus` のアイコンが出る
AND 項目「Terminal」に `terminal` のアイコンが出る

## B-016: ファイルタブのアイコン

GIVEN 開発者が、ファイルタブを Diff 表示で開いている
WHEN 開発者がファイルタブの上の操作の並びを見る
THEN Diff 表示、外部エディタで開く、全文に、`diff`、`square-arrow-out-up-right`、`unfold-vertical` のアイコンが出る
AND HEAD と比較、base と比較に、`git-commit-horizontal`、`git-branch` のアイコンが出る
AND 上下に並べる、左右に並べるに、`rows-2`、`columns-2` のアイコンが出る

## B-017: フッターのアイコン

GIVEN サーバとアプリの版が合っていて、フッターに Claude と Codex の使用量、hook の状態を確認できないときの警告、provider の警告が出ている
WHEN 開発者がフッターを見る
THEN Claude に `sparkles`、Codex に `terminal` のアイコンが出る
AND 2 つの警告に `triangle-alert` のアイコンが出る
AND サーバのメニューに `server` のアイコンが出る

## B-018: 版が合っていないときのサーバのメニューのアイコン

GIVEN サーバとアプリの版が合っていない
WHEN 開発者がフッターを見る
THEN サーバのメニューに `triangle-alert` のアイコンが出る

## B-019: 「サーバに接続できません」の画面のアイコン

GIVEN アプリがサーバに接続できない
WHEN 開発者がアプリのウィンドウを見る
THEN 「サーバに接続できません」の表示に `triangle-alert` のアイコンが出る

## B-020: Settings のアイコン

GIVEN 開発者が Settings を開いていて、Repositories タブで何も選んでいない
WHEN 開発者が Repositories タブを見る
THEN General、Repositories、Providers、Workflows、System のタブに、`settings`、`folder`、`terminal`、`waypoints`、`monitor` のアイコンが出る
AND 「リポジトリを選択してください」の表示に `folder` のアイコンが出る

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-012, B-013, B-014, B-015, B-016, B-017, B-018, B-019, B-020 |
| R-002 | B-002, B-003, B-010 |
| R-003 | B-003, B-004, B-010 |
| R-004 | B-005 |
| R-005 | B-006 |
| R-006 | B-007 |
| R-007 | B-008 |
| R-008 | B-009 |
