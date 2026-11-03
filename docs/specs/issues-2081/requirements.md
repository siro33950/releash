# Context

- 要求の正本は ISSUE #2081「Swift.app: Node 面で idle の行を閉じ、active / attention の行を開いて表示する」である。
- 補助資料は main `99a6a2af`、`clients/macos/Sources/RootListView.swift`、`server/src/domain/workspace_tree/value_objects.rs` である。
- Node 面の仕様は `docs/specs/issues-1762/`、status の 3 値と集約の仕様は `docs/specs/issues-1958/` が持つ。
- 各行の status はサーバが集約して配信する。親の status は自分と子孫の最も重い値であり、Completed と Aborted は idle になる（`server/src/domain/workspace_tree/value_objects.rs:397-451`）。子が動いていれば親は active か attention である。
- 行の開閉を status から導く判断と、手動操作の記憶は、Swift.app の画面に置く。サーバは開閉を配信せず、保存もしない。
- 子を持つ行は、Sequence の行、Fanout の行、delegate の子を持つ Session の行である。

# Outcome

対象者は、Swift.app の Node 面で Workflow の実行木を観測する開発者である。

今の Node 面は、子を持つ行を全部開いた状態で表示を始める。
完了済みの phase や Fanout が開いたまま残り、進行中の行が画面の下に押し出される。
実行木が深い Workflow では、今どの Node が動いているかを見るたびに、完了済みの行を手で閉じる必要がある。

変更後は、idle の行は閉じ、active か attention の行は開いた状態で表示され、status の変化に追従する。
開発者が手で開閉した行は、その操作が status より優先される。

# Current Behavior

調査の時点は main `99a6a2af` である。

- 開閉状態は画面のローカル状態 `collapsed: Set<String>` で、初期値は空集合である（`clients/macos/Sources/RootListView.swift:53`）。そのため、子を持つ行は全部開いた状態で表示が始まる。
- 行の開閉は、この集合に行の ID があるかで決まる（`RootListView.swift:333-339`）。
- Sequence と Fanout の行を押すと開閉が切り替わる（`RootListView.swift:245-250`）。集合への書き込みは、この `toggle` と DisclosureTableRow の Binding（`RootListView.swift:337-339`）の 2 か所にある。
- status が変わっても開閉は変わらない。手で閉じた行は、閉じたままである。
- 過去の試行の開閉は、別のローカル状態 `expandedPastAttempts` が持つ（`RootListView.swift:54`、`:231-241`、`:346-350`）。
- 左サイドバーの repository group の開閉は、サーバが所有して配信する（`clients/macos/Sources/AppModel.swift:620-629`、`clients/macos/Sources/SidebarView.swift:38-42`）。Node 面の行の開閉は、サーバに無い。
- サーバは各 Node に `pastAttemptsCollapsed` を配信するが（`server/src/adaptor/presenter/client/workspace.rs:287`）、Swift.app は読んでいない。

再現手順:

1. Sequence の子に Session を持つ Workflow を開始する。
2. Node 面で、Session が完了して実行木の Root が idle（緑）になるまで待つ。
3. Root の行は開いたままで、子の行が並び続ける。

# Scope / Non-goals

変更するもの:

- Node 面の、子を持つ行の初期の開閉。
- status の変化に合わせた開閉の追従。
- 手動の開閉と status の優先関係。

変更しないもの:

- 過去の試行の開閉。
- 開閉のサーバへの保存と、アプリの再起動や worktree の切り替えをまたいだ開閉の保持。
- サーバが配信する status の値と集約の規則。
- 行の選択、`▷`、Approve、右クリックメニューなど、開閉以外の Node 面の操作。

# Requirements

- R-001: Node 面の子を持つ行は、status が idle なら閉じた状態、active か attention なら開いた状態で表示される。
- R-002: 子を持つ行の status が idle から active か attention に変わると、その行は開く。active か attention から idle に変わると、その行は閉じる。
- R-003: 開発者が手で開閉した行は、その後の active と attention の間の変化では開閉が変わらない。idle かどうかが変わったときに、R-002 に従って開閉が上書きされる。

# Assumptions

- 行の開閉を status から導く判断と、手動操作の記憶は、Swift.app の画面に置く。
- 開閉の上書きの契機は、status から導いた開閉（idle かどうか）が変わったときだけである。active と attention の間の変化は契機にしない。

# Invariants

なし

# Risks

なし
