# Context

- 要求の正本: [siro33950/releash#1833](https://github.com/siro33950/releash/issues/1833)
- 補助資料: マイルストーン [#78](https://github.com/siro33950/releash/milestone/78)、[#1944](https://github.com/siro33950/releash/issues/1944)、[#1763](https://github.com/siro33950/releash/issues/1763)、[#1204](https://github.com/siro33950/releash/issues/1204)、[#1759](https://github.com/siro33950/releash/issues/1759)、[#1764](https://github.com/siro33950/releash/issues/1764)
- 対象の commit: main `2b9dc7d2`

確定済みの制約は次のとおりである。

- Swift.app の画面は SwiftUI の標準の部品と標準の style を使い、色はシステムの色、文字はシステムの文字の段階を使う（マイルストーン #78「UI の決まり」）。
- Swift.app は自前でロジックを持たず、操作は proto の RPC で行う（`AGENTS.md`「サーバがロジックを所有する」）。
- View の Theme と Font Size は引き継がない（#1763）。New Terminal と Back to Kanban は Tauri 版でも handler が空であり、引き継がない。Quit と About の振る舞いは #1764 が扱う。
- 画面の機能を足す PR は、機能の主たる動作の回帰の E2E を XCUITest で足す（マイルストーン #78「テストと CI の方針」）。

# Outcome

対象者は Swift.app を使う開発者である。

Swift.app のメニューバーには File の Open Folder… と Git の項目（Stage All、Unstage All、Create Branch…）が無く、Finder からファイルを terminal の上に落としても何も受け取らない。

変更後、開発者は Tauri 版と同じ項目をメニューバーから実行でき、Finder からファイルを落とした terminal にその path が書き込まれる。

# Current Behavior

Swift.app のメニューの定義は `clients/macos/Sources/ReleashApp.swift:9-26` だけである。
File の New 項目を「New Releash Window」に差し替え、「タブ」メニューを足している。
Releash、Edit、View、Window、Help の各メニューは SwiftUI の既定に任せている。既定で出る項目の実際の構成は、起動して確かめていない。
Settings… は Settings scene が app メニューに出す（#1763）。

Repository の追加は `AppModel.addRepository()`（`clients/macos/Sources/AppModel.swift:637-653`）が NSOpenPanel でフォルダを選ばせ、RPC `AddRepoPath` を呼ぶ。呼び出し元はサイドバーの追加ボタンだけである（`clients/macos/Sources/SidebarView.swift:20`）。

RPC `GitStage`、`GitUnstage`、`GitCreateBranch` はサーバにある（`proto/client.proto:2704-2707`）が、Swift.app に呼び出し元は無い。
`GitStage` と `GitUnstage` は `paths` が空のとき、サーバが対象を決める（`server/src/adaptor/gateway/code/staging.rs:11-59`）。
`GitCreateBranch` は HEAD の commit から branch を作り、checkout する（`server/src/adaptor/gateway/repository/branch.rs:64-74`）。

Tauri 版のメニューは `clients/desktop/src-native/infrastructure/platform/menu.rs` にある。
Open Folder… は repository の追加を呼ぶ（`clients/desktop/src/App.tsx:168`）。Stage All と Unstage All は `paths` を空にして RPC を呼び、Create Branch… は branch 名を入力するダイアログを開いて RPC を呼ぶ（`clients/desktop/src/screens/useWorktreeGitActions.ts:77-111`）。

Tauri 版のドロップは、Tauri シェルが WKWebView の `performDragOperation:` を差し替えて path を送り（`clients/desktop/src-native/infrastructure/platform/native_drop.rs:38-42`、`:69-71`）、React の `TerminalPanel` が RPC `WritePathsToTerminalSurface` を呼ぶ（`clients/desktop/src/components/panels/TerminalPanel.tsx:91-101`、`:118-135`）。
`TerminalPanel` は worktree の Terminal と Session の TUI の両方で使われている（`clients/desktop/src/components/panels/RightSidebarBottom.tsx:65`、`clients/desktop/src/components/panels/AgentSessionPanel/AgentSessionPanel.tsx:144-154`）。
サーバは path を shell 用に引用符で囲んで terminal に書く（`server/src/usecase/terminal_surface/io_usecase.rs:16-28`、`server/src/domain/shell.rs:9-15`）。

Swift.app の terminal view は `TerminalScreen`（`clients/macos/Sources/TerminalSession.swift:231-266`）で、Terminal タブ（`clients/macos/Sources/PaneWorkspace.swift:322-327`）と Session タブ（`clients/macos/Sources/SessionTabPane.swift:18`）の両方が使う。
`TerminalScreen` にも SwiftTerm 1.99.0 の `TerminalView` にも、`NSDraggingDestination` の実装は無い。
Swift.app に `WritePathsToTerminalSurface` の呼び出し元は無い。

再現手順は次のとおりである。結果はコードから導いたもので、起動して観測していない。

1. Swift.app を起動し、サイドバーで worktree を選ぶ。メニューバーに Git メニューと File の Open Folder… が無い。
2. Terminal タブを開き、Finder からファイルをその上にドラッグして離す。受け口が無いため、terminal には何も書かれない。

# Scope / Non-goals

変更する対象:

- Swift.app のアプリメニュー。Tauri 版の項目のうち、New Terminal、Back to Kanban、View の Theme と Font Size を除いたもの。
- Finder からドロップしたファイルの path を、ドロップ先の terminal へ書き込むこと。
- メニュー項目から操作を実行できることを確かめる回帰の E2E（XCUITest）。

変更しない対象:

- New Terminal と Back to Kanban。
- View の Theme（Dark / Light）と Font Size の増、減、リセット。
- Quit と About の振る舞い（#1764）。
- provider の hook の警告（#1944）。
- サーバと proto。既存の RPC で足りる。
- React と Tauri シェル。

# Requirements

- R-001: Swift.app のメニューバーに、Tauri 版のアプリメニューの項目のうち New Terminal、Back to Kanban、View の Theme と Font Size を除いたものがあり、開発者はそれぞれを Tauri 版と同じ操作として実行できる。Open Folder… は repository の追加、Stage All と Unstage All は選択中の worktree の変更すべての index への追加と除去、Create Branch… は入力した名前の branch の作成と切り替えである。
- R-002: 開発者が Finder からファイルを terminal の上に落とすと、その terminal にファイルの path が書き込まれる。

# Assumptions

なし。

# Invariants

なし。

# Risks

なし。
