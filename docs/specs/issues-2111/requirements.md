# Context

- 要求の正本は、ISSUE #2111「Swift.app: アイコンを Lucide に揃え、中央のタブのアイコンを ● にする」の本文と、本文に貼られた対応表の画像（<https://github.com/user-attachments/assets/e3c89931-13b3-4703-9ee1-249123782384>）である。
- 調査の時点は main `20278b88` である。
- Lucide は ISC ライセンスのアイコン集である。公式の配布物 `lucide-static` は、アイコンを 1 個ずつの SVG で収める。
- ISSUE は、アイコンの入手元を公式の `lucide-static` 1.71.0 の SVG と定め、非公式の Swift パッケージを使わないと定めている。
- Node の状態の 3 値（attention、active、idle）と色の仕様は `docs/specs/issues-1958/`、Node 面の仕様は `docs/specs/issues-1762/` が持つ。
- Node の種別は Session、Command、Sequence、Fanout の 4 つである（`docs/glossary/DOMAIN.md`）。
- 中央のタブは、依存ライブラリ Bonsplit が描く。Bonsplit は、アプリが渡した名前を SF Symbols として描く。そのため、アプリに同梱した画像を中央のタブのアイコンに出せない。

# Outcome

対象者は、Swift.app で Workflow と Session を観測する開発者である。

今の画面は、アイコンのほとんどを SF Symbols で出し、サイドバーの Session と Workflow の行だけを Lucide で出している。
2 種類の絵柄が同じ画面に混ざっている。
Node 面の行と詳細は、先頭に状態の色の丸を出すだけで、Node の種別は文字のラベルを読まないと分からない。

変更後は、中央のタブを除く画面のアイコンが Lucide に揃う。
Node 面の行と詳細は、Node の種別をアイコンの形で、状態をアイコンの色で示す。
中央のタブのアイコンは、タブの種類によらず ● になる。

# Current Behavior

調査は main `20278b88` のコードを読んで行った。
アプリを起動して画面を見ることはしていない。

## アイコンを出している場所

SF Symbols のアイコンが出る場所は、次のとおりである。

| 場所 | 対象 | 今の SF Symbols |
| --- | --- | --- |
| 右カラムの切り替え | Node / ファイル / Source Control / Thread | `list.bullet` / `doc` / `arrow.triangle.branch` / `bubble.left` |
| 右カラムの開閉 | | `sidebar.right` |
| Node 面の行と詳細 | 状態の丸 | `circle.fill` |
| Node 面の過去の試行 | | `clock.arrow.circlepath` |
| Node 面のメニュー | その他 / 開始 | `ellipsis` / `plus` |
| 中央のタブの並びの「開く」メニュー | メニュー / 項目「Terminal」 | `plus` / `terminal` |
| サイドバーの操作 | リポジトリを追加 / 再読み込み / Worktree を作成 | `folder.badge.plus` / `arrow.clockwise` / `plus` |
| worktree のカード | main / ブランチ | `house` / `arrow.triangle.branch` |
| worktree のカード | Merged の印 | `checkmark.circle` |
| worktree のカード | PR の状態 open / draft / merged / それ以外 | `arrow.triangle.pull` / `ellipsis.circle` / `arrow.triangle.merge` / `xmark.circle` |
| Thread | 解決済み | `checkmark.circle` |
| ファイルタブ | HEAD と比較 / base と比較 | `circle.and.line.horizontal` / `arrow.triangle.branch` |
| ファイルタブ | 上下に並べる / 左右に並べる | `rectangle.split.1x2` / `rectangle.split.2x1` |
| ファイルタブ | 全文 / Diff 表示 / 外部エディタで開く | `rectangle.expand.vertical` / `plus.forwardslash.minus` / `arrow.up.forward.app` |
| フッター | hook の状態を確認できないときの警告、provider の警告 | `exclamationmark.triangle.fill` |
| フッター | provider（Claude / Codex） | `sparkles` / `terminal` |
| フッター | サーバのメニュー（版が合っている / 合っていない） | `server.rack` / `exclamationmark.triangle` |
| 「サーバに接続できません」の画面 | | `exclamationmark.triangle` |
| Settings のタブ | General / Repositories / Providers / Workflows / System | `gearshape` / `folder` / `terminal` / `arrow.triangle.branch` / `desktopcomputer` |
| Settings の Repositories タブ | 「リポジトリを選択してください」 | `folder` |
| 中央のタブ | Terminal / ファイル / Session / Command | `terminal` / `doc` / `sparkle` / `terminal` |

`arrow.triangle.branch` は、Source Control、ブランチの worktree、base と比較、Settings の Workflows タブの 4 か所に、別々の意味で出る。

## Lucide のアイコン

- Lucide のアイコンが出るのは、サイドバーの Session の行（Bot）と Workflow の行（Waypoints）だけである。
- この 2 個の形は、`lucide-static` 1.71.0 の `bot`、`waypoints` と同じである。
- 対応表が挙げる 30 個のアイコンと、`sparkles`、`server` は、`lucide-static` 1.71.0 に同じ名前で収められている。
- アプリは Lucide のライセンスの表記を同梱している。その内容は、`lucide-static` 1.71.0 の `LICENSE` と 1 文字も違わない。`LICENSE` は、ISC の全文と、Feather 由来のアイコンに適用する MIT の全文を含む。

## アイコンの色

- サイドバーの Session の行と Workflow の行のアイコンは、その実行の状態の色で出る。
- フッターの、hook の状態を確認できないときの警告と provider の警告は、黄で出る。
- worktree のカードの Merged の印と PR の状態は、副次的な文字の色で出る。
- それ以外のアイコンは、置かれた場所の文字と同じ色で出る。右カラムの切り替えとファイルタブの切り替えでは、選んでいる項目のアイコンが、選択中の文字の色で出る。

## Node 面

- 行の先頭には、状態の色の丸が出る。色は attention が黄、active が青、idle が緑である。
- 行は、Node の種別を文字のラベルで出す。ラベルは `Session`、`Command`、`SEQ`、`FANOUT` である。
- 詳細の先頭にも、状態の色の丸が出る。詳細を開けるのは Session と Command の行だけで、詳細のラベルは `Session` か `Command` である。

## 中央のタブ

- タブのアイコンは、タブの種類ごとに違う SF Symbols である。
- Session のタブは、Node の状態を受け取ると、アイコンにその状態の色が付く。Node の状態を受け取っていない Session のタブと、Terminal、ファイル、Command のタブには、状態の色が付かない。

## SF Symbols を使っていない印

- Artifact があることを示す ◆ と、タブを開くボタンの ▷ は、文字である。
- サイドバーの、Workflow 配下の Session の状態を並べた四角は、図形である。
- メニューバーのメニューの、実行ごとの状態の丸は、図形である。
- メニューバーの Releash のアイコンは、Releash 独自の画像である。

## 再現手順

1. Swift.app を起動し、サイドバーで worktree を選ぶ。
2. 右カラムの Node 面で Session を開始する。
3. Node 面の行の先頭に状態の色の丸が出る。中央の Session のタブに `sparkle` のアイコンが出る。右カラムの切り替え、サイドバーの操作、フッターのアイコンは SF Symbols で出る。

# Scope / Non-goals

変更するもの:

- 中央のタブを除く、SF Symbols で出しているアイコン。Lucide のアイコンに替える。
- Node 面の行と詳細の、状態の丸。Node の種別のアイコンに替える。
- サイドバーの Session と Workflow の行のアイコン。`lucide-static` 1.71.0 のものに取り直す。
- 中央のタブのアイコン。● にする。

Non-goals:

- Bonsplit の変更。Bonsplit には手を入れず、Bonsplit が自分で描くアイコン（タブを閉じるボタンなど）は今のままにする。
- Artifact の ◆、タブを開くボタンの ▷、サイドバーの Session の状態の四角、メニューバーのメニューの状態の丸、メニューバーの Releash のアイコン。

# Requirements

- R-001: 次の表の場所のアイコンは、表の Lucide のアイコンで表示される。

  | 場所 | 対象 | Lucide のアイコン |
  | --- | --- | --- |
  | サイドバーの実行の行 | Session / Workflow | `bot` / `waypoints` |
  | サイドバーの操作 | リポジトリを追加 / 再読み込み / Worktree を作成 | `folder-plus` / `refresh-cw` / `plus` |
  | worktree のカード | main / ブランチ | `house` / `git-branch` |
  | worktree のカード | Merged の印 | `circle-check` |
  | worktree のカード | PR の状態 open / draft / merged / closed | `git-pull-request` / `git-pull-request-draft` / `git-merge` / `git-pull-request-closed` |
  | 右カラムの切り替え | Node / ファイル / Source Control / Thread | `list` / `file` / `git-branch` / `message-square` |
  | 右カラムの開閉 | | `panel-right` |
  | Node 面の過去の試行 | | `history` |
  | Node 面のメニュー | その他 / 開始 | `ellipsis` / `plus` |
  | 中央のタブの並びの「開く」メニュー | メニュー / 項目「Terminal」 | `plus` / `terminal` |
  | Thread | 解決済み | `circle-check` |
  | ファイルタブ | Diff 表示 / 外部エディタで開く / 全文 | `diff` / `square-arrow-out-up-right` / `unfold-vertical` |
  | ファイルタブ | HEAD と比較 / base と比較 | `git-commit-horizontal` / `git-branch` |
  | ファイルタブ | 上下に並べる / 左右に並べる | `rows-2` / `columns-2` |
  | フッター | hook の状態を確認できないときの警告、provider の警告 | `triangle-alert` |
  | フッター | provider（Claude / Codex） | `sparkles` / `terminal` |
  | フッター | サーバのメニュー（版が合っている / 合っていない） | `server` / `triangle-alert` |
  | 「サーバに接続できません」の画面 | | `triangle-alert` |
  | Settings のタブ | General / Repositories / Providers / Workflows / System | `settings` / `folder` / `terminal` / `waypoints` / `monitor` |
  | Settings の Repositories タブ | 「リポジトリを選択してください」 | `folder` |

- R-002: Node 面の行と詳細は、状態の丸の代わりに、その Node の種別のアイコンを表示する。Session は `bot`、Command は `terminal`、Sequence は `waypoints`、Fanout は `git-fork` である。種別のアイコンの大きさは 8 ポイントである。
- R-003: Node 面の行と詳細の種別のアイコンの色は、その Node の状態の色である。attention は黄、active は青、idle は緑である。
- R-004: R-001 のアイコンの色は、次のとおりである。サイドバーの Session の行と Workflow の行のアイコンは、その実行の状態の色（attention は黄、active は青、idle は緑）である。フッターの、hook の状態を確認できないときの警告と provider の警告は、黄である。worktree のカードの Merged の印と PR の状態は、副次的な文字の色である。それ以外のアイコンは、置かれた場所の文字と同じ色である。
- R-005: 中央のタブのアイコンは、タブの種類（Terminal、ファイル、Session、Command）によらず ● である。
- R-006: Session が始まっていて Node の状態を受け取っている Session のタブの ● は、その状態の色で表示される。それ以外のタブの ● には、状態の色が付かない。Session を準備しているあいだのタブ（「Sessionを準備しています」と表示するタブ）は、それ以外のタブに含まれる。
- R-007: 画面に出る Lucide のアイコンの形は、`lucide-static` 1.71.0 の同じ名前のアイコンと同じである。
- R-008: アプリは Lucide のライセンスの表記を同梱する。その内容は、`lucide-static` 1.71.0 の `LICENSE` と同じである。

# Assumptions

- 対応表の「Settings のタブ」の行にある「Terminal」は、Settings の Providers タブを指す。

# Invariants

なし

# Risks

- メニューの項目、フッターのサーバのメニュー、「サーバに接続できません」の画面は、macOS の標準部品がアイコンを描く。同梱した画像が、これらの部品で SF Symbols と同じ大きさと色で出るかは確かめていない。出ない部品があると、その場所で R-001 か R-004 を満たせない。
