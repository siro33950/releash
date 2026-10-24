# Context

- 要求の正本: [siro33950/releash#2072](https://github.com/siro33950/releash/issues/2072)。
- 補助資料: `clients/macos/Sources/SidebarView.swift`、`clients/macos/Sources/RootListView.swift`、`clients/macos/Sources/FileTreeView.swift`。
- Issue の統合ログ参照コマンド:

  ```sh
  /usr/bin/log show --start "2026-10-22 13:19:30" --end "2026-10-22 13:19:35" --predicate 'processID == 3641'
  ```

対象は macOS の Swift アプリである。
ユーザーが報告した発生場面は、GitHub Issue または Notion task から Worktree を作成し、作成後に Workflow を起動した直後である。
特定の Workflow 定義には限定しない。

`AGENTS.md:38` の責務分担に従い、サーバが配信する状態を画面が表示する。
作成画面は Workflow 指定を含む作成要求を送信し、`AppModel` は配信された Workspaces の状態を画面へ渡す（`clients/macos/Sources/CreateWorktreeView.swift:213`、`clients/macos/Sources/AppModel.swift:109`）。

# Outcome

Swift アプリで Worktree を作成して Workflow を開始する開発者が、一覧更新の不具合で作業を中断し、アプリを再起動しなければならない問題を解決する。
作成した Worktree を一覧で確認でき、その後の状態変化も画面で継続して確認できるようにする。

# Current Behavior

Sidebar は Repository の行に `repository.path`、その配下の Worktree の行に `branch.worktreePath` を ID として使う（`clients/macos/Sources/SidebarView.swift:33`、`:43`）。
main Worktree のパスは Repository のルートを指し、サーバはそのパスを両方の項目に渡すため、親と子の ID が重複する（`server/src/adaptor/gateway/repository/worktree.rs:295`、`server/src/adaptor/presenter/client/workspace.rs:75`、`:91`）。

作成操作の再現では、実際の作成画面と模擬サーバを使い、次を確認した。

1. main Worktree を含む Repository を表示し、GitHub Issue または Notion task を複数選択する。
2. 作成後に起動する対象を Workflow にし、定義を選択して作成する。
3. Workflow 指定を含む作成要求を受信した模擬サーバから、作成した Worktree と Workflow の状態を順次配信する。

GitHub Issue と Notion task のどちらでも、新しい Worktree の行が現れず、続く追加で子行挿入の例外が発生した。
Workflow の実行と外部サービスへの接続は模擬している。
再現コードと出力は次にある。

- `/private/tmp/releash-2072-investigation/WorkflowCreationOutlineTests.swift`
- `/private/tmp/releash-2072-investigation/workflow-issue-actual-4.log:205`
- `/private/tmp/releash-2072-investigation/workflow-notion-actual.log:44`

一覧以外への影響は、XCTest を使わず、実際の `RootView` を表示する独立した診断アプリで確認した。
再現手順は次のとおりである。

1. 3 個の Repository にそれぞれ main、b、m、z の Worktree を表示する。各 main Worktree のパスは親の Repository と同じにする。
2. 同じ Repository へ Worktree を 1 個ずつ追加し、それぞれの Workflow 状態を反映する。
3. 一覧の更新後、フッターに表示するサーバ情報を更新する。さらに 3 秒間待ち、フッターの情報だけを 1 秒ごとに 3 回更新する。再描画は強制しない。

最初の追加で一覧の行数は期待した 16 行にならず、15 行のままだった。
次の追加では、main Worktree の行を親とする子行挿入を SwiftUI が要求し、子を持たない行への挿入として AppKit が例外を投げた。
例外後もモデルの値は変わるが、フッターは最初の値のまま更新されなかった。
実際の出力から抜粋する。

```text
ROW MISMATCH at 0: 15, expected 16, attached true, current [15]
FOOTER-ONLY tick=2 matched=false model=footer-only-2 view=vafter-step-0 ·  · unspecified · 498010h 15m
RESULT: FAIL, collision mask 7
```

例外の reason は、子を持たない親への挿入を示している。

```text
NSOutlineView error inserting child indexes ... in parent ... (which has 0 children).
```

診断用データの main Worktree のパスだけを親と異なる値にした対照条件では、行の追加とフッターの更新が成功した。
作成画面を使った GitHub Issue、Notion task の対照テストも両方成功した。
これは原因を切り分けるためのデータ変更であり、実際の Worktree のパスを変更する要求ではない。

- 独立アプリ: `/private/tmp/releash-2072-investigation/root-workflow-observation.swift`
- 再現出力: `/private/tmp/releash-2072-investigation/root-workflow-observation-7.log:1`、`:58`
- 対照出力: `/private/tmp/releash-2072-investigation/root-workflow-observation-0.log:19`
- 作成画面の対照出力: `/private/tmp/releash-2072-investigation/workflow-controls.log:46`

Issue に記録された例外は行移動中の `NSOutlineViewException` であり、今回再現した例外は行挿入中の `NSOutlineView` である。
両者が同一原因かは確認していない。
今回の変更は、再現できた行追加の失敗と、その後の画面更新停止を対象とする。

# Scope / Non-goals

変更する対象:

- GitHub Issue または Notion task から Worktree を作成して Workflow を起動する際の、Sidebar の一覧更新。
- その一覧更新を契機とする、画面の状態更新停止。

変更しない対象:

- GitHub Issue と Notion task の取得、Worktree の作成、Workflow の実行の仕様。
- 実行木とファイル一覧の表示仕様。
- Issue に記録された行移動例外の追加再現と、今回の挿入例外との同一原因の確定。

# Requirements

- R-001: main Worktree を含む Repository で、GitHub Issue または Notion task から Worktree を作成して Workflow を起動したとき、配信された各 Worktree が所属する Repository の一覧に表示される。複数の Worktree を連続して作成した場合も満たす。
- R-002: R-001 の作成操作による一覧更新後も、アプリの再起動を要さず、サーバから配信された状態の変化が、一覧とフッターを含む画面に反映され続ける。

# Assumptions

なし。
