# Context

- 入力: https://github.com/siro33950/releash/issues/1760
- 補助資料:
  - https://github.com/siro33950/releash/issues/1204（SwiftUI クライアントの器）
  - https://github.com/siro33950/releash/issues/2013（ファイルタブの編集）
  - https://developer.apple.com/forums/thread/662937 と https://developer.apple.com/forums/thread/832946（SwiftUI の標準の木の遅さの報告）
  - https://github.com/jamesrochabrun/PierreDiffsSwift
  - https://diffs.com
  - mock: https://github.com/user-attachments/assets/4dd336fa-dada-489d-9898-ff292e42b07d
- この開発は main の `3bc6dd04` を基点にする。
  #1760 本文の「現状」は main `efda9bb7` を基準にしている。
- [03] #1759 が並行して進んでいる。
  #1759 も proto の `PaneTab`、Swift の `PaneWorkspace` と `AppModel`、UI 状態の保存形式を変える。
- ファイルタブの中身は `@pierre/diffs` を WKWebView に載せて作る。
  確認した版は 1.5.2（npm、2026-10-10 公開）。
  1.5.2 の型定義（`dist/**/*.d.ts`）に画像を扱う API は無い。
  差分は、前後の全文からライブラリが hunk を計算する方法と、外から渡した hunk を表示する方法の両方を持つ。
- マイルストーン #78 の決まりのうち、この開発に関わるもの:
  - 画面側に自前のロジックを実装しない。ライブラリで済む処理はライブラリに任せてよい。
  - コードと差分の表示の中は、ライブラリのテーマに従い、明暗はシステムの設定に合わせる。
  - 部品は SwiftUI の標準の部品と標準の style を使い、標準に無いものだけを自作する。
- #1760 本文と異なる決定:
  - キー操作: 本文は「j / k・n / p・s・c」を挙げる。
    この開発ではファイルタブのキー操作を作らない。
    hunk の stage と Thread の作成は、マウスで行う。
  - 「HEAD 比較」の意味: 本文は帯に「HEAD / base 比較」を置くとだけ書く。
    この開発では、HEAD 比較を HEAD → 作業ツリーのすべての変更（`git diff HEAD` と同じ）とする。
  - 大きいファイルの上限: 本文は「数万行のファイルでの速さ」を着手時に確かめるとだけ書く。
    この開発では、ファイルタブの全文と差分に表示の上限を付けない。
    今の上限（1 MiB・5,000 行・300 hunk・10 万文字）は React の差分表示のために作られたもので、1 タブに 1 ファイルを見える範囲だけ描くファイルタブには当てはまらない。
  - Diff の既定: 本文は base、上下 / 左右、変更箇所のみ / 全文の既定を UserDefaults に置くとする。
    この開発では、base の既定は HEAD に固定し、変える項目は [10] #1763 の Settings 画面に置く。
  - 証跡: 本文は手元で確認した証跡を PR に貼るとする。
    証跡を PR に載せる仕組みは利用者の決定で削除済みのため、この開発の要求に含めない。
- 標準として参照した振る舞い:
  - VS Code: 左右とインラインの切替（`diffEditor.renderSideBySide`）と、変更の無い範囲を畳む設定（`diffEditor.hideUnchangedRegions`）は、すべてのエディタに効く設定値である。
    `explorer.excludeGitIgnore` の既定は false で、除外されたものも木に出して薄く示す。
    変更を含むディレクトリは、中のファイルの状態の色で示す。
    ファイルを開くときは focus 中の editor group で開き、その group に同じファイルがあればそれを選ぶ（`workbench.editor.revealIfOpen` の既定は false）。
  - GitHub の PR の差分: 解決済みの Thread は畳んだ状態で行の間に残る。

# Outcome

対象者: Releash の macOS アプリ（SwiftUI クライアント）の利用者。

現在の問題: ネイティブのアプリには、ファイルの中身と差分を読む画面が無い。
中央の「ファイル」タブは中身の無い枠で、右の「ファイル」の面は見出しだけである。
変更の無いファイルの中身を取る手段と、作業ツリー全体のファイル一覧を取る手段が、サーバに無い。

変更後の状態: 利用者は、右の「ファイル」の面で worktree のファイルを木でたどり、git の状態を色で見分けられる。
ファイルを開くと中央のファイルタブに中身が出て、変更があれば差分が出る。
利用者はファイルタブの中で、hunk を stage / unstage し、行に Thread を作り、返信して解決できる。
開いていたファイルタブは、開き直したときに同じ表示で戻る。

# Current Behavior

開発開始時点（`3bc6dd04`）で確認した状態。

macOS アプリ（`clients/macos/`）:
- 中央の pane の「+」メニューから「ファイル」を選ぶと、タイトルが「ファイル」のタブが増える（`Sources/PaneOpenMenu.swift:17-19`、`Sources/PaneWorkspace.swift:84-88`）。
  タブの中にはタイトルの文字だけが出る（`Sources/PaneWorkspace.swift:120-123`）。
- 右の面は「ファイル」「Source Control」「Thread」「タスク」の切替ボタンを持ち、選んだ面の見出しだけを出す（`Sources/RightPanelView.swift:5-25`、`:57-62`）。
- pane の配置はサーバに保存される（`SaveWorkspaceState`）。
  保存するタブ（proto `PaneTab`）が持つのは id と kind だけで、ファイルの path を持つ欄は無い（`proto/client.proto:1921-1931`）。
- Diff の既定を持つ場所は無い。
  React の画面は、base・表示形式・変更箇所のみの既定を localStorage に置き、Settings の画面で変える（`clients/desktop/src/types/settings.ts:9-15`、`clients/desktop/src/hooks/useSettings.ts:20-45`）。

サーバ（`server/src/`）:
- 変更のあるファイルの内容は購読 `review-file-view`（引数は worktree の path、ファイル、section、base）で届く（`adaptor/presenter/state_subscription_target.rs:102-107`）。
  前後の全文、hunk、stage の単位（change group）が届く（proto `ReviewTextDiffDto`、`proto/client.proto:1034-1047`）。
- `review-file-view` の比較は次の 3 つだけである（`usecase/code_usecase.rs:91-120`）。
  - base=head、section=changes: index → 作業ツリー
  - base=head、section=staged: HEAD → index
  - base=branch-base: base → 作業ツリー
  HEAD → 作業ツリーを返す比較は無い。
- `review-file-view` は、変更の一覧に無い path を拒否する（`usecase/review_usecase.rs:211`、`:770-815`）。
  変更の無いファイルの内容を取る手段は無い。
- `review-file-view` は、1 MiB、5,000 行、300 hunk、10 万文字のどれかを超えると、中身を返さず理由と数値だけを返す（`domain/code/value_objects/review.rs:107-147`、`usecase/review_usecase.rs:241-257`、`:474-517`）。
  画像は前後の data URL を、画像以外のバイナリは前後の大きさだけを返す（`usecase/review_usecase.rs:259-284`）。
- 比較元の選択（`usecase/code_usecase.rs:91-120`）、表示の分類（`usecase/review_usecase.rs:241-284`、`:880-887`）、変更の一覧にあるかの判定（`usecase/review_usecase.rs:770-815`）は、usecase に置かれている。
- hunk の stage / unstage は、`GitStageReviewGroup` / `GitUnstageReviewGroup` が section・base・group_id を受け、サーバが patch を作って当てる（`usecase/review_usecase.rs:313-351`）。
- 作業ツリー全体のファイル一覧と、ディレクトリの直下を列挙する RPC・購読は無い。
  `ignore` / `walkdir` crate への直接の依存は無い。
  ファイルの変更の監視はサーバの中にある（`adaptor/gateway/repository/file_watcher.rs`）。
- Thread の作成・追記・解決は RPC が既存である（`CreateReviewThread`、`AppendReviewComment`、`ResolveReviewThread`）。
  worktree の Thread は購読 `review-threads` で届く（`adaptor/presenter/state_subscription_target.rs:108`）。
- 変更のあるファイル間の移動（`GetFileNavigation`）と外部エディタで開く RPC（`OpenInEditor`）は既存である（`proto/client.proto:2603`、`:2615`）。

# Scope / Non-goals

変更するもの:
- 中央のファイルタブ（File 表示、Diff 表示、画像、表示できないファイル）
- 右の「ファイル」の面（ファイルの木）
- ファイルタブの保存と復元
- Diff の表示形式の設定値の保持
- サーバ: ディレクトリの直下の一覧、ファイルの全文、HEAD → 作業ツリーの比較、hunk ごとの stage 状態、ファイルタブに上限を付けずに返す経路、ディレクトリの git 状態
- サーバ: 比較元の選択、表示の分類、変更の一覧にあるかの判定を domain へ移す。
  移すのはこの 3 つの判断だけで、周りの既存のコードは作り直さない。
- 回帰の E2E

変更しないもの:
- ファイルの編集と保存（#2013）
- ファイルタブのキー操作
- Thread の削除
- 右の「Thread」の面（[06] #1761）と「Source Control」の面（[07] #1942）
- Diff の既定を変える Settings の項目（[10] #1763）
- markdown を整形した見た目で出す表示
- プレビューのタブ（次に開くファイルで置き換わるタブ）
- 画像の差分の、重ねて透過する表示とスワイプの表示
- React の画面と Tauri シェル
- React の画面が使う既存の表示の上限（1 MiB・5,000 行・300 hunk・10 万文字）

# Requirements

- R-001: 右の「ファイル」の面に、選択中の worktree のファイルとディレクトリが木で出る。`.git` ディレクトリは出ない。
- R-002: 木のディレクトリの中身は、利用者がそのディレクトリを開いたときに読まれる。開いていないディレクトリの中身は読まれない。
- R-003: 木の各項目に git の状態が色で出る。変更を含むディレクトリは、閉じていても中の変更の色で出る。`.gitignore` で除外された項目は薄く出る。
- R-004: 開いているディレクトリの中身と、木の項目の git の状態が変わると、利用者の操作なしに木に反映される。
- R-005: 木でファイルを開くと、中央のファイルタブで開く。focus 中の pane に同じファイルのタブがあればそのタブが選ばれ、無ければ新しいタブで開く。
- R-006: 開くときの比較で変更の無いファイルは、File 表示（ファイルの内容をそのまま出す表示）で開く。変更のあるファイルは Diff 表示で開き、帯で File 表示に切り替えられる。File 表示は作業ツリーの内容を出す。
- R-007: Diff 表示の比較は、帯で「HEAD 比較」と「base 比較」を切り替えられる。HEAD 比較は HEAD → 作業ツリーのすべての変更（`git diff HEAD` と同じ）を、base 比較は base branch → 作業ツリーの変更を出す。
- R-008: Diff 表示は、帯で上下 / 左右と、変更箇所のみ / 全文（変更の無い範囲を畳むか開くか）を切り替えられる。
- R-009: 帯に、ファイルの path、比較の選択、追加行数と削除行数（+N −M）、File / Diff、上下 / 左右、変更箇所のみ / 全文、未解決の Thread の件数、「外部エディタで開く」が出る。
- R-010: 「外部エディタで開く」を押すと、そのファイルが外部エディタで開く。
- R-011: File 表示と Diff 表示の文字は、ファイルの言語に応じて色付けされる。markdown のファイルも、ほかのファイルと同じくコードとして色付けされて出る。
- R-012: ファイルタブの中の明暗は、システムの明暗の設定に合う。
- R-013: HEAD 比較の Diff 表示では、hunk ごとに stage 済みかどうかが出る。利用者は hunk ごとの操作で、その hunk を stage / unstage できる。
- R-014: base 比較の Diff 表示では、hunk の stage 状態と stage / unstage の操作が出ない。
- R-015: Diff 表示の行の間に、そのファイルの行に付いた Thread が出る。未解決の Thread は開いて出て、解決済みの Thread は畳んで出る。
- R-016: 利用者は、行の間の Thread に返信でき、Thread を解決できる。
- R-017: 利用者は、マウスで行を選んで、その行に Thread を作れる。
- R-018: 開いているファイルの内容、差分、stage 状態、Thread が変わると、利用者の操作なしにファイルタブに反映される。
- R-019: 画像のファイルは、画像として出る。変更のある画像は、前と後が左右に並んで出る。追加と削除の画像は、ある側だけが出る。
- R-020: テキストのファイルは、大きさ、行数、hunk 数にかかわらず、全文と差分の中身が出る。
- R-022: 画像以外のバイナリのファイルは、中身が出ずに、表示しない理由、大きさ、「外部エディタで開く」が出る。
- R-023: ファイルタブが開いているファイルと、そのタブの File / Diff と比較の選択が保存される。worktree を選び直したときとアプリを起動し直したときに、同じファイルが同じ表示で開く。
- R-024: 上下 / 左右と変更箇所のみ / 全文の選択は、アプリの設定値として保持され、すべてのファイルタブに効く。帯で切り替えると設定値が変わる。
- R-025: 新しく開くファイルタブの比較の初期値は、アプリの設定値の base の既定から読む。設定値が無いときは HEAD 比較になる。
- R-026: ファイルタブは、外部のネットワークに接続せずに中身を出せる。
- R-027: 回帰の E2E が、変更のあるファイルをファイルタブで開くと差分が出ることと、「ファイル」の面からファイルをファイルタブで開けることを確かめる。

# Assumptions

なし
