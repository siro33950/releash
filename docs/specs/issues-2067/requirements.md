# Context

- 要求の正本は [Issue #2067「Swift.app: Node 面の実行木を既定で全て展開した状態にする」](https://github.com/siro33950/releash/issues/2067) である。
- 補助資料は [Issue #1762](https://github.com/siro33950/releash/issues/1762)、[参照 commit e0eb05b0](https://github.com/siro33950/releash/commit/e0eb05b06c2c0019e20a6da6cffbde05f1fcd04f)、[同 commit の RootListView.swift](https://github.com/siro33950/releash/blob/e0eb05b0/clients/macos/Sources/RootListView.swift)、`docs/specs/issues-1762/requirements.md`、`docs/specs/issues-1762/behavior.md`、`docs/specs/issues-1762/design.md` である。
- Node 面は、選択した worktree の実行木を表示する。
  ツリーの開閉矢印で扱う子と、「過去の試行」ボタンで扱う Retry 前の履歴は、別々に開閉する（`docs/specs/issues-1762/requirements.md:107`）。
- `AGENTS.md` に従い、実行木の内容と実行状態はサーバが所有し、クライアントは表示とレイアウトを制御する。

# Outcome

対象者は、Swift.app の Node 面で Workflow の実行を観測する開発者である。
現在は、子の Node を見るために、親の行を階層ごとに開く必要がある。
変更後は、ツリーが全階層で既定展開され、行を一つずつ開かずに子の Node を確認できる。

# Current Behavior

開発開始時点の HEAD は `6301d433c2eb4b55ae298069f88ff80f940ca15f` である。
以下はコードと既存テストの記述で確認した変更前の挙動であり、実機での再現およびテスト実行は行っていない。
`RootListView.swift` は、入力資料が参照する commit `e0eb05b0` から変更されていない。

再現手順は、子を持つ Sequence または Fanout を含む Workflow を開始し、対象 worktree の Node 面を表示することである。
子の行は表示されず、親の開閉矢印を押すか、Sequence または Fanout の行を選ぶと表示される。
入れ子の子を見るには、各階層を開く必要がある。

- 展開状態の初期値は空集合であり、行の ID が集合に含まれる場合だけ子を表示する（`clients/macos/Sources/RootListView.swift:53`、`:327–335`）。
- この開閉は Sequence、Fanout と、delegate の子を持つ Session に共通する（`clients/macos/Sources/RootListView.swift:274–279`）。
- Sequence と Fanout の行を選ぶと開閉を反転する。
  Session と Command の行を選ぶと、選択した Node の詳細を表示する（`clients/macos/Sources/RootListView.swift:94–112`、`:239–243`）。
- 「過去の試行」は初めは表示されず、専用ボタンで表示と非表示を切り替える。
  ツリーの子の開閉とは別の状態で扱う（`clients/macos/Sources/RootListView.swift:229–234`、`:269`、`:338–339`）。
- worktree を切り替えると Node 面の表示状態を作り直す（`clients/macos/Sources/RightPanelView.swift:55–56`）。
- 実行木の内容は購読で更新されるが、その更新時に展開状態を初期化する処理はない（`clients/macos/Sources/AppModel.swift:109–119`、`clients/macos/Sources/RootListView.swift:49–61`）。
- 既存の統合テストは、delegate の子が初めは非表示であることと、過去の試行との独立した開閉を確認する構成である（`clients/macos/Tests/Integration/RootListViewTests.swift:282–298`）。
  入れ子の実行木を扱う統合テストと回帰 E2E も、親の行を開いてから子を確認する構成である（`clients/macos/Tests/Integration/RootListViewTests.swift:373–384`、`clients/macos/UITests/WorkbenchTests.swift:457–462`）。

# Scope / Non-goals

変更するもの:

- Swift.app の Node 面で、ツリーの開閉矢印が扱う全ての行の既定の展開状態。
  Sequence、Fanout、delegate の子を持つ Session を含み、階層の深さで対象を限定しない。

変更しないもの:

- 「過去の試行」ボタンによる履歴の表示と非表示、および履歴を初めは表示しない挙動。
- 既定の展開状態以外の行操作（手動の開閉、Node の選択、タブを開く操作、Approve など）。
- 実行木の内容と実行制御、サーバとの通信契約、展開状態の永続化、React の画面。

# Requirements

- R-001: Swift.app の Node 面では、ツリーの開閉矢印で開閉する全ての行を、既定で展開した状態で表示する。
  Sequence、Fanout、delegate の子を持つ Session を含め、利用者が行を開く操作をしなくても、全階層の子が表示される。
- R-002: 「過去の試行」の履歴は初めは非表示とし、専用ボタンで表示と非表示を切り替える既存の挙動を維持する。
  この表示切り替えは、ツリーの子の開閉と独立している。

# Assumptions

なし
