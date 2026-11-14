# Context

- 要求の正本は、ISSUE #2115「Swift.app: 設定画面を、左にナビゲーション・右に詳細を並べる形にする」の本文と、本文に貼られた mock 8 枚である。
- mock に出ている定義や Facet の名前、実行ファイルのパスなどの値は例である。
- 調査の時点は main `373fb61b` である。ISSUE の本文の「現状」は main `20278b88` の時点で書かれている。
- ISSUE の本文は、アイコンに #2111 で入れた Lucide のアイコンを使うと定めている。#2111 は main に入っている。
- Repository の名前の決め方（R-008）は、ISSUE #2125 の本文が新しい Worktree のダイアログ向けに書いている決め方と同じである。
- ISSUE の本文は、完了条件として次の 2 つを挙げている。
  - 回帰の E2E で、設定画面のタブを探している箇所を、ナビゲーションの項目に合わせて変える。該当する箇所は `clients/macos/UITests/SettingsTests.swift` と `clients/macos/UITests/UpdateTests.swift` にある。
  - 手元で確認し、証跡を PR に貼る。
- この文書では、語を次の意味で使う。
  - ナビゲーション: 設定画面の左に並ぶ、General・Repositories・Providers・Workflows・System の項目の列。
  - ページ: ナビゲーションの項目を選んだときに、設定画面の右に出る内容。
  - まとまり: ページの中で、関係する設定の行を一つの枠に入れたもの。

# Outcome

対象者は、Swift.app の設定画面で設定を確かめ、変える開発者である。

今の設定画面は 560×560 の大きさで、上部のタブでページを切り替える。
Repositories は、この大きさの中に Repository の一覧と設定を左右に並べるので、どちらも狭い。
General は、関係の薄い設定が見出し無しで一つのまとまりに入っている。
どの項目にも説明の文が無く、「Diff base の既定」や「有効な実行ファイル」のように、項目名だけでは何が変わるか分からない項目がある。

変更後は、設定画面が 1024×800 になり、左のナビゲーションでページを選び、右でそのページの設定を読む。
各ページは、設定を見出し付きのまとまりに分けて並べる。
Repositories と Workflows は、一覧から行を選んで詳細のページに入る。
意味が分かりにくい項目には、項目名の下に説明の文が付く。

# Current Behavior

調査は main `373fb61b` のコードと回帰の E2E を読んで行った。
アプリを起動して画面を見ることはしていない。

## 再現手順

1. Swift.app を起動する。
2. ⌘, を押して設定画面を開く。
3. 上部のタブを順に選ぶ。

## 設定画面の形

- 設定画面の大きさは 560×560 で固定されている。
- 上部に、General・Repositories・Providers・Workflows・System のタブがアイコン付きで並ぶ。アイコンは Lucide のアイコンである。
- タブを選ぶと、タブの下にそのページが出る。開いた直後は General が選ばれている。
- どのページのどの項目にも、項目名の下の説明の文は無い。

## General

- 見出しの無いまとまりが 2 つある。
- 1 つ目のまとまりに、外部エディタ、Terminal の起動コマンド、承認の自動化が並ぶ。
- 2 つ目のまとまりに、Diff base の既定がある。
- Terminal の起動コマンドは、入力を確定したとき、入力欄から離れたとき、ページを離れたときに保存される。

## Repositories

- ページの左に Repository の一覧、右に選んだ Repository の設定が並ぶ。
- 一覧の行は、Repository のフルパスだけを出す。
- 開いた直後は、一覧の最初の Repository が選ばれている。
- 選んだ Repository の設定は、次の 3 つのまとまりである。
  - 見出しの無いまとまり: パスと「Repositoryを一覧から削除」のボタン。ボタンを押すと確認が出て、「ディスク上のRepositoryは削除されません。」と表示する。
  - 「Base branch」: Base branch の選択。
  - 「Notion」: API token、Database ID、「接続テスト」のボタン、接続の状態を表す文。接続テストに成功すると、Title の選択、プロパティごとの「Labels」のトグル、Branch 名の選択、Branch 名の prefix、「Notion設定を保存」のボタンが出る。保存済みの設定があると、「Notion設定を削除」のボタンが出る。

## Providers

- provider ごとに、provider の名前を見出しにしたまとまりが 1 つある。
- まとまりには、可用性、既定の実行ファイル、有効な実行ファイル、実行ファイルの上書き、「リセット」のボタンが並ぶ。provider を利用できるときは解決された実行ファイルが、利用できないときは利用できない理由が加わる。
- provider のまとまりの下に、「可用性を再確認」のボタンがある。

## Workflows

- 一覧のページに、「定義 / Facet」の切り替え、一覧、「定義のフォルダを外部エディタで開く」のボタンが並ぶ。Facet を選んでいるときは、種類（policy・knowledge・instruction）の切り替えも出る。
- 定義の一覧の行は、名前、説明、builtin の印、実行中の印、定義の形式、その定義の診断の件数（error と info）を出す。読み込みに失敗した定義は、その理由も出す。
- Facet の一覧の行は、key、説明、builtin の印、その Facet の診断の件数（error と info）を出す。
- 診断の全体の件数は、一覧のページに出ない。
- 一覧の行を選ぶと、詳細のページに入る。詳細のページの上部には、定義の名前か Facet の key と、一覧に戻るボタンが出る。ISSUE の本文の「現状」は一覧と中身を 1 つのタブに並べると書いているが、調査の時点では詳細は別のページである。
- 定義の詳細のページは、説明、診断、Node を出す。Node は行ごとに折りたたまれていて、開くと中身が出る。builtin でない定義では、説明の下に「外部エディタで開く」のボタンが出る。
- Facet の詳細のページは、key、種類、説明、診断、参照元を出す。builtin でない Facet では、説明の下に「外部エディタで開く」のボタンが出る。
- 定義の詳細の Node から Facet の詳細へ、Facet の詳細の参照元から定義の詳細へ移れる。
- builtin の定義と Facet は、外部エディタで開けない。開こうとすると、開けないことを伝える失敗になる。

## System

- 「更新」「ログイン項目」「CLIの設置」「Telemetry」の 4 つのまとまりがある。
- 「更新」に自動更新と「nightly アップデートを受け入れる」、「ログイン項目」に「ログイン時に起動」と「最小化状態で起動」、「CLIの設置」に設置先のパスと「CLIを設置」のボタン、「Telemetry」に Performance telemetry と Crash reporting が並ぶ。

# Scope / Non-goals

変更するもの:

- 設定画面の大きさと、ページを切り替える形。
- 各ページの、まとまりの分け方と行の並べ方。
- Repositories と Workflows の、一覧と詳細の出し方。
- Repositories の一覧と詳細に出す、Repository の名前。
- 項目名の下に添える説明の文。

Non-goals:

- ナビゲーションの検索欄。
- mock の Repositories の一覧の下にある「Repository を追加…」のボタン。Repository の追加をどこに置くかは別に決める。

# Requirements

- R-001: 設定画面の大きさは 1024×800 である。
- R-002: 設定画面は、macOS のシステム設定と同じ形で、左にナビゲーション、右にページを並べる。ナビゲーションは、General・Repositories・Providers・Workflows・System を、この順に、Lucide のアイコン付きで並べる。
- R-003: ナビゲーションの項目を選ぶと、右にその項目のページが出る。
- R-004: ウィンドウの上部に、選んだ項目の名前は出ない。
- R-005: 各ページは、設定を見出し付きのまとまりに分け、まとまりの中に行で並べる。各行は、左に項目名、右に値か操作を置く。
- R-006: General のページは、「エディタ」「Terminal」「Workflow」の 3 つのまとまりを持つ。「エディタ」に外部エディタと Diff base の既定、「Terminal」に Terminal の起動コマンド、「Workflow」に承認の自動化を置く。
- R-007: Repositories のページは、Repository の一覧を出す。一覧の各行は、Repository の名前とパスを出す。
- R-008: Repository の名前は、次のとおりに決まる。
  - Repository の origin が GitHub のリポジトリなら、GitHub 上のリポジトリ名（例: `releash`）。
  - それ以外は、Repository のフォルダ名。
  - 同じ名前の Repository が並ぶときだけ、origin が GitHub の Repository の名前に、`siro33950/releash` のように owner を付ける。
- R-009: Repositories の一覧の行を選ぶと、その Repository の詳細のページに入る。詳細のページは、上部にその Repository の名前を出す。左上のボタンで一覧に戻る。
- R-010: Repository の詳細のページは、パスと Base branch、Notion の連携、一覧からの削除の 3 つのまとまりを持つ。Notion の連携のまとまりは、接続テストに成功すると、新しい Worktree のダイアログで絞り込みに使うプロパティを選ぶ「Labels」のトグルを、プロパティごとに出す。
- R-011: Providers のページは、provider ごとに 1 つのまとまりを持ち、いちばん下に「可用性を再確認」を置く。
- R-012: Workflows のページは、一覧のページとして、「定義 / Facet」の切り替え、一覧、「定義のフォルダを外部エディタで開く」のボタン、診断の error と info の件数を出す。一覧の各行は、名前と、当てはまる印を出す。印は、実行中、builtin、その行の診断の error の件数である。
- R-013: Workflows の一覧の行を選ぶと、定義の行なら定義の詳細のページに、Facet の行なら Facet の詳細のページに入る。定義の詳細と Facet の詳細は別のページである。詳細のページは、上部に定義の名前か、Facet の種類と key を出す。左上のボタンで一覧に戻る。builtin でない定義と Facet の詳細のページは、右上に「外部エディタで開く」を置く。builtin の定義と Facet は外部エディタで開けないので、その詳細のページに「外部エディタで開く」は出ない。
- R-014: System のページは、「更新」「ログイン項目」「CLI の設置」「Telemetry」の 4 つのまとまりを持つ。
- R-015: 次の表の項目は、項目名の下に、灰色の説明の文を出す。

  | ページ | 項目 | 説明 |
  | --- | --- | --- |
  | General | Diff base の既定 | ファイルタブで差分を開いたときに、最初に比べる相手です。 |
  | General | Terminal の起動コマンド | 新しい Terminal のタブで shell が起動した直後に、このコマンドを入力して実行します。空なら何もしません。 |
  | General | 承認の自動化 | 承認を求める Node を、人の確認なしに完了させます。 |
  | Repositories | Base branch | ファイルタブで「base と比較」を選んだときに、比べる相手になるブランチです。 |
  | Repositories | Labels | 新しい Worktree のダイアログで、Notion task を絞り込むのに使います。 |
  | Providers | 有効な実行ファイル | 上書きがあればその値、無ければ既定の実行ファイルです。 |
  | Providers | 解決された実行ファイル | 有効な実行ファイルを PATH から探した結果です。 |
  | Providers | 実行ファイルの上書き | 既定と別の実行ファイルを使うときに指定します。 |
  | System | nightly アップデートを受け入れる | 毎日作られる開発中の版も、更新の対象にします。 |
  | System | 最小化状態で起動 | ログイン時に起動したときは、ウィンドウを開きません。 |
  | System | Performance telemetry | サーバの動作の計測値（metrics と traces）を送ります。 |
  | System | Crash reporting | 異常終了したときに、その記録を送ります。 |

- R-016: 各ページは、次の設定と操作を持つ。設定した値は保存され、設定画面を開き直しても保たれる。
  - General: 外部エディタの選択、Diff base の既定の選択、Terminal の起動コマンドの入力、承認の自動化の切り替え。
  - Repository の詳細: Base branch の選択、Notion の API token と Database ID の入力、接続テスト、Title の選択、Labels の切り替え、Branch 名の選択、Branch 名の prefix の入力、Notion 設定の保存と削除、Repository の一覧からの削除。
  - Providers: 実行ファイルの上書きの入力とリセット、可用性の再確認。
  - Workflows: 定義と Facet の切り替え、Facet の種類の切り替え、定義のフォルダを外部エディタで開く操作、定義と Facet を外部エディタで開く操作。
  - System: 自動更新の切り替え、nightly アップデートを受け入れるかの切り替え、ログイン時に起動の切り替え、最小化状態で起動の切り替え、CLI の設置、Performance telemetry の切り替え、Crash reporting の切り替え。

# Assumptions

なし

# Invariants

なし

# Risks

なし
