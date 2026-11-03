# Context

- 要求の正本は ISSUE #2066「Swift.app: Archive した Session のタブが自動で閉じない」である。
- 補助資料は main `e0eb05b0`、`clients/macos/Sources/RootListView.swift`、`clients/macos/Sources/AppModel.swift`、`clients/macos/Sources/PaneWorkspace.swift`、`clients/macos/Sources/NodePane.swift`、`server/src/usecase/workflow/workspace_tree.rs`、`server/src/usecase/workspace_state/usecase.rs` である。
- 中央のカラムのタブの並び（pane layout）は、worktree ごとの workspace state としてサーバが所有し、保存し、配信する。画面は購読して復元するだけである（`clients/macos/Sources/AppModel.swift:442-446`、`:498-506`）。
- Session の存在は、サーバの実行木が決める。Archive 済みの実行は画面に出す木から除かれる（`server/src/domain/workspace_tree/visible.rs:47-72`）。Session は open、paused、archived の lifecycle を持ち（`server/src/domain/agent_session/aggregates/agent_session.rs:12-16`）、archived だけが「居ない」状態である。
- Session の Archive は、Node 面の右クリックメニューと Session タブの Archive の両方が、同じ `ArchiveWorkspaceWorkflowExecution` を呼ぶ（`clients/macos/Sources/RootListView.swift:287-297`、`clients/macos/Sources/SessionTabPane.swift:117-127`）。
- 規則は 2 つである。Session が増えたら Swift.app はタブを増やし、Session が消えたら Swift.app はタブを消す。Swift.app がタブを増減しても、選択中のタブは変わらない。

# Outcome

対象者は、Swift.app で Workflow と Session を扱う開発者である。

今は、Session を Archive しても、その Session を開いているタブが中央のカラムに残る。
開発者は Archive 済みの Session のタブを一つずつ手で閉じる必要があり、開発者が閉じ忘れたタブを Swift.app は再起動後も出す。
また、Workflow の Node や CLI が起動した Session は、開発者が Node 面の `▷` を押すまでタブに現れない。

変更後は、Session が消えると Swift.app がそのタブを消し、Session が増えると Swift.app がそのタブを増やす。
Swift.app がタブを増減しても、開発者が見ているタブは変わらない。

# Current Behavior

調査の時点は main `2b9dc7d2` である。実機では確かめておらず、コードで確認した挙動を記す。

- Node 面の Archive は `ArchiveWorkspaceWorkflowExecution` を呼ぶだけで、タブには触れない（`clients/macos/Sources/RootListView.swift:231-239`、`:287-297`）。Session タブの Archive も同じ RPC を呼ぶ（`clients/macos/Sources/SessionTabPane.swift:117-127`）。
- サーバの Archive は、実行を止めて archive を記録し、worktree の購読へ通知するだけである（`server/src/usecase/workflow/execution_archive.rs:99-144`）。pane layout を変える処理は無い。pane layout を扱う usecase は、保存時に省略された pane を保つ処理と、読み込み時の legacy な Session タブの解決だけである（`server/src/usecase/workspace_state/usecase.rs:6-34`、`:36-98`）。
- 画面は workspace state の購読で pane layout を受け取り、Command タブの題名を実行木に合わせて付け直す（`clients/macos/Sources/AppModel.swift:459-511`、`:665-670`、`clients/macos/Sources/PaneWorkspace.swift:217-222`）。実行木から消えた Session のタブを閉じる処理は無い。
- タブを閉じる経路は、利用者の操作だけである（`clients/macos/Sources/PaneWorkspace.swift:239-255`）。
- タブが自動で開くのは、この画面から Session を起動したときと Workflow を起動したときだけである（`clients/macos/Sources/AppModel.swift:672-694`、`:601-608`）。Workflow の Node や CLI が起動した Session、History から Restore した Session のタブは開かない。
- Archive 後に残った Session タブは、`session-tab` の購読が archived を返すと「SessionはArchiveされています」を表示する（`clients/macos/Sources/SessionTabPane.swift:29`）。Session の ID が付く前の Node のタブは `node-detail` の購読で表示を決める（`clients/macos/Sources/NodePane.swift:13-29`）。

再現手順:

1. Swift.app で worktree を開き、Node 面の `▷` で Session のタブを開く。
2. Node 面の右クリックメニューか Session タブの Archive で、その Session を Archive する。
3. Swift.app は Node 面から Session の行を消す。Session のタブは残り、Swift.app はそのタブに「SessionはArchiveされています」を表示する。
4. 開発者が Swift.app を再起動すると、Swift.app はそのタブを出す。

# Scope / Non-goals

変更するもの:

- Session が消えたときに、Swift.app がその Session のタブを消すこと。
- Session が増えたときに、Swift.app がその Session のタブを増やすこと。
- Swift.app がタブを増減したときに、選択中のタブを変えないこと。
- 開発者がこの画面から Session と Workflow を起動したときに、画面側がタブを開く処理の置き換え。

変更しないもの:

- 開発者がタブを手で開く操作と閉じる操作。
- Terminal、File、Command のタブ。
- Archive、Restore、Resume の操作そのもの。
- Session の lifecycle と、サーバが画面に出す木を決める規則。

# Requirements

- R-001: Session が消えると（開発者が Archive する）、Swift.app はその Session のタブを消す。
- R-002: Session が増えると、Swift.app はその Session のタブをその worktree の中央のカラムに増やす。Session は、誰かが起動したとき、開発者が Archive 済みの Session を Restore したときに増える。Session を起動したのがこの画面、Workflow の Node、CLI、別のウィンドウのどれであっても同じである。
- R-003: Swift.app が R-001、R-002 でタブを増減しても、Swift.app は選択中のタブと、各 pane が表示しているタブを変えない。

# Assumptions

なし

# Invariants

なし

# Risks

なし
