# Design 01

## 開始状態

初回の周であり、既存の `design-NN.md` は無い。開始状態の実装は `docs/specs/issues-1885/requirements.md` の Current Behavior を参照する。

- 差分の基準は base ブランチ `main`、派生点は `feat/issues/1885` の `42be41e0`。未コミットの変更は `docs/specs/issues-1885/` の文書だけで、実装は `42be41e0` のままである。
- 依存する #1878 は `6b961a9c` で main に入っており、購読の土台（`OpenStateStream` / `StartStateSubscription` / `StopStateSubscription`、`Subscriptions`、`StateSubscriptionUsecase`、`subscribeState`）は開始状態に存在する。購読済みの対象は Repository のパス一覧 1 件だけである。
- この周までに解消・見送りとなった Thread は無い（open Thread 0 件）。

## 変える部分

### 購読の土台の拡張

- 購読対象の値オブジェクトの新設: `domain/state_subscription` に購読対象を表す値オブジェクトを新設し、対象名の組み立て・解釈・引数の検証をそこが所有する。`Subscriptions` の鍵を、裸の `String`（`src-tauri/src/domain/state_subscription/mod.rs:112`）からその型にする。根拠: R-001「Workspaces の表示が使う daemon の状態は、#1878 の購読で届く」、R-002「R-001 の状態を返す単発の呼び出しは無い。例外は設けない」、B-001、B-002。ルート: 固定するルートの 1 つめ。
- 監視の寿命を購読から導く: 上記の値オブジェクトが「必要とする監視（種類とパス）」を持ち、`Subscriptions` が購読者の増減から開始すべき監視・停止すべき監視を導く。監視の実行（I/O）は usecase が行う。根拠: R-007「Workspaces の表示のための git のディレクトリとファイルの監視は、購読の対象が必要とする間だけ daemon が張る。その対象の購読が全て終われば監視も終わる」、B-007、B-008、B-009。ルート: 固定するルートの 2 つめ。
- 購読で運ぶ値の型の追加: 本 ISSUE で増える購読対象の値を `StateValue`（Rust）と `StatePayload` の `oneof`（`proto/client.proto:3368-3370`）、`StateValues`（`src/lib/client.ts:222`）に足す。根拠: R-001、R-003「購読の単位は、画面が使う読み取り結果 1 つである。複数の状態の組み合わせは daemon が行い、client は届いた結果をそのまま表示できる」、B-001、B-003。ルート: 委任。

### 読み取りの購読への移設と rpc の削除

いずれも「その読み取りを購読の対象へ移し、対応する単発の rpc を削除する」変更である。対象の切り方（どの読み取りを 1 つの対象にまとめ、どの引数で分けるか）は委任。

- `get_workspaces`（`proto/client.proto:3138`、呼び出し元 `src/hooks/useWorkspaceList.ts:62`）。根拠: R-001（対象のうち「Workspaces の一覧」）、R-002、B-001、B-002。ルート: 委任。
- `get_workspace_tree_selection_reconciliation`（`:3201`、呼び出し元 `src/hooks/useWorkspaceTreeNodes.ts:139-145`）。根拠: R-001（「worktree のツリーと選択の突き合わせ」）、R-002、B-001、B-002。ルート: 委任。
- `get_workspace_node_detail`（`:3199`、呼び出し元 `src/hooks/useWorkspaceNodeDetail.ts:72,169,186,203`）。根拠: R-001（「Node の詳細」）、R-002、B-001、B-002。ルート: 委任。
- `get_agent_session`（`:3171`、呼び出し元 `src/components/panels/AgentSessionPanel/AgentSessionPanel.tsx:346`）。根拠: R-001（「agent session」）、R-002、B-001、B-002。ルート: 委任。
- `get_workspace_session_node_id`（`:3200`、呼び出し元 `src/components/workspace/WorkspaceList.tsx:822,895,1042`）。操作の戻り値へ畳まず購読にする。根拠: R-013「worktree と session から引き当てた Node の id …は、購読で届く」、R-002、B-020。ルート: 固定するルートの 4 つめ。
- `list_agent_session_history`（`:3208`、呼び出し元 `src/components/workspace/WorkspaceList.tsx:843,862`）。購読の対象を「表示している件数」で定義し、`limit` / `after` のページ送りを無くす。根拠: R-011「利用者は agent session の履歴の表示件数を増やすことができ、増やした後の件数の履歴が届く」、R-001、R-002、B-019。ルート: 固定するルートの 5 つめ。
- `list_available_agent_session_providers`（`:3209`、呼び出し元 `src/components/workspace/WorkspaceList.tsx:983`）。根拠: R-001（「利用できる provider」）、R-002、B-001、B-002。ルート: 委任。
- `list_branches`（`:3210`、呼び出し元 `src/components/workspace/CreateWorktreeModal.tsx:111`、`src/hooks/useBaseBranch.ts:23`、`src/components/panels/SettingsModal.tsx:405`）。Settings の base branch 選択肢も同じ購読へ移し、rpc を残さない。根拠: R-012「Workspaces と Settings の base branch 選択肢が使う branch の一覧は、いずれも購読で届く」、R-002、B-015。ルート: 委任。
- `get_branch_base`（`:3175`、呼び出し元 `src/hooks/useBaseBranch.ts:19`）。根拠: R-001（「branch の …base」）、R-002、B-001、B-002。ルート: 委任。
- `list_branches_with_status_snapshot`（`:3211`、呼び出し元 `src/components/workspace/CreateWorktreeModal.tsx:125`）。根拠: R-001（「branch の …状態」）、R-002、B-001、B-002。ルート: 委任。
- `get_current_branch`（`:3178`、呼び出し元 `src/hooks/useCurrentBranch.ts:13`）。根拠: R-001（「現在の branch」）、R-002、B-001、B-002。ルート: 委任。
- `get_cached_issues`（`:3176`、呼び出し元 `src/hooks/useIssues.ts:14`）。根拠: R-001（「issue の一覧」）、R-002、R-008、B-011。ルート: 委任。
- `list_worktrees`（`:3218`、呼び出し元 `src/App.tsx:166-172`）。根拠: R-001（「worktree の一覧」）、R-002、B-001、B-002。ルート: 委任。
- `get_main_repo_path`（`:3184`、呼び出し元 `src/App.tsx:166-172,193`）。client が渡した任意のパスからの解決を購読にする。根拠: R-013「…client が渡した任意のパスから解決した repository のルートは、購読で届く」、R-002、B-021。ルート: 固定するルートの 4 つめ。
- `get_cwd`（`:3179`、呼び出し元 `src/App.tsx:166-172`）。daemon の作業ディレクトリを返す rpc は削除し、起動ディレクトリから解決した repository のルートを 1 つの購読対象にする。根拠: R-014「daemon の起動ディレクトリから解決した repository のルートは購読で届く。daemon の作業ディレクトリそのものを返す呼び出しは無い」、B-022。ルート: 固定するルートの 4 つめ。
- `load_workspace_state`（`:3219`、呼び出し元 `src/hooks/useWorkspaceStateCache.ts:46`）。書き戻しの `save_workspace_state` は状態を変える操作として単発のまま残す。根拠: R-001（「worktree ごとの保存済み表示状態」）、R-002、B-001、B-002。ルート: 委任。

### 呼び出し元の無い読み取りの rpc の削除

- `get_cached_pr_status`（`proto/client.proto:3177`）、`list_workspace_worktree_nodes`（`:3217`）、`list_workspace_workflow_history`（`:3216`）の rpc を削除する。いずれも `src` の本番コードに呼び出し元が無い。PR の状態と workflow の履歴を daemon が一覧の組み立てに使う内部の経路（`src-tauri/src/usecase/workspace_tree/list_query_service.rs:50,65`）は残る。根拠: R-002「R-001 の状態を返す単発の呼び出しは無い」、R-015「本 ISSUE で使われなくなるコードと、本 ISSUE で触れたファイルの中で使われていないコードは無い」、B-002、B-016。ルート: 委任。
- `get_workflow_execution_state`（`:3197`）と `resolve_active_execution_by_worktree`（`:3239`）の rpc を削除し、これだけを使う `src/hooks/useWorkflowState.ts` とその `useWorkflowState.test.ts` を削除する。本番のコードから `useWorkflowState` を使っている箇所は無い。workflow の実行状態は R-001 の購読の対象に含まれない。根拠: R-002、R-015、B-002、B-016。ルート: 委任。

### client 側の取り直し・通知・監視要求の削除

- Workspaces の画面が持つ取り直しと再試行の削除: 変更通知を受けての取り直し、`window` イベント（`branch-list-refresh` / `workspace-tree-refresh`）を受けての取り直し、可視時の定期実行（削除中の branch があるとき 1 秒、それ以外 120 秒）、issue の 30 秒ごとの取り直し、取得の失敗を受けての再試行を無くす。対象は `src/hooks/useWorkspaceList.ts:111-150`、`src/hooks/useWorkspaceNodeDetail.ts:112-135`、`src/components/panels/AgentSessionPanel/AgentSessionPanel.tsx:323-339`、`src/hooks/useIssues.ts:39-41`、`src/lib/agentSessionEvents.ts`。根拠: R-004「Workspaces の表示は、client からの取り直しの呼び出しを行わずに、daemon の状態の変化を反映する」、R-005「client に、Workspaces の表示のための定期的な取り直し、変更通知を受けての取り直し、取得の失敗を受けての再試行は無い」、B-004、B-005。ルート: 委任。
- 変更通知の削除: `Push` の `oneof event` から `branch_list_sync`、`workflow_execution_changed`、`agent_session_changed`、`workspace_list_changed` の 4 つを削除し、daemon 側の送出（`src-tauri/src/adaptor/gateway/push.rs`）と client 側の受け口を無くす。根拠: R-006「`branch-list-sync`、`workflow-execution-changed`、`agent-session-changed`、`workspace-list-changed` の変更通知は無い」、B-006。ルート: 委任。
- client からの git ディレクトリ監視の要求の削除: `start_git_dir_watching` の要求（`src/hooks/useWorkspaceList.ts:165-173`、`src/hooks/useGitDirWatcher.ts:5-14`、`src/screens/useWorktreeState.tsx:74`）を無くす。監視の開始・停止は購読から導く（土台の 2 つめの項目）。根拠: R-007、B-009。ルート: 委任。

### 外部の情報の取得

- PR の状態と issue を daemon が取りに行く: 対象の購読者がいる間は daemon が取りに行き、変わったときに購読へ配信する。鮮度の規則は `domain/git_host` の `CacheTtl`（30 秒、`src-tauri/src/domain/git_host/value_objects/cache.rs:4-14`）だけとし、取り直しの間隔を別に持たない。利用者の取り直しの要求では 30 秒を待たずに取りに行く。根拠: R-008「PR の状態と issue は daemon が取りに行き、変わったときに購読へ配信する。その対象の購読者がいる間は 30 秒以内に取り直される。利用者は取り直しを要求でき、その要求では 30 秒を待たずに取りに行く」、B-010、B-011、B-017。ルート: 固定するルートの 3 つめ。

### 値の更新を要求する呼び出し

- `refresh_workspaces`（`proto/client.proto:3137`）が `WorkspaceListSnapshotDto` を返すのをやめ、状態を返さない形にする。再走査の結果は購読で届く。根拠: R-016「値の更新を要求する呼び出しは状態を返さない。要求の結果は購読で届く」、R-010、B-023、B-018。ルート: 委任。
- `fetch_issues`（`:3169`）が `ListIssueInfoDto` を返すのをやめ、状態を返さない形にする。根拠: R-016、B-023。ルート: 委任。
- `fetch_pr_status`（`:3270`、`PrStatusDto` を返す。`src` に呼び出し元が無い）: 利用者の取り直しの要求に割り当てるなら状態を返さない形にし、どの UI 操作にも割り当てないなら削除する。根拠: R-016、R-008、R-015、B-023、B-017。ルート: 委任（利用者の取り直しの要求をどの UI 操作に割り当てるかは委任の範囲）。

### 使われなくなるコードの削除

- 上記の変更で使われなくなる Rust・TypeScript・proto のコードと、それらだけを対象とするテストを削除する。本 ISSUE で触れたファイルの中で使われていないコードも残さない。根拠: R-015、B-016。ルート: 委任。

## 固定するルート

人間が指定したものだけを書く。粒度は指定されたままとする。

1. 購読対象を表す値オブジェクトを `domain/state_subscription` に新設し、対象名の組み立て・解釈・引数の検証をそこが所有する。`Subscriptions` の鍵をその型にする。範囲: `domain/state_subscription` と、対象名と値オブジェクトを相互変換する adaptor。粒度: 所有者の指定まで。型名・モジュール配置・文字列表現は委任。理由: 購読対象が 1 件から 15 件前後へ増え、対象名の形式と引数の検証（空のパス、0 件、上限 100 件超、存在しない対象の拒否）が最も増える規則になるため。`AGENTS.md` のレビュー観点「ドメインの規則（判断・計算・分類・検証・遷移）を domain が所有しているか」に従う。
2. 上記の値オブジェクトが「必要とする監視（種類とパス）」を持ち、`Subscriptions` が購読者の増減から開始すべき監視・停止すべき監視を導く。監視の実行（I/O）は usecase が行う。範囲: `domain/state_subscription` と購読の usecase。粒度: 所有者の指定まで。実行側の作り（`WatcherUsecase` の再利用か新設か）は委任。理由: 「購読者が居る間だけ」は購読の状態遷移そのものであり、購読者数を知っているのは `Subscriptions` だけ。対象と必要な監視を 2 か所に分けると、対象を増やすたびに離れた対応表を直すことになる。
3. 外部情報の鮮度は `domain/git_host` の `CacheTtl` を唯一の規則とし、取り直しの間隔を別に持たない。範囲: `domain/git_host` と購読の usecase。粒度: 所有者の指定まで。取りに行く実装の形は委任。理由: PR の状態と issue を同じ 30 秒に揃えたため対象ごとに違う間隔を持つ必要が無く、同じ 30 秒を「取り直しの間隔」と「キャッシュの失効」の 2 か所に持たない。
4. worktree と session から引き当てた Node の id、client が渡した任意のパスから解決した repository のルート、daemon の起動ディレクトリから解決した repository のルートは、いずれも購読の対象にする。範囲: adaptor と usecase。粒度: 購読にするという指定まで。対象の切り方は委任。理由: 「すべての読み込みは購読にする」という方針。`get_workspace_session_node_id` を操作の戻り値へ畳む案は採らない。`get_cwd` は削除し、起動ディレクトリからの解決結果を 1 つの対象にする。
5. agent session の履歴は、購読の対象を「表示している件数」で定義する。件数を増やすときは購読を張り直す。範囲: `domain/state_subscription` の対象の定義と、履歴を組み立てる usecase。粒度: 対象の定義の指定まで。件数を増やす UI の作りは委任。理由: ページ送りを単発の呼び出しで残す案は #1878 の規則に反し、上限（provider ごと 201 件・2 provider で最大 402 件）まで全件送る案は `AGENTS.md` の full-retention 回避に反する。#1878 の `StartStateSubscription` / `StopStateSubscription` で足りるため rpc を増やさない。

## 変えないもの

- Workspaces 行の Refresh ボタン（`src/components/workspace/WorkspaceList.tsx:1745-1759`）。ボタンは 1 つのままで、一覧の取得に失敗していても使え、押すと Repository の走査を実際にやり直す。理由: #1861 の要求事項 2・4 が利用者に見える機能として実装済みであり、本 ISSUE が求めるのは取り直しの処理の削除であって機能の取り消しではない。
- 一覧の保持、失敗した範囲の表示、初回の取得の失敗と「項目なし」の区別。所有者は `src-tauri/src/domain/workspace_tree/refresh.rs` の `WorkspaceListState` / `WorkspaceListEntry` であり、表示は `ListRefreshError`（`src/components/workspace/WorkspaceList.tsx:1581-1596`）。理由: 開始状態で満たされており、購読へ移した後も同じ所有者のまま維持する。
- 旧 Push の枠組み（`SubscribePush`、`Push`、`WatchFiles`、`WatchGitDirectory`、`StopWatching`）と監視の上限（`src-tauri/src/domain/repository/watch_subscriptions.rs`）。理由: これらの削除は #1885・#1886・#1887・#1898 のうち最後に終わる ISSUE で行うと正本 Issue が定めており、確定時点で #1886・#1887・#1898 はいずれも OPEN である。`Push` は `file-change` / `git-status-changed` / `review-comments-changed` を運ぶために残る。
- Review と Automation が要求するファイル監視（`start_watching`。`src/hooks/useGitEventRefresh.ts:51`、`src/hooks/useAutomation.ts:124`）。理由: 本 ISSUE で移す監視は Workspaces の購読対象が必要とする分だけであり、この 2 つは #1886・#1887 まで旧来の経路のまま残る。
- Settings の他の設定の読み取り（`get_releash_base`、`get_external_editor`、`get_workflow_config` など）。理由: 本 ISSUE が Settings で触るのは base branch 選択肢が使う branch の一覧だけであり、残りは #1898 が扱う。
- 状態を変える操作の呼び出し方。単発の呼び出しのまま変えない。`save_workspace_state` は既に `Unit` を返しており R-016 を満たすため変えない。

## 未確定・リスク

- 自動判断（R-007 と B-009 の範囲の限定）: 「client は監視の開始と停止を要求しない」を、Workspaces の表示のための監視に限ると解釈して Requirements と Behavior を修正した。無限定の記述は、ファイルの監視を要求する Review と Automation を #1886・#1887 まで残すという Non-goals と矛盾していた。詳細は `requirements.md` の Assumptions を参照する。
- `[DEFERRED]` で人間へ渡した件: 無し。
- 未決のまま残した要求: 無し。
