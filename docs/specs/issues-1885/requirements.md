# Context

- 正本: [#1885 `[02] Workspaces の表示を全て購読に移す`](https://github.com/siro33950/releash/issues/1885)、[#1878 `[01] 状態の変化を購読で届ける土台を作り、Repository 一覧を移す`](https://github.com/siro33950/releash/issues/1878)
- 最初の周の調査基準は branch `feat/issues/1885` の `42be41e0`。
- 所属は [milestone 99 `02. UI と daemon の間の通信の仕組みを一本化する`](https://github.com/siro33950/releash/milestone/99)。同じ milestone の中で、Review は #1886、Automation は #1887、設定とアプリ全体は #1898、terminal は #1888、接続の確立と接続状態は #1895、再接続時の扱いは #1896 が担う。
- アーキテクチャの正本は `AGENTS.md`。アプリケーションロジックは daemon が所有し、client は表示とレイアウト制御、入力受付、呼び出しと購読、表示用フォーマットだけを担う。
- 通信の分け方は次を確定済みの制約とする。**daemon の状態の読み取りは全て購読にする。単発の呼び出しは、状態を変える操作と、値の更新を要求する操作に限る。** 値の更新を要求する呼び出しは状態を返さず、結果は購読で届く。
- 本 ISSUE は #1878 に依存する。#1878 は `6b961a9c` で main に入っている。#1878 が定めた通信の規則と方針（`docs/specs/issues-1878/requirements.md` の R-001〜R-015）は本 ISSUE でも成立する前提である。
- #1878 が作った土台の形: rpc は `OpenStateStream` / `StartStateSubscription` / `StopStateSubscription`（`proto/client.proto:3134-3136`）。購読の対象は文字列で識別し、版は `StateVersion { epoch, sequence }`。届く事象は `ready` / `snapshot` / `change` / `bookmark`（`proto/client.proto:3372-3381`）。client 側の入口は `subscribeState(target, onValue)`（`src/lib/client.ts:313`）。daemon 側は `Subscriptions`（`src-tauri/src/domain/state_subscription/mod.rs`）と `StateSubscriptionUsecase`（`src-tauri/src/usecase/state_subscription.rs`）。
- #1878 が購読へ移した対象は Repository のパス一覧 1 件だけである（`src-tauri/src/usecase/state_subscription.rs:9`、`src/hooks/useRepoList.ts:13`）。対象は daemon の起動時に登録され、引数を取らない。`Subscriptions` は対象を裸の `String` を鍵とする `HashMap` で持ち（`mod.rs:112`）、未登録の名前での購読は `UnknownTarget` を返す（`mod.rs:177-179`）。値の型は `StatePayload` の `oneof`（`proto/client.proto:3368-3370`）と `StateValues`（`src/lib/client.ts:222`）に列挙されている。
- #1878 の `requirements.md` は、Workspaces のツリーが表示する一覧（`get_workspaces` / `refresh_workspaces` が返す `WorkspaceListSnapshotDto`）の購読への移設を本 ISSUE の対象として明記している。この 2 つは正本 Issue の対象表には無い。
- 正本 Issue の対象表が使用箇所に挙げる `src/hooks/useWorktreeList.ts` は、調査基準時点で存在しない。同じ役割は `src/hooks/useWorkspaceList.ts` が持つ。
- #1861（CLOSED）は「Workspaces の更新中・更新失敗時に一覧を保持し、全体更新ボタンから再取得できるようにする」を要求し、実装済みである。要求事項 2「Workspaces 行に Refresh ボタンを一つ置く。一覧取得に失敗していても更新ボタンを利用できるようにする」と要求事項 4「手動更新で実際の再取得を要求する。自動更新失敗後も、アプリを再起動せず再試行できるようにする」は維持する。
- #1895 は `GetServerInfo` を接続の確立（接続先のインスタンスとリリースの確認）だけに絞る ISSUE であり、読み取りの引き受け先ではない。
- 現行実装の確認先: `src/lib/client.ts`、`src/hooks/useWorkspaceList.ts`、`src/hooks/useWorkspaceTreeNodes.ts`、`src/hooks/useWorkspaceNodeDetail.ts`、`src/hooks/useWorkflowState.ts`、`src/hooks/useBaseBranch.ts`、`src/hooks/useCurrentBranch.ts`、`src/hooks/useIssues.ts`、`src/hooks/useWorkspaceStateCache.ts`、`src/hooks/useGitDirWatcher.ts`、`src/hooks/useRepoList.ts`、`src/components/workspace/WorkspaceList.tsx`、`src/components/workspace/CreateWorktreeModal.tsx`、`src/components/panels/AgentSessionPanel/AgentSessionPanel.tsx`、`src/components/panels/SettingsModal.tsx`、`src/App.tsx`、`src/screens/MainLayout.tsx`、`src/screens/useWorktreeState.tsx`、`src/lib/agentSessionEvents.ts`、`src-tauri/src/adaptor/gateway/push.rs`、`src-tauri/src/usecase/workspace_tree/list_query_service.rs`、`src-tauri/src/usecase/workspace_tree/list.rs`、`src-tauri/src/domain/workspace_tree/refresh.rs`、`src-tauri/src/usecase/watcher.rs`、`src-tauri/src/usecase/git_host/git_host_usecase.rs`、`src-tauri/src/domain/git_host/value_objects/cache.rs`、`src-tauri/src/adaptor/gateway/agent_session/agent_session_history_query_service.rs`、`src-tauri/src/domain/repository/watch_subscriptions.rs`、`proto/client.proto`

# Outcome

対象者は、Releash の Workspaces を使う利用者と、UI と daemon の間の通信を実装・保守する開発者である。

現在、Workspaces の表示は daemon の状態を単発の呼び出しで取り、変更通知と定期実行をきっかけに取り直している。取り直しの起動、失敗時の扱い、再試行は画面ごとに別々に書かれている。git の監視は client が開始を要求し、PR の状態と issue の鮮度は client の定期実行に依存する。同じ状態を返す経路が daemon と client の双方に重複し、一つの取得の失敗が画面ごとに別々の結果になる。

変更後は、Workspaces の表示が使う daemon の状態が #1878 の購読だけで届く。client は購読して届いた結果を描き、取り直し、定期の取り直し、失敗時の再試行を持たない。git とファイルの監視は購読の対象が必要とする間だけ daemon が張り、PR の状態と issue は daemon が取りに行って変化を配信する。利用者が明示的に要求する再走査と取り直しは残るが、それらの呼び出しは状態を返さず、結果は購読で届く。これらの状態を返す単発の呼び出しと、対応する変更通知は無くなる。

# Current Behavior

調査基準 `42be41e0` のコードで確認した挙動である。

## Workspaces 一覧は単発の呼び出しで取り、5 つのきっかけで取り直す

- `useWorkspaceList` は `get_workspaces`（読み取り）と `refresh_workspaces`（再走査つきの取得）を呼び分ける（`src/hooks/useWorkspaceList.ts:62-63`）。`refresh_workspaces` は各 Repository の `rescan_branches` を走らせ（`src-tauri/src/usecase/workspace_tree/list_query_service.rs:43`）、結果の `WorkspaceListSnapshotDto` を返す。
- 取り直しのきっかけは、`workspace-list-changed`（`useWorkspaceList.ts:111`）、`branch-list-sync`（`:142`）、`workflow-execution-changed`（`:143`）、agent session の変化（`:145`）、`window` の `branch-list-refresh` / `workspace-tree-refresh`（`:146-147`）、および可視時の定期実行（`:148-150`）である。
- 定期実行の間隔は、削除中の branch があるときは 1 秒、それ以外は 120 秒である（`useWorkspaceList.ts:124-128`）。削除中かどうかは、client が snapshot の `is_deleting` を見て判定している。
- 取得に失敗すると `requestError` を立てる。`get_workspaces` の失敗は console への出力だけで、一覧は前回の値のまま残る（`useWorkspaceList.ts:69-81`）。
- 一覧の保持、初回の失敗と項目なしの区別、失敗した範囲の記録は daemon の domain が持つ（`src-tauri/src/domain/workspace_tree/refresh.rs` の `WorkspaceListState`、`WorkspaceListEntry`）。client はその結果を表示する（`ListRefreshError`、`WorkspaceList.tsx:1581-1596`）。
- 利用者の操作による再取得は、Workspaces 行の Refresh ボタン 1 つである（`WorkspaceList.tsx:1745-1759`）。#1861 の要求事項 2 のとおり Repository ごとのボタンは撤去済みで、`refreshRepository`（`WorkspaceList.tsx:1794`）は `remove_worktree` / `delete_branch` の直後に結果を反映するための取り直しである（`:1633,1642`）。
- Repository ごとの git ディレクトリの監視は、client が `start_git_dir_watching` を要求して張る（`useWorkspaceList.ts:165-173`）。worktree ごとにも `useGitDirWatcher` が同じ要求を行う（`src/hooks/useGitDirWatcher.ts:5-14`、`src/screens/useWorktreeState.tsx:74`）。この監視の結果が `branch-list-sync` として配信される（`src-tauri/src/adaptor/gateway/push.rs:52`）。daemon 側で監視の寿命を持つのは `WatcherUsecase`（`src-tauri/src/usecase/watcher.rs`）で、Push の購読 id に紐づけて管理する。

## worktree のツリーと Node の詳細も単発の呼び出しと変更通知で動く

- `useWorkspaceTreeNodes` は、ツリーと workflow の履歴を `WorkspaceListContext` の snapshot から読み、選択の突き合わせだけを `get_workspace_tree_selection_reconciliation` で取る（`src/hooks/useWorkspaceTreeNodes.ts:139-145`）。取り直しは `refreshWorktree` 経由で `refresh_workspaces` を呼ぶ。
- `useWorkspaceNodeDetail` は `get_workspace_node_detail` を呼び（`src/hooks/useWorkspaceNodeDetail.ts:72,169,186,203`）、`window` の `workspace-tree-refresh`、agent session の変化、`workflow-execution-changed` で取り直す（`:112-135`）。取得に失敗したときは、前回の詳細があればそれを残す（`:87-95`）。
- `AgentSessionPanel` は `get_agent_session` を呼び（`src/components/panels/AgentSessionPanel/AgentSessionPanel.tsx:346`）、agent session の変化で `attempt` を進めて取り直す（`:323-339`）。
- `WorkspaceList` は `get_workspace_session_node_id`（`:822,895,1042`）、`list_agent_session_history`（`:843,862`）、`list_available_agent_session_providers`（`:983`）を呼ぶ。`get_workspace_session_node_id` は Session の復元・作成・履歴からの再開の直後に、選択すべき Node を引き当てるために使う。操作そのもの（`create_agent_session` など）は agent session の id を返す（`:1031`）。
- `list_agent_session_history` は `limit` と `after` を取るページ送りで、client は `limit: 100` で取り、「Load more Provider history」（`:1268-1276`）で続きを取る。daemon 側の上限は 1 ページ 100 件（`MAX_PAGE_SIZE`）、provider ごとの走査 201 件（`MAX_SCAN_PER_PROVIDER`）で（`src-tauri/src/adaptor/gateway/agent_session/agent_session_history_query_service.rs:15-16`）、対応 provider は Claude と Codex の 2 つである（`src-tauri/src/domain/provider_lifecycle/value_objects/provider_kind.rs:9`）。

## branch・issue・worktree の読み取り

- `list_branches` は `CreateWorktreeModal`（`src/components/workspace/CreateWorktreeModal.tsx:111`）、`useBaseBranch`（`src/hooks/useBaseBranch.ts:23`）、および `SettingsModal`（`src/components/panels/SettingsModal.tsx:405`）から呼ばれる。`SettingsModal` では `get_releash_base` と同じ `Promise.all` で呼ばれており、`get_releash_base` だけが #1898 の対象表にある。`list_branches` はどの ISSUE の対象表にも無い。
- `get_branch_base` は `useBaseBranch`（`:19`）から呼ばれる。取り直しのきっかけは無い。失敗すると値を空にする（`:33-36`）。
- `list_branches_with_status_snapshot` は `CreateWorktreeModal`（`:125`）からだけ呼ばれる。
- `get_current_branch` は `useCurrentBranch`（`src/hooks/useCurrentBranch.ts:13`）から呼ばれ、`MainLayout`（`src/screens/MainLayout.tsx:366`）と `useWorktreeState`（`src/screens/useWorktreeState.tsx:58`）が使う。失敗すると `null` にする。
- `get_cached_issues` は `useIssues`（`src/hooks/useIssues.ts:14`）から 30 秒ごとに呼ばれる（`:39-41`）。「Refresh issues」ボタン（`CreateWorktreeModal.tsx:569`）は `fetch_issues` を呼ぶ（`useIssues.ts:35`）。失敗すると一覧を空にする（`:24-27`）。使うのは `CreateWorktreeModal`（`:483`）だけである。
- `get_cwd`、`get_main_repo_path`、`list_worktrees` は `App.tsx` の起動時に順に呼ばれる（`src/App.tsx:166-172`）。`get_cwd` は daemon プロセスの `std::env::current_dir()` を返し（`src-tauri/src/adaptor/gateway/repository/util.rs:29-34`）、その値はすぐ `get_main_repo_path` へ渡される。client は cwd を表示にも判断にも使わない。`get_main_repo_path` は Repository の追加操作の中でも、利用者が OS のダイアログで選んだパスを解決するために呼ばれる（`App.tsx:193`）。
- `load_workspace_state` は `useWorkspaceStateCache`（`src/hooks/useWorkspaceStateCache.ts:46`）が worktree ごとに 1 回呼ぶ。同じ状態は `save_workspace_state` で client が書き戻す（`:30`）。

## PR の状態と issue の取得

- `GitHostUsecase` は、TTL を見る `get_cached_pr_status` / `get_cached_issues` と、TTL を無視して取りに行く `fetch_pr_status` / `fetch_issues` の 2 系統を持つ（`src-tauri/src/usecase/git_host/git_host_usecase.rs:27-52`）。
- TTL は `CacheTtl`（`src-tauri/src/domain/git_host/value_objects/cache.rs:4-14`）で、値は 30 秒であり PR の状態と issue に同じものが使われる（`src-tauri/src/adaptor/controller/wiring.rs:112`）。
- PR の状態は daemon が Workspaces の一覧を組み立てる中で `get_cached_pr_status` として使う（`list_query_service.rs:50`）。client の 120 秒ごとの取り直しと TTL 30 秒の組み合わせにより、実際に `gh` が走るのは Repository ごとに約 120 秒ごとである。`fetch_pr_status` を呼ぶ client のコードは `src` に無い。
- issue は client の 30 秒ごとの `get_cached_issues` と TTL 30 秒により、Repository ごとに約 30 秒ごとに `gh` が走る。`gh` のタイムアウトは 10 秒である（`src-tauri/src/adaptor/gateway/git_host/github.rs:16`）。

## 正本 Issue の対象表と現在のコードの差

- `get_cached_pr_status` を呼ぶ箇所は `src` に無い。rpc は `proto/client.proto:66,241` に残っている。
- `list_workspace_worktree_nodes` を呼ぶ箇所は `src` に無い（テストを除く）。rpc は `proto/client.proto:107` に残っている。
- `list_workspace_workflow_history` を呼ぶ箇所は `src` に無い（テストを除く）。daemon が一覧の組み立てで使っている（`list_query_service.rs:65`）。rpc は `proto/client.proto:106` に残っている。
- `get_workflow_execution_state` と `resolve_active_execution_by_worktree` を呼ぶのは `useWorkflowState`（`src/hooks/useWorkflowState.ts:24,33`）だけであり、`useWorkflowState` を使う本番のコードは無い。使っているのは `src/hooks/useWorkflowState.test.ts` だけである。

## 変更通知の現在の姿

- `Push` の `oneof event` は `agent_session_changed`、`branch_list_sync`、`file_change`、`git_status_changed`、`review_comments_changed`、`workflow_execution_changed`、`resync`、`workspace_list_changed` を持つ（`proto/client.proto:6-19`）。
- このうち `branch-list-sync`、`workflow-execution-changed`、`agent-session-changed`、`workspace-list-changed` を受けているのは Workspaces の画面だけである（`useWorkspaceList.ts:111,142,143,145`、`useWorkspaceNodeDetail.ts:115,124`、`useWorkflowState.ts:53`、`src/lib/agentSessionEvents.ts:28`、`AgentSessionPanel.tsx:324`）。
- `file-change`、`git-status-changed`、`review-comments-changed` を受けているのは Review と Automation の画面である（`src/hooks/useGitEventRefresh.ts`、`src/hooks/useAutomation.ts`）。

## 監視の上限

- 変更通知の購読は 16 件まで、その下の監視は合計 64 件までである（`src-tauri/src/domain/repository/watch_subscriptions.rs:42,59-66`）。
- `start_git_dir_watching` を要求するのは Workspaces の画面だけである（`useWorkspaceList.ts:168`、`useGitDirWatcher.ts:7`）。`start_watching`（ファイル監視）を要求するのは Review と Automation の画面である（`useGitEventRefresh.ts:51`、`useAutomation.ts:124`）。

# Scope / Non-goals

## 変更する対象

- Workspaces の表示が使う次の読み取りの購読への移設と、その読み取りを返す単発の呼び出しの削除
  - `get_workspaces`
  - `get_workspace_tree_selection_reconciliation`、`list_workspace_worktree_nodes`、`list_workspace_workflow_history`
  - `get_workspace_node_detail`
  - `get_workflow_execution_state`、`resolve_active_execution_by_worktree`
  - `get_agent_session`、`get_workspace_session_node_id`、`list_agent_session_history`、`list_available_agent_session_providers`
  - `list_branches`、`get_branch_base`、`list_branches_with_status_snapshot`、`get_cached_pr_status`
  - `get_current_branch`
  - `get_cached_issues`
  - `list_worktrees`、`get_main_repo_path`、`get_cwd`
  - `load_workspace_state`
- `SettingsModal` の base branch 選択肢が使う branch の一覧（`src/components/panels/SettingsModal.tsx:405`）の購読への移設。`list_branches` の rpc を残さないため
- 値の更新を要求する呼び出しへの作り替え。`refresh_workspaces` と `fetch_issues` は状態を返さなくなる
- `branch-list-sync`、`workflow-execution-changed`、`agent-session-changed`、`workspace-list-changed` の変更通知の削除
- Workspaces の画面が持つ取り直し、定期の取り直し、失敗時の扱い、再試行の削除
- git のディレクトリとファイルの監視の所有者の移動（client の要求から、購読の対象が必要とする間の daemon の監視へ）。本 ISSUE で移すのは Workspaces の購読対象が必要とする分
- PR の状態と issue の取得のきっかけの daemon への移動と、変化の配信
- 本 ISSUE で使われなくなるコードと、本 ISSUE で触れたファイルの中で使われていないコードの削除

## 変更しない対象

- `SubscribePush`、`Push`、`WatchFiles`、`WatchGitDirectory`、`StopWatching`、および監視の上限（`src-tauri/src/domain/repository/watch_subscriptions.rs`）の削除。正本 Issue はこれを #1885・#1886・#1887・#1898 のうち最後に終わる ISSUE で行うと定めており、#1886・#1887・#1898 は本 ISSUE の確定時点でいずれも OPEN である。`Push` は `file-change`、`git-status-changed`、`review-comments-changed` を運ぶために残る
- Review の表示の読み取り（#1886）、Automation の表示の読み取り（#1887）、terminal の出力（#1888）
- 設定とアプリ全体の表示の読み取り（#1898）。`SettingsModal` のうち本 ISSUE が触るのは base branch 選択肢の branch の一覧だけであり、`get_releash_base`、`get_external_editor`、`get_workflow_config` などは #1898 が扱う
- `file-change`、`git-status-changed`、`review-comments-changed` の変更通知
- 接続を確立する `GetServerInfo` と、接続状態の client 側での保持（#1895）
- 再接続時に画面を作り直さない扱い（#1896）
- 状態を変える操作の呼び出し方。単発の呼び出しのまま変えない
- 購読の仕組みで差分を送る対象の実装。差分を使うのは terminal だけであり、#1888 が扱う
- #1878 が定めた購読の土台の規則（版番号、つなぎ直し、送り待ち、印、重複、後始末）そのもの

# Requirements

- R-001: Workspaces の表示が使う daemon の状態は、#1878 の購読で届く。対象は Workspaces の一覧、worktree のツリーと選択の突き合わせ、Node の詳細、agent session とその履歴と利用できる provider、branch の一覧と base と状態、現在の branch、issue の一覧、worktree の一覧、worktree ごとの保存済み表示状態である。
- R-002: R-001 の状態を返す単発の呼び出しは無い。例外は設けない。
- R-003: 購読の単位は、画面が使う読み取り結果 1 つである。複数の状態の組み合わせは daemon が行い、client は届いた結果をそのまま表示できる。
- R-004: Workspaces の表示は、client からの取り直しの呼び出しを行わずに、daemon の状態の変化を反映する。
- R-005: client に、Workspaces の表示のための定期的な取り直し、変更通知を受けての取り直し、取得の失敗を受けての再試行は無い。
- R-006: `branch-list-sync`、`workflow-execution-changed`、`agent-session-changed`、`workspace-list-changed` の変更通知は無い。
- R-007: Workspaces の表示のための git のディレクトリとファイルの監視は、購読の対象が必要とする間だけ daemon が張る。その対象の購読が全て終われば監視も終わる。client は、これらの対象のための監視の開始と停止を要求しない。
- R-008: PR の状態と issue は daemon が取りに行き、変わったときに購読へ配信する。その対象の購読者がいる間は 30 秒以内に取り直される。利用者は取り直しを要求でき、その要求では 30 秒を待たずに取りに行く。
- R-009: Workspaces の一覧の取得が失敗しても、Workspaces は最後に得られた一覧を表示し続ける。失敗した Repository と worktree には、失敗したことと前回の情報を表示していることが示される。初回の取得の失敗は「項目なし」と区別される。
- R-010: 利用者は Workspaces の一覧の再走査を要求でき、要求のたびに Repository の走査がやり直される。一覧の取得に失敗していても要求できる。
- R-011: 利用者は agent session の履歴の表示件数を増やすことができ、増やした後の件数の履歴が届く。
- R-012: Workspaces と Settings の base branch 選択肢が使う branch の一覧は、いずれも購読で届く。
- R-013: worktree と session から引き当てた Node の id、および client が渡した任意のパスから解決した repository のルートは、購読で届く。
- R-014: daemon の起動ディレクトリから解決した repository のルートは購読で届く。daemon の作業ディレクトリそのものを返す呼び出しは無い。
- R-015: 本 ISSUE で使われなくなるコードと、本 ISSUE で触れたファイルの中で使われていないコードは無い。対象は Rust、TypeScript、proto、およびそれらだけを対象とするテストである。
- R-016: 値の更新を要求する呼び出しは状態を返さない。要求の結果は購読で届く。

# Assumptions / Open Questions

- Assumption（自動判断）: R-007 と B-009 の「client は監視の開始と停止を要求しない」は、Workspaces の表示のための監視に限る。Scope / Non-goals は本 ISSUE で移す監視を「Workspaces の購読対象が必要とする分」に限っており、ファイルの監視を要求するのは Review と Automation の画面だけである（`src/hooks/useGitEventRefresh.ts:51`、`src/hooks/useAutomation.ts:124`）。この 2 つは #1886・#1887 まで旧来の経路のまま残るため、無限定の記述は Non-goals と矛盾していた。既存の挙動を維持し対象範囲を広げない解釈として、Workspaces の購読対象が必要とする監視に限定した。
- Open Question: なし。
