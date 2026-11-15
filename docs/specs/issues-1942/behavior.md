## B-001: Source Control の面に変更の一覧が出る

GIVEN 選んでいる worktree に、stage 済みの変更が 2 ファイル、まだ stage していない変更が別の 3 ファイルにある
WHEN 開発者が右カラムで Source Control を選ぶ
THEN 面の上部に「5 ファイルの変更」と出る
AND STAGED CHANGES の区画に 2 行、CHANGES の区画に 3 行が出て、それぞれの見出しに 2 と 3 が出る
AND 各行に、状態（M / A / D / U）、ファイル名、ディレクトリ、追加の行数と削除の行数が出る

## B-002: ファイル名で一覧を絞り込む

GIVEN 変更の一覧に `bridge.js` と `FileTabPane.swift` の行が出ている
WHEN 開発者が面の上部の絞り込みに「bridge」と入力する
THEN 一覧に `bridge.js` の行が出る
AND `FileTabPane.swift` の行は出ない

## B-003: Force Push を確認してから行う

GIVEN upstream があるブランチの Source Control の面が出ている
WHEN 開発者が面の上部の「…」を押し、「Force Push」を選ぶ
THEN 確認のダイアログが出る
AND 開発者が了承すると、ブランチが `--force-with-lease` で upstream に push され、upstream がローカルのブランチと同じ commit を指す
AND 開発者が取りやめると、upstream は変わらない

## B-004: upstream があるブランチのブランチ行

GIVEN ブランチ `feature/thread-ui` の upstream が `origin/feature/thread-ui` で、先行の commit が 2、遅れの commit が 0 である
WHEN 開発者が Source Control の面を見る
THEN ブランチ行に、`feature/thread-ui`、`origin/feature/thread-ui`、↑ 2、↓ 0 が出る

## B-005: upstream が無いブランチを Publish Branch で origin に出す

GIVEN ブランチに upstream が無く、origin のどのブランチにも無い commit が 2 つある
WHEN 開発者が Source Control の面を見る
THEN ブランチ行に、ブランチ名、「origin にまだありません（2 commit）」、「Publish Branch」が出て、↑ と ↓ は出ない
AND 開発者が「Publish Branch」を押すと、origin に同じ名前のブランチができ、2 つの commit がそこに入る
AND ブランチ行に、upstream の名前、↑ 0、↓ 0 が出る

## B-006: 変更を stage して commit する

GIVEN CHANGES の区画に `file.txt` の行があり、STAGED CHANGES の区画が空である
WHEN 開発者が `file.txt` の行にマウスを載せて「stage」を押す
THEN `file.txt` の行が CHANGES から消え、STAGED CHANGES に出る
AND 開発者が Commit メッセージを入力して「Commit」を押すと、`file.txt` の変更を含む、そのメッセージの commit ができる
AND STAGED CHANGES から `file.txt` の行が消え、入力欄が空になり、先行の commit 数が 1 増える

## B-007: commit の署名は git の設定に従う

GIVEN git の設定で commit の署名が有効で、STAGED CHANGES に行がある
WHEN 開発者がメッセージを入力して「Commit」を押す
THEN できた commit に署名が付いている

## B-008: stage 済みの変更かメッセージが無い間は commit できない

GIVEN CHANGES の区画に行があり、STAGED CHANGES の区画が空で、Commit メッセージが入力してある
WHEN 開発者が「Commit」と、その ▾ のメニューを見る
THEN 「Commit」は押せない
AND メニューの Commit、Commit & Push、Commit & Sync は選べない
AND STAGED CHANGES に行があっても、入力欄が空の間は、同じく押せず、選べない

## B-009: 「Commit」の ▾ のメニューに操作と commit 数が並ぶ

GIVEN 先行の commit が 2、遅れの commit が 0 で、STAGED CHANGES に行があり、Commit メッセージが入力してある
WHEN 開発者が「Commit」の ▾ を押す
THEN Commit、Commit & Push、Commit & Sync、Push、Pull、Sync、Fetch が、この順で出る
AND Push の右に ↑ 2、Pull の右に ↓ 0、Sync の右に ↓ 0 と ↑ 2 が出る
AND Commit & Push の右に ↑ 3、Commit & Sync の右に ↓ 0 と ↑ 3 が出る
AND Commit と Fetch の右に数は出ない
AND 開発者が Commit を選ぶと、「Commit」のボタンを押したときと同じく、stage 済みの変更が commit になる

## B-010: Push で commit を upstream に送る

GIVEN 先行の commit が 2、遅れの commit が 0 である
WHEN 開発者が ▾ のメニューで Push を選ぶ
THEN 2 つの commit が upstream に入る
AND ブランチ行の ↑ が 0 になる

## B-011: Pull で upstream の commit を取り込む

GIVEN 先行の commit が 0、遅れの commit が 1 である
WHEN 開発者が ▾ のメニューで Pull を選ぶ
THEN upstream の 1 つの commit が、git の設定に従った取り込み方でブランチに入る
AND ブランチ行の ↓ が 0 になる

## B-012: Sync は Pull の後に Push する

GIVEN 先行の commit が 1、遅れの commit が 1 である
WHEN 開発者が ▾ のメニューで Sync を選ぶ
THEN upstream の commit がブランチに入り、その後にブランチの commit が upstream に入る
AND ブランチ行の ↑ と ↓ が 0 になる

## B-013: Fetch で remote の状態を取得する

GIVEN 最後に remote の状態を取得した後に、upstream に commit が 1 つ足されている
WHEN 開発者が ▾ のメニューで Fetch を選ぶ
THEN ブランチ行の ↓ が 1 増える
AND worktree のファイルと、ブランチの commit は変わらない

## B-014: Commit & Push と Commit & Sync

GIVEN STAGED CHANGES に行があり、Commit メッセージが入力してあり、遅れの commit が 0 である
WHEN 開発者が ▾ のメニューで Commit & Push を選ぶ
THEN stage 済みの変更が commit になり、その commit が upstream に入る
AND ブランチ行の ↑ が 0 になる
AND 同じ状態で Commit & Sync を選ぶと、stage 済みの変更が commit になり、その後に Pull と Push が行われる

## B-015: upstream が無いブランチの Push

GIVEN ブランチに upstream が無い
WHEN 開発者が ▾ のメニューで Push を選ぶ
THEN origin に同じ名前のブランチができ、ブランチの commit がそこに入り、そのブランチが upstream になる
AND ブランチ行に、upstream の名前、↑、↓ が出る

## B-016: 操作が失敗したら理由が出る

GIVEN 遅れの commit が 1 以上で、upstream の remote が Push を受け付けない
WHEN 開発者が ▾ のメニューで Push を選ぶ
THEN 失敗の理由が画面に出る

## B-017: 同じファイルが両方の区画に出る

GIVEN `bridge.js` に、stage 済みの変更と、まだ stage していない変更がある
WHEN 開発者が Source Control の面を見る
THEN `bridge.js` の行が、STAGED CHANGES と CHANGES の両方に出る
AND STAGED CHANGES の行には stage 済みの変更の行数が、CHANGES の行にはまだ stage していない変更の行数が出る

## B-018: 行の unstage

GIVEN STAGED CHANGES の区画に `file.txt` の行がある
WHEN 開発者が `file.txt` の行にマウスを載せて「unstage」を押す
THEN `file.txt` の行が STAGED CHANGES から消え、CHANGES に出る

## B-019: 区画の全部を stage する・unstage する

GIVEN CHANGES の区画に 3 行があり、STAGED CHANGES の区画が空である
WHEN 開発者が CHANGES の見出しで、全部を stage する操作を押す
THEN CHANGES が空になり、3 行が STAGED CHANGES に出る
AND 開発者が STAGED CHANGES の見出しで、全部を unstage する操作を押すと、STAGED CHANGES が空になり、3 行が CHANGES に出る

## B-020: 行をクリックして差分を開く

GIVEN 変更の一覧に `bridge.js` の行がある
WHEN 開発者が `bridge.js` の行をクリックする
THEN 右カラムがファイル面に切り替わる
AND `bridge.js` のタブが、HEAD との差分で開く

## B-021: 行の破棄を確認してから行う

GIVEN CHANGES の区画に、状態 M の `bridge.js` の行があり、追加が 23 行、削除が 9 行である
WHEN 開発者が `bridge.js` の行にマウスを載せて「破棄」を押す
THEN 警告のダイアログが出て、`bridge.js` の名前、消える行数（+23 −9）、取り消せないことが出る
AND 開発者が了承すると、`bridge.js` のまだ stage していない変更が消え、行が CHANGES から消える
AND `bridge.js` に stage 済みの変更があれば、その変更と STAGED CHANGES の行は残る
AND 開発者が取りやめると、`bridge.js` の中身も CHANGES の行も変わらない

## B-022: git が追跡していないファイルの破棄はゴミ箱に移す

GIVEN CHANGES の区画に、状態 U の `thread-card.css` の行がある
WHEN 開発者が `thread-card.css` の行の「破棄」を押す
THEN ダイアログが出て、`thread-card.css` の名前と、ファイルをゴミ箱に移すことが出る
AND ダイアログに、取り消せないことは出ない
AND 開発者が了承すると、`thread-card.css` が worktree から消えてゴミ箱に入り、行が CHANGES から消える

## B-023: 区画の全部を破棄する

GIVEN CHANGES の区画に、状態 M のファイルの行が 2 つと、状態 U のファイルの行が 1 つある
WHEN 開発者が CHANGES の見出しで、全部を破棄する操作を押す
THEN 警告のダイアログが出て、消える行数の合計、取り消せないこと、状態 U のファイルをゴミ箱に移すことが出る
AND 開発者が了承すると、状態 M の 2 つのファイルのまだ stage していない変更が消え、状態 U のファイルがゴミ箱に入り、CHANGES が空になる

## B-024: 変更が無いときの表示

GIVEN 選んでいる worktree に変更したファイルが無く、先行の commit が 2 である
WHEN 開発者が Source Control の面を見る
THEN 面の上部に「変更なし」と出る
AND 変更の一覧の代わりに、「Worktree に変更はありません」と「Push（2 commit）」が出る
AND 「Commit」は押せない
AND 「Commit」の ▾ のメニューは開き、Push、Pull、Sync、Fetch を選べる
AND 開発者が「Push（2 commit）」を押すと、2 つの commit が upstream に入り、「Push（2 commit）」が消える

## B-025: rebase の途中で conflict があるときの表示

GIVEN ブランチ `feature/thread-ui` を main に rebase している途中で、3 つの commit のうち 2 つ目を適用していて、2 つのファイルに conflict がある
WHEN 開発者が Source Control の面を見る
THEN 面の上部の帯に、「main への rebase の途中です」、「3 commit 中 2 つ目」、conflict が 2 件あること、「中止」が出る
AND ブランチ行に、`feature/thread-ui` と「rebase 中」が出て、upstream の名前、↑、↓ は出ない
AND 変更の一覧の先頭に CONFLICTS の区画が出て、見出しに 2 が出る
AND CONFLICTS の各行に、ファイル名、ディレクトリ、conflict の種類、「+」が出る
AND 「Commit」の位置に「Rebase を続ける」が出て、▾ は出ない
AND 「Rebase を続ける」は押せず、横に「conflict をすべて解決すると続けられます」と出る
AND Commit メッセージの入力欄に、適用している commit のメッセージが入っている

## B-026: conflict を解決済みとして stage する

GIVEN rebase の途中で、CONFLICTS の区画に `bridge.js` の 1 行だけがある
WHEN 開発者が `bridge.js` の行の「+」を押す
THEN `bridge.js` の行が CONFLICTS から消え、STAGED CHANGES に出る
AND 「Rebase を続ける」が押せて、横の「conflict をすべて解決すると続けられます」は出ない

## B-027: Rebase を続ける

GIVEN rebase の途中で、3 つの commit のうち 3 つ目を適用していて、conflict が残っていない
WHEN 開発者が入力欄のメッセージを書き換えて「Rebase を続ける」を押す
THEN 書き換えたメッセージの commit ができ、rebase が終わる
AND 帯が消え、「Rebase を続ける」が「Commit」に戻る
AND 3 つのうち 2 つ目を適用しているときに押した場合は、rebase が 3 つ目に進み、帯の進み具合が「3 commit 中 3 つ目」になる
AND 進んだ先の commit で conflict が起きた場合は、帯の conflict の件数と CONFLICTS の区画が、その conflict の内容になり、失敗の理由が画面に出る

## B-028: merge の途中の表示

GIVEN merge の途中で、conflict が残っていない
WHEN 開発者が Source Control の面を見る
THEN 面の上部の帯に、merge の途中であることと「中止」が出る
AND 「Commit」の位置に「Merge を続ける」が出て、▾ は出ない
AND Commit メッセージの入力欄に、merge の commit のメッセージが入っている

## B-029: merge と rebase を中止する

GIVEN rebase の途中で、conflict がある
WHEN 開発者が帯の「中止」を押す
THEN ブランチと worktree のファイルが、rebase を始める前の状態に戻る
AND 帯と CONFLICTS の区画が消え、「Commit」が戻る
AND merge の途中で「中止」を押した場合も、ブランチと worktree のファイルが merge を始める前の状態に戻り、帯が消える

## B-030: Merge を続ける

GIVEN merge の途中で、conflict が残っていない
WHEN 開発者が「Merge を続ける」を押す
THEN 入力欄のメッセージで、merge の commit ができる
AND 帯が消え、「Merge を続ける」が「Commit」に戻る

## B-031: Pull が conflict で止まる

GIVEN upstream の commit とブランチの commit が、同じファイルの同じ行を変えている
WHEN 開発者が ▾ のメニューで Pull を選ぶ
THEN 面の上部に、merge か rebase の途中であることの帯が出て、CONFLICTS の区画にそのファイルの行が出る
AND 失敗の理由が画面に出る

## B-032: commit を作らないところで止まった rebase を続ける

GIVEN rebase が `break` で止まっていて、conflict が無い
WHEN 開発者が Source Control の面を見る
THEN 面の上部の帯に、rebase の途中であることと「中止」が出る
AND Commit メッセージの入力欄は空で、入力できない
AND 「Rebase を続ける」が押せる
AND 開発者が「Rebase を続ける」を押すと、commit は増えずに rebase が先へ進み、残りの commit が無ければ帯が消える

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-024 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-005 |
| R-007 | B-006 |
| R-008 | B-007 |
| R-009 | B-008, B-024 |
| R-010 | B-009 |
| R-011 | B-009, B-010, B-011, B-012, B-013, B-014, B-031 |
| R-012 | B-009 |
| R-013 | B-015 |
| R-014 | B-016, B-027, B-031 |
| R-015 | B-001 |
| R-016 | B-001 |
| R-017 | B-017 |
| R-018 | B-006 |
| R-019 | B-018 |
| R-020 | B-019, B-023 |
| R-021 | B-020 |
| R-022 | B-021 |
| R-023 | B-022 |
| R-024 | B-023 |
| R-025 | B-024 |
| R-026 | B-024 |
| R-027 | B-024 |
| R-028 | B-025, B-028, B-032 |
| R-029 | B-025 |
| R-030 | B-025 |
| R-031 | B-026 |
| R-032 | B-025, B-028 |
| R-033 | B-025, B-026 |
| R-034 | B-025, B-027, B-028, B-030, B-032 |
| R-035 | B-027 |
| R-036 | B-030 |
| R-037 | B-029 |
