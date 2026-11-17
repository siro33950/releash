# Context

- 要求の正本は、ISSUE #1943「[18] PR の面（作成・本文・Comment・Checks）」の本文である。
- 見た目の参考は、ISSUE #1943 の本文に貼られた mock 2 枚（「PR が無い」「PR がある」）である。mock の PR、Comment、check の名前は例である。
- マイルストーン #78 の「UI の決まり」は、mock を、配置と、出す情報と、できる操作の正としている。
- ISSUE #1943 の完了条件は、回帰の E2E を足すことと、手元で確認した証跡を PR に貼ることである。回帰の E2E が確かめる主たる動作は、PR の面にブランチの PR のタイトル、本文、Comment、checks が出ることである。
- 調査の時点は main `9e9cc211a` である。
- ブランチを origin に push する操作は、ISSUE #1942「Source Control の面」が扱う。調査の時点で #1942 は未着手である。
- この文書で「PR の面」とは、右カラムの切り替えで PR のアイコンを選んだときに、右カラムに出る表示を指す。
- この文書で「ブランチの PR」とは、選んでいる Worktree のブランチを head とする PR のうち、次の順で最初に見つかる 1 件を指す。open の PR、merged の PR、closed の PR の順である。Workspaces のブランチの行が PR のアイコンで示す PR と同じである。
- この文書で「Comment」とは、GitHub の PR に書かれた文章の投稿を指す。Releash の Thread の Comment とは別のものである。
- GitHub の PR には、文章の投稿が 3 種類ある。PR 全体への Comment、レビューを提出するときに付ける本文、コードの行へのレビューの Comment である。この文書は、2 つ目を「レビューの本文」と呼ぶ。
- この文書で「check」とは、GitHub が PR に対して報告する検査の 1 件を指す。check には、GitHub Actions の job と、外部のサービスが報告するものがある。
- check の分類は、gh の分類に従う 5 つである。失敗（`fail`）、取り消し（`cancel`）、実行中（`pending`）、skip（`skipping`）、通過（`pass`）である。
- この文書で「相対表示」とは、「40 分前」「2 時間前」のように、今からさかのぼった長さで時刻を表すことを指す。

# Outcome

対象者は、Swift.app で Worktree のブランチの作業を進め、その PR を GitHub で扱う開発者である。

今の Swift.app は、ブランチの PR について、状態のアイコンと番号だけを Workspaces に出す。
開発者は、PR を作る、PR の本文と Comment を読む、Comment を書く、CI の結果と失敗のログを見る、のすべてをブラウザか Terminal で行っている。

変更後は、右カラムに PR の面がある。
開発者は、PR の面で、ブランチの PR を作り、タイトル、本文、check の結果、Comment を読み、Comment を書き、失敗した check のログを中央のタブで開ける。

# Current Behavior

調査は main `9e9cc211a` の画面のコードを読み、アプリが GitHub から PR の一覧を受け取るときと同じ gh のコマンドを手元で実行して行った。
アプリの画面を見ることはしていない。

## 右カラムの切り替え

- 右カラムの切り替えには、Node、ファイル、Source Control、Thread の 4 つのアイコンがこの順に並ぶ。PR のアイコンは無い（`clients/macos/Sources/RightPanelView.swift:7-26`）。
- Source Control を選ぶと、右カラムの中身は空になる（`RightPanelView.swift:61-69`）。

## Workspaces に出る PR

- Workspaces のブランチの行には、そのブランチの PR の状態が、アイコンと色で出る。Open は `git-pull-request` の緑、Draft は `git-pull-request-draft` の灰、Merged は `git-merge` の紫、Closed は `git-pull-request-closed` の赤である（`clients/macos/Sources/SidebarView.swift:122-127`、`:187-194`）。
- アイコンにマウスを載せると、「PR #番号: 状態」が出る（`SidebarView.swift:126`）。
- 同じブランチを head とする PR が複数あるとき、行に出るのは 1 件である。open の PR があればそれが出て、無ければ merged、それも無ければ closed の PR が出る。
- GitHub のものでないリポジトリのブランチの行には、PR のアイコンは出ない。
- Workspaces が出ているあいだ、行の PR の状態は、30 秒ごとに GitHub の今の状態に替わる。Workspaces の更新のボタンを押したときも替わる（`SidebarView.swift:88`）。
- PR の状態を GitHub から取れないとき、ブランチの行に、警告の印と「PR unavailable: 理由」が出る（`SidebarView.swift:178-182`）。

## PR について出ないもの、できないこと

- PR のタイトル、本文、Comment、レビューの本文、check は、アプリのどの画面にも出ない。
- PR を作る操作、Comment を書く操作、check のログを開く操作は、アプリのどの画面にも無い。
- 調査範囲は `clients/macos/Sources/` の全ファイルである。PR を扱う箇所は、上に挙げた Workspaces のブランチの行（`SidebarView.swift:123`、`:126`、`:181`）だけだった。

## 再現手順

アプリが GitHub から受け取る PR の情報は、ブランチ名、番号、URL、draft かどうかの 4 項目である。
次の手順で、その内容を確かめた。

1. このリポジトリの Worktree で、アプリが merged の PR の一覧を受け取るときと同じコマンドを、件数を 2 件に絞って実行する。

   ```sh
   gh pr list --state merged --json headRefName,number,url,isDraft --limit 2
   ```

2. 実際の出力は次のとおりである。タイトル、本文、Comment、check は含まれない。

   ```json
   [{"headRefName":"fix/archive-history-timestamp","isDraft":false,"number":2145,"url":"https://github.com/siro33950/releash/pull/2145"},{"headRefName":"feat/issues/2126","isDraft":false,"number":2143,"url":"https://github.com/siro33950/releash/pull/2143"}]
   ```

## gh で確かめたこと

- PR 全体への Comment とレビューの本文は、別の項目として取れる。このリポジトリの PR #2142 には、Comment が 8 件、本文があるレビューが 3 件あった（`gh pr view 2142 --json comments,reviews`）。
- 書き手が bot かどうかは、GitHub の API が返す（`gh api repos/{owner}/{repo}/issues/2142/comments` の `user.type` が `Bot`）。
- check の分類は 5 つである（`gh pr checks --help`）。このリポジトリの PR #2145 は、通過が 17 件、skip が 1 件だった。check ごとに、開始と終了の時刻も取れる（`gh pr checks 2142 --json startedAt,completedAt`）。
- 外部のサービスが報告する check は、ログを GitHub に置かない。PR #2142 の `CodeRabbit` の check が持つのは、詳細ページの URL だけである。
- ブランチが origin に push されていないとき、gh は push 先を対話で尋ねる（`gh pr create --help`）。

# Scope / Non-goals

変更するもの:

- 右カラムの切り替え。PR のアイコンを足す。
- PR の面。ブランチの PR が無いときの PR を作る欄と、PR があるときのタイトル、本文、状態、check、Comment、レビューの本文の表示を作る。
- PR の面からの操作。PR を作る、Comment を書く、失敗した check のログを開く。
- 中央のタブ。失敗した check のログを開けるようにする。

Non-goals:

- コードの行へのレビューの Comment を PR の面に出すこと。
- commit の一覧、merge の操作、Reviewers などの情報を PR の面に出すこと。
- ブランチを origin に push すること。ISSUE #1942 が扱う。
- mock の Description の入力欄の右上にあるアイコンと、その操作。
- 外部のサービスが報告する check の詳細ページを、check の行から開く操作。

# Requirements

## 切り替え

- R-001: 右カラムの切り替えの、Source Control と Thread の間に、PR のアイコンが出る。開発者が PR のアイコンを押すと、右カラムに PR の面が出る。
- R-002: PR の面は、選んでいる Worktree のブランチの PR を扱う。開発者が別の Worktree を選ぶと、PR の面は、その Worktree のブランチの PR の表示に替わる。

## PR が無いとき

- R-003: ブランチの PR が無いとき、PR の面に、「このブランチの PR はありません」と、PR を作る欄が出る。PR を作る欄には、Title の入力欄、Description（任意）の入力欄、base の選択、「draft で作る」のチェック、「PR を作成」ボタンが出る。
- R-004: base の選択には、origin にあるブランチが並ぶ。初めに選ばれているのは、リポジトリの既定のブランチである。
- R-005: Title の入力欄が空のあいだ、「PR を作成」は押せない。
- R-006: 開発者が Title を書いて「PR を作成」を押すと、選んでいる Worktree のブランチを head、選んだ base を base とする PR が GitHub にできる。書いた Title が PR のタイトルになり、書いた Description が PR の本文になる。「draft で作る」にチェックがあれば、PR は draft でできる。PR ができると、PR の面は、できた PR の表示（R-010）に替わる。
- R-007: ブランチが origin に無いとき、開発者が「PR を作成」を押しても、PR はできず、ブランチは push されない。PR の面に、ブランチの push が要ることが出る。書いた Title と Description は残る。
- R-008: R-007 以外の理由で PR の作成が失敗したとき、PR の面に、失敗の理由が出る。書いた Title と Description は残る。
- R-009: リポジトリが GitHub のものでないとき、PR の面は、ブランチの PR が無いとき（R-003）と同じ表示になる。開発者が「PR を作成」を押すと、R-008 のとおり失敗の理由が出る。

## PR があるとき

- R-010: ブランチの PR があるとき、PR の面に、上から順に、見出し、本文、CHECKS、COMMENTS が出る。下端には、Comment の入力欄と「Comment」ボタンが出る。
- R-011: 見出しには、状態のアイコン、PR のタイトル、PR の番号、ブラウザで開くアイコンが出る。状態は文字で出ず、次のアイコンと色で出る。

  | 状態 | アイコン（Lucide） | 色 |
  | --- | --- | --- |
  | Open | `git-pull-request` | 緑 |
  | Draft | `git-pull-request-draft` | 灰 |
  | Merged | `git-merge` | 紫 |
  | Closed | `git-pull-request-closed` | 赤 |

- R-012: 開発者がブラウザで開くアイコンを押すと、その PR の GitHub のページが既定のブラウザで開く。
- R-013: PR の本文は、Markdown として整形されて出る。
- R-014: ブランチの PR が Closed のとき、PR の面に、新しい PR を作る操作が出る。開発者がこの操作を押すと、PR の面に PR を作る欄（R-003）が出る。この欄から PR を作ると、PR の面は、できた PR の表示に替わる。Open、Draft、Merged のとき、この操作は出ない。

## CHECKS

- R-015: CHECKS の見出しの右に、失敗、取り消し、実行中、skip、通過の分類ごとの check の件数が出る。check が 0 件の分類の件数は出ない。
- R-016: 失敗、取り消し、実行中の check は、1 件につき 1 行で出る。行には、分類を表すアイコンと check の名前が出る。失敗の行には、失敗までにかかった時間が「4 分で失敗」の形で出る。実行中の行には、実行している時間が「実行中 6 分」の形で出る。
- R-017: 通過の check は、「通過した N 件」の 1 行にたたまれて出る。skip の check は、「skip した N 件」の 1 行にたたまれて出る。開発者がたたまれた行を押すと、その分類の check の名前が並ぶ。もう一度押すと、1 行に戻る。
- R-018: 失敗の行のうち、GitHub Actions の job の行には、「ログを開く」が出る。開発者が「ログを開く」を押すと、中央のタブに、その check のログが開く。外部のサービスが報告する check の失敗の行には、「ログを開く」は出ない。

## COMMENTS

- R-019: COMMENTS には、PR 全体への Comment と、本文があるレビューの本文が、書いた時刻の古い順に混ざって並ぶ。COMMENTS の見出しの右に、並んだ件数が出る。
- R-020: COMMENTS の各項目には、書き手のアイコン、書き手の名前、書いた時刻の相対表示、本文が出る。本文は Markdown として整形されて出る。書き手が人のとき、アイコンは名前の頭文字である。書き手が bot のとき、アイコンは bot を表す絵であり、名前の右に「bot」の印が出る。
- R-021: Comment の入力欄が空のあいだ、「Comment」は押せない。
- R-022: 開発者が Comment の入力欄に文を書いて「Comment」を押すと、その文が PR 全体への Comment として GitHub に投稿され、COMMENTS の最後に出る。入力欄は空に戻る。
- R-023: Comment の投稿が失敗したとき、PR の面に、失敗の理由が出る。書いた文は入力欄に残る。

## 内容の更新と取得の失敗

- R-024: PR の面が出ているあいだ、GitHub で変わった PR の状態、タイトル、本文、check、Comment、レビューの本文は、開発者が操作しなくても、30 秒の周期で PR の面に反映される。
- R-025: PR の内容を GitHub から取れないとき、PR の面に、取れない理由が出る。

# Assumptions

なし

# Invariants

なし

# Risks

- GitHub Actions の job のログを gh で取れることは、`gh run view --help` の記載で確かめただけであり、失敗した job に対して実行していない。ログが取れない、または取得に長い時間がかかる場合、R-018 を満たせない。
