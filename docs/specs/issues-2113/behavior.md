## B-001: サイドバーの上端

GIVEN リポジトリを 1 つ以上登録している
WHEN 開発者がサイドバーを見る
THEN 「Workspaces」の見出しが出ない
AND サイドバーの上端に出るボタンは、「Worktreeを作成」の 1 つだけである
AND 「Worktreeを作成」のボタンは、ウィンドウの操作ボタンと同じ高さの帯の右端に出る

## B-002: サイドバーのフッター

GIVEN リポジトリを 1 つ以上登録している
WHEN 開発者がサイドバーの下端を見る
THEN フッターに、リポジトリを追加するボタンと「更新」のボタンの 2 つが、右寄せで並ぶ

## B-003: 「更新」のボタンの働き

GIVEN サイドバーに worktree の一覧が出ている
WHEN 開発者がフッターの「更新」のボタンを押す
THEN worktree の一覧が、読み込み直した内容で出る

## B-004: リポジトリのエラー

GIVEN 取得に失敗したリポジトリがある
WHEN 開発者がサイドバーを見る
THEN そのリポジトリの見出しの右端に、警告の印（`triangle-alert`）が出る
AND リポジトリの区切りの中に、エラー文の行は出ない

## B-005: リポジトリのエラー文

GIVEN リポジトリの見出しに警告の印が出ている
WHEN 開発者が警告の印にマウスを乗せる
THEN そのリポジトリのエラー文が出る

## B-006: PR の状態のアイコン

GIVEN PR の状態が open、draft、merged、closed のブランチの worktree が、それぞれある
WHEN 開発者がサイドバーを見る
THEN それぞれのカードの 1 行目の右端に、PR の状態のアイコンが出る
AND アイコンの色は、open が緑、draft が灰、merged が紫、closed が赤である
AND どのカードにも、PR の状態のための行は出ない

## B-007: Merged の印と変更の数

GIVEN マージ済みで、変更の数が 1 以上のブランチの worktree がある
WHEN 開発者がサイドバーを見る
THEN その worktree のカードに、Merged の印（`circle-check`）が出ない
AND その worktree のカードに、「N changes」が出ない

## B-008: 取得失敗の印

GIVEN 変更の数の取得失敗、PR の取得失敗、worktree のエラーのうち、1 つだけがある worktree と、3 つとも重なった worktree がある
WHEN 開発者がサイドバーを見る
THEN どちらのカードも、1 行目の右端に、警告の印（`triangle-alert`）を 1 つ出す
AND どちらのカードにも、「Changes unavailable」「PR unavailable」と worktree のエラー文の行は出ない

## B-009: 取得失敗のエラー文

GIVEN 変更の数の取得失敗、PR の取得失敗、worktree のエラーが重なった worktree のカードに、警告の印が出ている
WHEN 開発者が警告の印にマウスを乗せる
THEN 「Changes unavailable」と変更の数のエラー文が出る
AND 「PR unavailable」と PR のエラー文が出る
AND worktree のエラー文が出る

## B-010: PR の状態のアイコンと警告の印

GIVEN PR があり、変更の数の取得に失敗したブランチの worktree がある
WHEN 開発者がサイドバーを見る
THEN その worktree のカードの 1 行目の右端に、PR の状態のアイコンと警告の印が並ぶ

## B-011: Session の行

GIVEN Session が動いている worktree がある
WHEN 開発者がサイドバーを見る
THEN その worktree のカードの Session の行に、アイコンとタイトルが出る
AND Session の行に、provider 名（Claude、Codex）の文字は出ない

## B-012: Workflow の行

GIVEN Workflow が動いている worktree があり、同じサイドバーに Session の行が出ている
WHEN 開発者がサイドバーを見る
THEN その worktree のカードの Workflow の行に、アイコン、名前、「· N nodes」が出る
AND 名前は、Session の行のタイトルと同じ文字色、同じ大きさである
AND 「· N nodes」は灰色である
AND Workflow の行の下に、Session の状態の四角の並びは出ない

## B-013: 実行が無い worktree のカードの高さ

GIVEN 実行が無く、削除している途中でもない worktree と、実行が 1 つだけある worktree がある
WHEN 開発者がサイドバーを見る
THEN 実行が無い worktree のカードは、2 行目が空である
AND 実行が無い worktree のカードは、実行が 1 つだけある worktree のカードと同じ高さである

## B-014: 削除している途中の worktree のカード

GIVEN 実行が 1 つある worktree がある
WHEN 開発者がその worktree を削除し、削除が終わるまでの間にサイドバーを見る
THEN その worktree のカードの 2 行目に、進行中の印と「削除中」の文字が出る
AND 実行の行は、「削除中」の行の下に出る

## B-015: 変更の数のバッジ

GIVEN 右カラムを開いていて、変更の数が 3 の worktree がある
WHEN 開発者がその worktree を選ぶ
THEN Source Control の切り替えアイコンの右上に、「3」と書いた灰色の丸いバッジが重なる

## B-016: 変更の数が 99 を超えるときのバッジ

GIVEN 右カラムを開いていて、変更の数が 100 の worktree がある
WHEN 開発者がその worktree を選ぶ
THEN Source Control の切り替えアイコンのバッジに、「99+」と出る

## B-017: 変更の数が 0 のときのバッジ

GIVEN 右カラムを開いていて、変更の数が 0 の worktree がある
WHEN 開発者がその worktree を選ぶ
THEN Source Control の切り替えアイコンにバッジは出ない

## B-018: Lucide のアイコン

GIVEN PR があるブランチの worktree、取得に失敗した worktree、取得に失敗したリポジトリがある
WHEN 開発者がサイドバーを見る
THEN 「Worktreeを作成」のボタン、リポジトリを追加するボタン、「更新」のボタンのアイコンが、Lucide のアイコンで出る
AND PR の状態のアイコンと警告の印が、Lucide のアイコンで出る

## B-019: 変更の数を取得できなかったときのバッジ

GIVEN 右カラムを開いていて、変更の数の取得に失敗した worktree がある
WHEN 開発者がその worktree を選ぶ
THEN Source Control の切り替えアイコンにバッジは出ない

## B-020: worktree を選んでいないときのバッジ

GIVEN 右カラムを開いていて、変更の数が 1 以上の worktree がある
WHEN 開発者がどの worktree も選ばずに、Source Control の切り替えアイコンを見る
THEN Source Control の切り替えアイコンにバッジは出ない

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002, B-003 |
| R-003 | B-004, B-005 |
| R-004 | B-006 |
| R-005 | B-006 |
| R-006 | B-007 |
| R-007 | B-007 |
| R-008 | B-008 |
| R-009 | B-009 |
| R-010 | B-010 |
| R-011 | B-011 |
| R-012 | B-012 |
| R-013 | B-013 |
| R-014 | B-014 |
| R-015 | B-015 |
| R-016 | B-016 |
| R-017 | B-017, B-019, B-020 |
| R-018 | B-018 |
