# Context

- 要求の正本は、ISSUE #1942「[17] Source Control の面（変更の一覧・stage・Commit・Push）」の本文と、本文に貼られた参考 mock の画像 7 枚である。
  - 通常: <https://github.com/user-attachments/assets/25bdd43f-bbcc-4396-8c2a-e12c89e2ae5d>
  - 「Commit」の ▾ を開いた: <https://github.com/user-attachments/assets/28b850a2-ea64-47d2-8765-5bd00e8584c8>
  - 行をクリックして差分を開いた: <https://github.com/user-attachments/assets/839ac76f-2705-4fe9-99ba-3b0a3ddee795>
  - 破棄の確認: <https://github.com/user-attachments/assets/77473166-caef-41d2-8298-8b179c390de6>
  - 変更が無い: <https://github.com/user-attachments/assets/50efa37c-7bfe-4e63-9ade-1b162fe020db>
  - rebase の途中で conflict がある: <https://github.com/user-attachments/assets/5e9eff03-6564-4ebe-a73f-3b6118908054>
  - origin に無いブランチ: <https://github.com/user-attachments/assets/e500958e-ae90-41af-b32f-3391138f5bd3>
- 調査の時点は main `9e9cc211a` である。
- マイルストーン #78 は、mock を、配置と、出す情報と、できる操作の正と定めている。色と質感の正ではない。mock の中のファイル名、コード、commit メッセージは例である。
- 右カラムの上部には、Node、ファイル、Source Control、Thread の 4 つの面を切り替えるアイコンが並ぶ。Source Control の切り替えアイコンに重なる変更の数のバッジは、ISSUE #2113 で作った。その仕様は `docs/specs/issues-2113/` が持つ。
- ファイル面のタブは、ファイルの中身と差分を出す。差分の比較の基準は、HEAD と、ブランチの base の 2 つから選ぶ。その仕様は `docs/specs/issues-1760/` が持つ。
- mock の右カラムの上部には、5 つ目の切り替えアイコンがある。これは PR の面のもので、ISSUE #1943 が扱う。
- この文書は、次の語を使う。
  - **upstream**: ブランチが追跡している、remote の側のブランチ。remote は origin とは限らない。
  - **先行の commit 数**: ブランチにあり、upstream に無い commit の数。画面では ↑ で出す。
  - **遅れの commit 数**: upstream にあり、ブランチに無い commit の数。画面では ↓ で出す。
  - **状態 M / A / D / U**: M は中身を変えたファイル、A は新しく足して stage したファイル、D は消したファイル、U は git が追跡していないファイルである。
- ISSUE は、完了条件として、変更を stage して commit できることを確かめる回帰の E2E を足すことと、手元の fixture のリポジトリで確認した証跡を PR に貼ることを挙げている。

# Outcome

対象者は、Swift.app で worktree の変更を確かめ、commit と push を行う開発者である。

今の Swift.app は、右カラムで Source Control を選んでも何も出さない。
開発者が画面からできるのは、メニューで全部の変更をまとめて stage するか unstage することだけである。
ファイルを選んで stage すること、commit、push、pull、fetch、変更の破棄は、terminal かほかの道具で行うことになる。
agent が作った変更を確かめて commit するたびに、開発者は Releash を離れる。

変更後は、開発者が Source Control の面で、変更したファイルを見て、ファイルごとに stage し、commit と push までを済ませる。
merge と rebase の途中で conflict が起きたときも、同じ面で、解決済みの印を付け、続行するか中止する。

# Current Behavior

調査は main `9e9cc211a` のコードを読んで行った。
アプリを起動して画面を見ることはしていない。

## Source Control の面

- 右カラムの上部の切り替えで Source Control を選ぶと、切り替えの行の下は空である。Node、ファイル、Thread を選んだときは、それぞれの面が出る。
- Source Control の切り替えアイコンには、選んでいる worktree の変更の数がバッジで重なる。

再現手順は次のとおりである。

1. 変更したファイルがある worktree を、サイドバーで選ぶ。
2. 右カラムの上部の Source Control の切り替えアイコンを押す。
3. 切り替えの行の下に、何も出ない。

## stage と unstage

- メニューバーの「Git」に、「すべてステージ」「すべてアンステージ」「ブランチを作成…」の 3 つがある。
- 「すべてステージ」は、選んでいる worktree の変更を全部 stage する。「すべてアンステージ」は、stage 済みの変更を全部 unstage する。
- ファイルを選んで stage する操作と unstage する操作は、画面に無い。

## 画面に無い操作と表示

次の操作と表示は、画面に無い。
Swift.app の画面のソース（`clients/macos/Sources/`）を読み、これらを出す画面と、これらを行うボタンやメニューの項目が無いことを確かめた。

- commit、push、pull、fetch、変更の破棄。
- stage 済みの変更と、まだ stage していない変更を分けた一覧。ファイル面のファイル一覧は、ファイルごとに状態を 1 つだけ出す。
- upstream の名前と、先行と遅れの commit 数。
- merge と rebase の途中であることの表示と、その中止と続行。

# Scope / Non-goals

## Scope

- 右カラムの Source Control の面の中身。上部、ブランチ行、Commit の入力欄とボタン、変更の一覧、破棄の確認、変更が無いときの表示、merge と rebase の途中の表示と操作を含む。

メニューバーの「Git」の 3 つの項目、Source Control の切り替えアイコンのバッジ、ファイル面は、今のまま変えない。

## Non-goals

- PR の作成と表示。ISSUE #1943 が扱う。
- mock の Commit メッセージの入力欄の右上にある「AI で生成」。

# Requirements

## 面の上部

- R-001: 開発者が右カラムで Source Control を選ぶと、選んでいる worktree の Source Control の面が出る。面の上部に、変更したファイルの数が「5 ファイルの変更」の形で出る。変更したファイルが無いときは「変更なし」と出る。
- R-002: 面の上部に、ファイル名での絞り込みがある。開発者がファイル名の一部を入力すると、変更の一覧が、名前にその文字を含むファイルの行だけになる。
- R-003: 面の上部に「…」がある。開発者が「…」を押すと、「Force Push」が出る。開発者が「Force Push」を選ぶと、確認のダイアログが出る。開発者が了承すると、ブランチが `--force-with-lease` で upstream に push される。開発者が取りやめると、push されない。force の push は、`--force-with-lease` を付けたものだけである。

## ブランチ行

- R-004: upstream があるブランチでは、ブランチ行に、ブランチ名、upstream の名前（remote の名前を先頭に付けた、`origin/feature/thread-ui` の形）、先行の commit 数（↑）、遅れの commit 数（↓）が出る。
- R-005: upstream が無いブランチでは、ブランチ行に、ブランチ名、「origin にまだありません（N commit）」、「Publish Branch」が出る。↑ と ↓ は出ない。N は、origin のどのブランチにも無い commit の数である。
- R-006: 開発者が「Publish Branch」を押すと、origin に同じ名前のブランチができ、ブランチの commit が push され、そのブランチが upstream になる。ブランチ行は R-004 の形になる。

## Commit

- R-007: 面に、Commit メッセージの入力欄と、右寄せの「Commit」がある。開発者がメッセージを入力して「Commit」を押すと、stage 済みの変更が、そのメッセージの commit になる。STAGED CHANGES の区画からその行が消え、入力欄が空になり、先行の commit 数が 1 増える。
- R-008: commit の署名は、git の設定に従う。
- R-009: STAGED CHANGES の区画が空の間と、入力欄が空の間、「Commit」は押せない。「Commit」の ▾ のメニューの Commit、Commit & Push、Commit & Sync も、同じ間は選べない。
- R-010: 開発者が「Commit」の ▾ を押すと、Commit、Commit & Push、Commit & Sync、Push、Pull、Sync、Fetch の 7 つが、この順で出る。
- R-011: メニューの項目は、次のとおり動く。
  - Commit は、「Commit」のボタンと同じである。
  - Push は、ブランチの commit を upstream に push する。
  - Pull は、upstream の commit をブランチに取り込む。取り込み方は、git の設定に従う。取り込んだ commit が conflict して止まったときは、worktree が merge か rebase の途中になり、失敗の理由が画面に出る。
  - Sync は、Pull の後に Push する。
  - Fetch は、upstream がある remote の状態を取得する。upstream が無いブランチでは、origin の状態を取得する。worktree のファイルとブランチの commit は変わらず、ブランチ行の遅れの commit 数が、取得した状態のものになる。
  - Commit & Push は、Commit の後に Push する。
  - Commit & Sync は、Commit の後に Sync する。
- R-012: メニューの項目の右に、送る commit 数（↑）と取る commit 数（↓）が出る。Push は ↑、Pull は ↓、Sync は ↓ と ↑ を出す。Commit & Push は ↑、Commit & Sync は ↓ と ↑ を出し、その ↑ は、これから作る commit の 1 を足した数である。Commit と Fetch は数を出さない。
- R-013: upstream が無いブランチで Push すると、「Publish Branch」と同じ結果になる。
- R-014: commit、push、pull、fetch、破棄、merge と rebase の中止と続行が失敗したとき、失敗の理由が画面に出る。

## 変更の一覧

- R-015: 変更の一覧は、STAGED CHANGES と CHANGES の 2 つの区画に分かれる。stage 済みの変更は STAGED CHANGES に、まだ stage していない変更は CHANGES に出る。区画の見出しに、その区画の行の数が出る。
- R-016: 一覧の各行に、状態（M / A / D / U）、ファイル名、ディレクトリ、追加の行数と削除の行数が出る。
- R-017: 同じファイルに stage 済みの変更とまだ stage していない変更があるとき、そのファイルは両方の区画に出る。行数は、それぞれの区画の変更の分である。
- R-018: 開発者が CHANGES の行にマウスを載せると、その行に「破棄」と「stage」が出る。開発者が「stage」を押すと、そのファイルのまだ stage していない変更が stage され、STAGED CHANGES に出る。
- R-019: 開発者が STAGED CHANGES の行にマウスを載せると、その行に「unstage」が出る。開発者が「unstage」を押すと、そのファイルの stage 済みの変更が unstage され、CHANGES に出る。
- R-020: CHANGES の見出しに、区画の全部を破棄する操作と、区画の全部を stage する操作がある。STAGED CHANGES の見出しに、区画の全部を unstage する操作がある。
- R-021: 開発者が一覧の行をクリックすると、右カラムがファイル面に切り替わり、そのファイルのタブが HEAD との差分で開く。

## 破棄の確認

- R-022: 開発者が CHANGES の行の「破棄」を押すと、警告のダイアログが出る。ダイアログには、ファイル名、消える行数、取り消せないことが出る。開発者が了承すると、そのファイルのまだ stage していない変更が消え、行が CHANGES から消える。そのファイルの stage 済みの変更は残る。開発者が取りやめると、何も変わらない。
- R-023: 状態 U のファイルの破棄は、ファイルをゴミ箱に移す。このときのダイアログには、ファイル名と、ファイルをゴミ箱に移すことが出る。取り消せないことは出ない。
- R-024: 開発者が CHANGES の見出しで全部の破棄を選ぶと、警告のダイアログが出る。ダイアログには、消える行数の合計と、取り消せないことが出る。区画に状態 U のファイルがあるときは、そのファイルをゴミ箱に移すことも出る。開発者が了承すると、CHANGES の全部のファイルが R-022 と R-023 のとおり破棄され、CHANGES が空になる。

## 変更が無いとき

- R-025: 変更したファイルが無いとき、変更の一覧の代わりに「Worktree に変更はありません」と出る。
- R-026: 変更したファイルが無く、先行の commit 数が 1 以上のとき、「Push（N commit）」も出る。N は先行の commit 数である。開発者がこれを押すと、Push が行われる。
- R-027: 変更したファイルが無いときも、「Commit」の ▾ のメニューは開き、Push、Pull、Sync、Fetch を選べる。

## merge と rebase の途中

- R-028: merge か rebase の途中のとき、面の上部に帯が出る。帯には、何の途中か（「main への rebase の途中です」の形）、conflict の件数、「中止」が出る。rebase では、進み具合（「3 commit 中 2 つ目」の形）も出る。
- R-029: merge か rebase の途中のとき、ブランチ行には、ブランチ名と、何の途中か（「rebase 中」の形）が出る。upstream の名前、↑、↓ は出ない。
- R-030: conflict があるとき、変更の一覧の先頭に CONFLICTS の区画が出る。見出しに conflict の件数が出る。各行に、ファイル名、ディレクトリ、conflict の種類（「両方で変更」の形）、「+」が出る。
- R-031: 開発者が CONFLICTS の行の「+」を押すと、そのファイルが解決済みとして stage される。行は CONFLICTS から消え、STAGED CHANGES に出る。
- R-032: rebase の途中のとき、「Commit」は「Rebase を続ける」に替わる。merge の途中のとき、「Commit」は「Merge を続ける」に替わる。この間、▾ は出ない。
- R-033: conflict が残っている間、「Rebase を続ける」と「Merge を続ける」は押せず、ボタンの横に「conflict をすべて解決すると続けられます」と出る。conflict が残っていないときは、押せて、この文は出ない。
- R-034: merge か rebase の途中のとき、Commit メッセージの入力欄には、続行で作る commit のメッセージが入っている。開発者は、これを書き換えられる。続行は、入力欄のメッセージで commit を作る。続行で作る commit が無いとき（rebase が `break` か、`exec` の失敗で止まっているとき）は、入力欄は空で、入力できない。このとき「Rebase を続ける」は、メッセージが無くても押せて、commit を作らずに rebase を先へ進める。
- R-035: 開発者が「Rebase を続ける」を押すと、rebase が次の commit に進む。残りの commit が無ければ、rebase が終わり、帯が消え、「Commit」が戻る。次の commit で conflict が起きたときは、帯と CONFLICTS がその内容になり、失敗の理由が画面に出る。
- R-036: 開発者が「Merge を続ける」を押すと、merge の commit ができ、帯が消え、「Commit」が戻る。
- R-037: 開発者が帯の「中止」を押すと、merge か rebase が中止される。ブランチと worktree のファイルは、merge か rebase を始める前の状態に戻り、帯が消える。

# Assumptions

なし

# Invariants

なし

# Risks

- push、pull、fetch、Publish Branch、Force Push の認証に、パスフレーズやパスワードの対話入力が要る環境で、これらの操作が終わるかを確かめていない。終わらない場合、その環境では R-003、R-006、R-011、R-013 を満たせない。
- commit の署名に対話入力が要る設定で、commit が終わるかを確かめていない。終わらない場合、その設定では R-007 と R-008 を満たせない。
