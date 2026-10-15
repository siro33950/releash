# Context

- 要求の正本は ISSUE #1762「[07] 右カラムの Node 面（Root の一覧と、今あるデータでの実行木の観測）」である。
- 補助資料は #1205（[04] Session タブと Command タブ）、#2011（[11] 定義と Facet の画面）、マイルストーン #89 の #1708・#1709・#1710（経過 / 実働、Token / コスト、イベント）、ISSUE に添付された mock 2 枚である。
- マイルストーン #78 の決まりに従う。
  ロジックはサーバに置き、Swift.app は表示と操作の受付だけを行う。
  RPC と購読の形は Swift.app の都合に合わせず、どのクライアントからも同じ形で使えるものにする。
  色はシステムの色と状態の 3 色だけを使い、部品は SwiftUI の標準の部品を使う。
- mock は配置と、出す情報と、できる操作の正である。
  ただし、この開発では mock の「Workflow 管理」タブを中央のタブではなく右カラムの面に置き、経過 / 実働・TOKENS / COST の列、タイムライン、イベント、実行の見出しの行、下段の「出力」ボタン、Artifact の表の TYPE 列は作らない。
- 状態の分類は active / attention / idle の 3 値である（`server/src/domain/workspace_tree/value_objects/mod.rs:60-67`）。
- 利用者のいない段階なので、proto の enum 値の削除などの非互換な変更を行ってよい。

# Outcome

対象者は、Swift.app で Workflow の実行を観測し、判断点で操作する開発者である。

今の Swift.app の右カラムの「Root の一覧」は、worktree の実行木の Root を 1 行ずつ並べるだけで、子の Node を出さない。
Sequence と Fanout の Root を押すと「Workflow 管理」タブが開くが、中身はタブ名だけである。
そのため、React を撤去すると、Workflow の実行木・各 Node の状態・失敗理由・Artifact を見る画面と、Approve を行う画面が Swift.app から無くなる。

変更後は、右カラムの「Node」の面で、Session の Root と Workflow の実行木を同じ面に並べて観測できる。
承認待ちの Node をその行から Approve でき、選んだ Node の失敗理由と Artifact の中身を同じ面で読み、Retry / Abort を行える。

# Current Behavior

調査の時点は HEAD `23442620` である。

Swift.app（`clients/macos/Sources/`）の挙動は次のとおりである。

- 右カラムの面は「Root の一覧」「ファイル」「Source Control」「Thread」の順に並ぶ（`RightPanelView.swift:7-25`）。
- 「Root の一覧」は、選んでいる worktree の実行木の Root を 1 行ずつ並べ、子は出さない（`RootListView.swift:61-88`）。
  行は状態の色のアイコン、provider の名前、タイトルを出す。
- Root の行を押すと、Session なら Session タブ、Command なら Command タブ、Sequence と Fanout なら「Workflow 管理」タブを開く（`RootListView.swift:18-20`、`AppModel.swift:617-618`）。
  「Workflow 管理」タブの中身はタブ名だけである（`PaneWorkspace.swift:201-202`）。
- 面の上端に、New Session と New Workflow の「+」メニュー、History の「…」メニューがある（`RootListView.swift:42-59`）。
  History は、archive 済みの Session と provider の履歴だけを出し、archive 済みの Workflow の実行は出さない（`SessionHistoryView.swift:19-33`）。
- Root の行の右クリックメニューに Rename、Resume、Archive がある（`RootListView.swift:77-87`）。
- Swift.app には Approve を呼ぶ箇所がない。
  Session タブは terminal と Resume だけを出す（`NodePane.swift:31-72`）。
  Retry と Abort は Command タブにだけある（`NodePane.swift:97-98`）。
- Command タブは、過去の試行を、画面が持つ実行木の Root からだけ探す（`NodePane.swift:79-86`）。
- 右カラムは、開いているとき最小幅 200、最大幅 400 である（`RightPanelView.swift:61-65`、`Style.swift:13`、`Style.swift:16`）。
  左のサイドバーも最大幅 400 の同じ定数を使う（`ReleashApp.swift:57-60`）。
- 中央のカラムには最小幅が無く、`layoutPriority(1)` だけがある（`ReleashApp.swift:69-87`）。
- タブの種類 workflow は、Swift の `PaneTab.Kind`（`PaneLayout.swift:11-31`、`:51`、`:93`）にある。
- 回帰の E2E は、面の見出し「Root の一覧」を文字列で探し、Root の行を押してタブを開く（`UITests/WorkbenchTests.swift:257-260`、`:292-295`）。

サーバ（`server/src/`）と proto（`proto/client.proto`）は次のとおりである。

- 実行木は `WorkspaceTreeSnapshot`（`proto/client.proto:1694-1698`）で届く。
  項目の種類は node / sequence / fanout で、周回を表す種類は無い。
  同じ名前の Node へ戻った実行は、別の Node として並ぶ。
- domain の実行木の Node は、実行の ID、Node の実行の ID、Node 名、attempt、Artifact の有無を持つ（`domain/workspace_tree/value_objects/mod.rs:110-119`）。
  wire の `WorkspaceNode`（`proto/client.proto:1709-1724`）は、これらを持たない。
- 失敗理由は `WorkspaceNode.errorReason` にあり、Sequence と Fanout（`proto/client.proto:1756-1774`）には無い。
- provider と model は定義の Session の指定（`SessionSpec`。`domain/workflow/value_objects/definition.rs:551-561`）にある。
  model は任意である。
  実行木の項目には model が無い。
- Artifact の本文を返す購読は 2 つある。
  `workflow-output`（引数は実行の ID と Node 名）は Node 名ごとに最新の 1 件を返すため、同じ名前の Node の周回を区別できない（`usecase/workflow/output.rs:16-54`）。
  `workflow-execution`（引数は実行の ID）は実行全体を返す。
  どちらも使っているのは CLI だけである。
- `node-detail`（`WorkspaceNodeDetailDto`。`proto/client.proto:1622-1640`）には過去の試行が無い。
- Approve / Retry / Resume / Abort / Archive / 実行の復元の RPC は既存である（`proto/client.proto:2632`、`:2636`、`:2638`、`:2685`、`:2687`、`:2688`）。
  archive 済みの Workflow の実行の一覧は、worktree ごとの `workflowHistory`（`proto/client.proto:2781`）で届く。
- タブの種類 workflow は、proto の `PaneTabKind`（`PANE_TAB_KIND_WORKFLOW`。`proto/client.proto:1968`）、domain（`domain/workspace_state/value_objects/pane_layout.rs:33-39`）、保存形式（`adaptor/gateway/local_event_store/workspace_state_codec.rs:290-296`）にある。
  保存形式から消すと、`"kind":"workflow"` のタブを含む保存済みのイベントは読み込みで malformed になる（`workspace_state_codec.rs:45`）。

React（`clients/desktop/src/`）の実行木の表示は、各行に状態の色・タイトルを出す。
Node の行は過去の試行を開閉して並べ、Sequence と Fanout の行は隔離 worktree の branch と path を出す（`components/workspace/WorkspaceList.tsx:200-560`）。
失敗理由はどこにも出さない。

# Scope / Non-goals

変更するもの:

- 右カラムの面「Root の一覧」を「Node」の面に置き換える。
- Node 面の上段の実行木と、下段の選択した Node の詳細。
- 右カラムの最大幅の制約と、中央と右のカラムの最小幅と初期の幅。
- タブの種類 workflow の削除（Swift、proto、domain、保存形式）。
- History に、archive 済みの Workflow の実行の復元を足す。
- 実行木の項目に、Node の実行の識別、MODEL、Artifact の有無を出す。
- Node の実行ごとに Artifact の本文を返す購読を足す。
- `node-detail` に過去の試行を出し、Command タブはそれを使う。
- 回帰の E2E が探す面の名前と、タブを開く操作の変更。
- Node 面の背景色を、左のサイドバーや右カラムの他の面と揃える。

変更しないもの:

- 経過 / 実働・TOKENS / COST の列、タイムライン、イベント（マイルストーン #89 で足す）。
- 実行の見出しの行（Root の名前、Node の数、深さ、色ごとの件数）。
- 下段の「出力」ボタン。
- Artifact の表の TYPE 列。
- Artifact の提出の回数の表示。
- 定義と Facet の画面（#2011）。
- 左のサイドバーの最大幅。
- React の画面。

# Requirements

- R-001: 右カラムに「Node」の面がある。「Root の一覧」の面は無い。
- R-002: Node 面の上段は、選んでいる worktree の実行木を並べる。Session と Command の Root は 1 行で出す。Sequence と Fanout の行は開閉でき、開くと子の Node・Sequence・Fanout を実行木の形で字下げして並べる。
- R-003: 実行木の各行は、状態の 3 色、タイトルを出す。Sequence の行は種別 `SEQ`、Fanout の行は種別 `FANOUT` を出す。Session と Command の行は、失敗理由の一文、MODEL、Artifact の有無を出す。
- R-004: Session の行の MODEL は、定義の Session の指定にある model である。model の指定が無い Session と、Command の行の MODEL は空欄である。
- R-005: Artifact を提出した Session と Command の行は、ART 列に `◆` を出す。提出の回数は出さない。
- R-006: Session と Command の行は、過去の試行を開閉して並べられる。delegate の子を持つ Session の行は、開閉の矢印で delegate の子を開閉でき、過去の試行はそれとは別の操作で開閉する。Sequence と Fanout の行は、隔離 worktree を持つとき、その branch と path を出す。
- R-007: 並列 12・深さ 4 の実行木と、同じ名前の Node へ戻った実行（周回の再入）が、それぞれ別の行として潰れずに並ぶ。
- R-008: Session と Command の行を押すと、その行が選択され、下段にその Node の詳細が出る。Sequence と Fanout の行を押すと、開閉だけが起き、選択は変わらない。
- R-009: Session と Command の行は `▷` ボタンを持ち、押すと Session の行は Session タブ、Command の行は Command タブを中央に開く。
- R-010: 下段は、選択した Node の名前、種別、MODEL、失敗理由を出す。
- R-011: 下段は、選択した Node の実行が提出した Artifact の中身を、FIELD / VALUE の 2 列の表で出す。object と配列は字下げして展開する。同じ名前の Node の周回は、それぞれの Node の実行の Artifact を出す。
- R-012: 下段は、Artifact の中身を JSON で表示する切り替えを持つ。
- R-013: 下段は、選択した Node について、サーバが可とするときに Retry / Abort を出し、押すとその操作が実行される。
- R-014: Node 面の上端に、New Session と New Workflow の「+」メニューと、History の「…」メニューがある。Root の行の右クリックメニューに、サーバが可とするときに Rename、Resume、Archive がある。
- R-015: History から、archive 済みの Workflow の実行を選んで復元できる。復元した実行は Node 面の実行木に戻る。
- R-016: 右カラムを 400 より広げられる。左のサイドバーの最大幅は変わらない。
- R-017: 中央のカラムと、開いている右カラムの最小幅は、どちらも、ウィンドウの内側（外周の余白を除いた部分）の最小幅から左のサイドバーの最小幅を引いた残りを 2 等分した値である。中央のカラムと右カラムの初期の幅は、どちらも、ウィンドウの内側の初期の幅から左のサイドバーの初期の幅を引いた残りを 2 等分した値である。左のサイドバーの最小幅と初期の幅、ウィンドウの外周の余白、ウィンドウの最小幅は変わらない。
- R-018: 中央のタブに「Workflow 管理」タブを開く経路が無い。クライアントとサーバの通信で、タブの種類 workflow は使えない。
- R-019: 保存済みのペインのレイアウトに「Workflow 管理」タブが含まれていても、そのタブを除いた残りのレイアウトが読み込まれる。
- R-020: Command タブは、Root か子かによらず、その Command の過去の試行を出す。
- R-021: どのクライアントも、Node の実行を指定して、その Node の実行の Artifact だけを購読で受け取れる。
- R-022: 実行木の行は、サーバがその Node の Approve を可とするときに Approve ボタンを出し、押すとその Node が承認される。
- R-023: Node 面の背景（上段の実行木と下段の詳細）は、左のサイドバーや右カラムの他の面と同じ色である。白や黒の背景を使わない。

# Assumptions

なし
