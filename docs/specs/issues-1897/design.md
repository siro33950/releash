# Design

## 変える部分
- workflow の定義の保存・削除と facet の削除の後の表示: `src/hooks/useAutomation.ts` の `save_workflow_source` の戻り値による表示の書き込み（:143-147）と、`delete_workflow`・`delete_facet` の成否による選択の解除（:153-163, :230-231）をやめ、購読の値だけで表示する。根拠: R-001「状態を変える操作の戻り値、操作の成否からの画面側の推定（…）を、表示の出どころにしない」、B-001。ルート: 委任
- AgentSession の表示状態: `src/components/panels/AgentSessionPanel/AgentSessionPanel.tsx` の open・restore の戻り値（:102-140）、失敗時の推定（:141-148）、delete の成功（:159-163）による表示状態の書き込みをやめ、購読の AgentSession の状態だけで表示する。購読の状態で足りない表示状態は daemon の購読の値に足す。根拠: R-001、B-001。ルート: 委任
- Provider の実行ファイルの設定と Notion の設定の表示: `src/hooks/useProviderAvailabilitySettings.ts` の `update_provider_executable` の戻り値との突き合わせ（:109-130, :207-215）と、`src/hooks/useNotionSettings.ts` の削除後の画面側の書き換え（:200-210）をやめ、購読の値だけで表示する。根拠: R-001、B-001。ルート: 委任
- Session を選ぶ経路: restore・resume・create の操作が Session Node の ID を返す形にする。daemon は、node が購読に流れる状態に入った後（その状態の commit の後）に ID を返す。`src/components/workspace/WorkspaceList.tsx` の `firstState` による `session-node` の引き当て（:813, :862, :994）をやめ、返った ID の node を選ぶ。根拠: R-002、R-003、B-002、B-003。ルート: 下記「固定するルート」1
- worktree の作成の後の表示: `src/components/workspace/CreateWorktreeModal.tsx` は、戻り値の path による移動を残し、branch とリポジトリ名の表示を購読から取る。根拠: R-002、B-002。ルート: 委任
- 起動時に開く worktree と起動時のリポジトリの一覧への追加: 「worktree が 1 つだけなら開く」の判断と、path からのリポジトリ名の切り出しを daemon に移す。`startup-repository` の購読の値が、開く worktree（path・branch・リポジトリ名）を持つ。起動時のリポジトリの一覧への追加（`src/App.tsx:130` の `initFromCwd`、`src/hooks/useRepoList.ts:30-32`）を daemon が起動時に行い、`initFromCwd` を消す。画面は `src/App.tsx:123-148` で、届いた値のとおりに起動時に 1 回だけタブを開く。根拠: R-004、B-004。ルート: 下記「固定するルート」2
- リポジトリの root: `src/App.tsx:150-163` の `repository-root` の購読の 1 回だけの読み取りを、選んだディレクトリに対する計算の単発の呼び出しにする。根拠: R-005、B-005。ルート: 下記「固定するルート」2
- 使い手の無い購読の対象と画面の状態: `session-node` と `repository-root` の購読の対象を、daemon 側の対象・読み取り・proto・生成コードまで消す。`src/screens/useWorktreeState.tsx` の `gitRefreshKey`・`refreshGit` と、それを呼ぶ箇所を消す。根拠: R-006、B-006。ルート: 委任
- terminal の入力: `src/hooks/useTerminal.ts` で、購読で届くプロセスの状態（:502-504）が動いていない間は xterm の入力を止め、プロセスが動いていないことを terminal に表示する。打鍵を捨てる処理（:769）を無くす。起動の完了時にプロセスが動いていないために送れない入力（:514-516）は、起動できなかったことを画面に出す。バッファの上限を超えた分（:290-294）は `onTerminalErrorRef` で画面に出す。根拠: R-007、R-008、B-007、B-008、B-009。ルート: 委任
- 利用者の操作の失敗の表示: `src/App.tsx:232-234`、`src/components/panels/DiffFileTree.tsx:134-144`、`src/components/panels/DiffCommentList.tsx:146-147`、`src/lib/terminalLinkActivation.ts:4-6`、`src/components/workspace/CreateWorktreeModal.tsx:182-183` の失敗を、原因とともに画面に出す。根拠: R-009、B-010。ルート: 委任
- 更新の確認の失敗: `src/hooks/useUpdateChecker.ts:55-57` で、失敗を更新が無いときと区別して画面に出す。根拠: R-010、B-011。ルート: 委任
- workspace の状態の購読の失敗: `src/hooks/useWorkspaceStateCache.ts:44-63` で、失敗を「状態が無い」と同じ値にしない。根拠: R-011、B-012。ルート: 委任
- 読まれない失敗の記録: `usecase/retry.rs` の `Retrying` を、呼び出し元が失敗を記録するかを指定できる形にする。`repository_scan`・`terminal_checkpoint`・`provider_session_title_list`・`provider_session_title`・`provider_lifecycle_append`・`provider_lifecycle_resolution`・`workflow_recovery_list`・`client_request_limit` の書き手は記録しない指定にし、ログにだけ残す。`workflow_recovery` は記録を残す。根拠: R-012、B-013、B-014。ルート: 下記「固定するルート」3
- node の起動の失敗の記録: `adaptor/gateway/workflow/workflow_host.rs:1399-1402` と `usecase/workflow/node_startup.rs:145-148` の `workflow_node_start` の記録を消す。根拠: R-013、B-015、B-016。ルート: 委任
- HTTP local API のステータスコード: `adaptor/presenter/api_error.rs` の、性質を見ない 503（:41-45, :118-123）と 500（:63-72, :106-110）を、Connect（`adaptor/presenter/connect.rs`）と同じく失敗の分類と性質で決める形にする。根拠: R-014、B-017、B-018。ルート: 委任
- presenter の外のステータスコードの選択: `adaptor/controller/api/client.rs:61-68`、`adaptor/controller/api/provider_lifecycle.rs:64,149`、`adaptor/controller/api/workflow.rs:169,338`、`adaptor/controller/client/dispatch.rs:85,132,149` で controller が選んでいるコードの種類を、失敗として presenter へ渡し、presenter が決める形にする。根拠: R-014、B-017。ルート: 委任
- JoinError の扱い: `adaptor/controller/terminal_surface.rs:124`、`adaptor/controller/client/` の `repository/mod.rs:36`・`git_host/mod.rs:15`・`comment/commands.rs:16`・`app_config/commands.rs:7`・`code/mod.rs:29`・`external_editor/commands.rs:20`・`notion/commands.rs:9`・`workflow/facet.rs:32,48,65`・`workflow/definition.rs:24,32,41,52,77` を、`client/dispatch.rs:142-146` と同じく `TechnicalFailure` を通して性質に従う形にする。根拠: R-015、B-019。ルート: 下記「固定するルート」4
- 監視のパスの失敗: `usecase/workspace_tree/list.rs:154-170` の `watch_paths` が、worktrees と repository_root の失敗を捨てずに返す。`:104` は変えない。根拠: R-016、B-020。ルート: 下記「固定するルート」7
- 失敗のやり直しの待ち: Notion の 429 の再送（`adaptor/gateway/notion/service_impl.rs:70-108`）を common の `attempts`・`RetryLimiter`・`RetryBackoff` で回し、Retry-After を待ち時間の計算より優先する指定を common の部品に足して使う。`MAX_RETRIES` を消す。SQLite の busy handler（`adaptor/gateway/local_event_store/connection.rs:47-57`）は待ち時間を `RetryBackoff` で計算し、期限を呼び出しの期限と合成する。`adaptor/gateway/local_event_store/store.rs:215,289` の `busy_timeout` の直接指定をやめ、同じ handler を通す。根拠: R-017、R-018、R-019、B-022、B-023。ルート: 下記「固定するルート」5
- 確かめのループと取り消し可能な待ち: `infrastructure/file_lock.rs:21-41`、`infrastructure/process/search_path.rs:72-89`、`infrastructure/platform/desktop_restart.rs:48-57`、`infrastructure/platform/single_instance.rs:19-24` の確かめる間隔を common の `RetryBackoff` の新しい定数で計算し、期限を operation_context の `with_timeout` で呼び出しの期限と合成する。`adaptor/gateway/application_lifecycle.rs:101-103` と `adaptor/gateway/workflow/workflow_host/node_startup.rs:79-90` の待ちを operation_context の `wait`・`sleep` にする。根拠: R-017、R-024、B-021、B-028、B-029。ルート: 下記「固定するルート」5
- `RetryBackoff::SERVICE`: `common/retry.rs:15` を消し、`adaptor/gateway/provider_lifecycle/event_repository_impl.rs:70,214` を `RetryBackoff::ITEM` にする。根拠: R-020、B-024。ルート: 下記「固定するルート」6
- `spawn_blocking` の直接の使用: Current Behavior の 19 か所を、operation_context の `spawn_blocking` で期限と取り消しの文脈を引き継ぐ形にする。根拠: R-021、B-028。ルート: 委任
- worktree の削除: `usecase/repository_usecase.rs:307-318` で、削除を operation_context の `spawn_blocking` で待ち、remove と `set_branch_base_override` の失敗を呼び出しの失敗として返す。成功・失敗のどちらでも、削除中を解いた後に `notify_repository_changed` を呼ぶ。根拠: R-021、R-022、B-026。ルート: 下記「固定するルート」7
- Workspaces の取り直し: `usecase/workspace_tree/list.rs:128-152` の `refresh` で、PR の状態の取り直しを Repository ごとに並列に、終わるまで待つ。走査（:131-135）と PR の状態の取り直しの失敗を、読み取りの失敗として購読に載せる。根拠: R-021、R-023、B-027。ルート: 下記「固定するルート」7
- 処理の先の期限: gh（`adaptor/gateway/git_host/github.rs:13,61-62`）、login shell の PATH の取得（`infrastructure/process/search_path.rs:12,32,49-101`）、Notion の HTTP、外部プロセスの期限を、呼び出しから引き継いだ期限と合成し、先に来る方を採る。根拠: R-024、B-028、B-029。ルート: 委任
- HTTP local API の期限・取り消し・同時実行の枠: `adaptor/controller/api/mod.rs:26-55` の HTTP local API の入口に、Connect と同じ既定の期限、operation_context の `ingress` による期限と切断時の取り消し、Connect と同じ `PriorityLimits` を分け合う同時実行の枠を掛ける。経路を既存の段に振り分ける。根拠: R-025、B-030、B-031。ルート: 下記「固定するルート」8
- command の完了の監視: `adaptor/gateway/workflow/workflow_host.rs:1569-1580` の `spawn_blocking` と `block_on` をやめ、async のまま tokio のタスクとして起動する。根拠: R-021、R-026、B-025、B-032。ルート: 下記「固定するルート」9
- `block_on` の残り: `adaptor/controller/client/repository/worktree.rs:20-33` は `run_blocking` と `block_on` をやめて `remove_worktree` を直接 await する。`adaptor/gateway/notion/service_impl.rs:619-630` の `send` と `infrastructure/process/output.rs:10-35` の `output` を async にし、呼び出し元（`adaptor/gateway/code/staging.rs:127`、`adaptor/gateway/git_host/github.rs:62` を含む）も async にして daemon のランタイム上で await する。`adaptor/gateway/terminal_surface/runtime_gateway_impl.rs:888,1345` の `checkpoint_io` を sync の排他にし、flush の closure で `block_on` しない。根拠: R-026、B-032。ルート: 下記「固定するルート」9
- 監視の失敗: `usecase/state_subscription.rs` の `reconcile_watches` を、失敗した監視の要求の一覧を返す形にする。Workspaces の更新（:317-321）では、失敗した要求が支える単位へ失敗を届ける。張った後の監視が壊れたとき（`adaptor/gateway/repository/state.rs:102,145-151`）は、その worktree の状態を読み取りの失敗にして購読に届け、次の走査か張り直しの成功で解く。購読の対象全体に掛かる共通のファイル監視（`usecase/state_subscription.rs` の `WatchRequirement::Files`、`infrastructure/file_watcher/mod.rs`）も、張った後に壊れたときは、その監視の要求を持つ購読の対象へ読み取りの失敗を届け、その要求を張り直しの対象に戻して、次の張り直しの成功で解く。根拠: R-028、B-034、B-035、B-036。ルート: 下記「固定するルート」10
- 文書: `AGENTS.md` の「アーキテクチャ原則」に、「サーバは状態を配信し、client は購読する。client からサーバへの単発の呼び出しは、状態を変える操作と、client の入力に対する計算だけ」を原則として足す。根拠: R-029、B-037。ルート: 委任

## 固定するルート
1. 状態を変える操作は、作ったもの・対象の識別子だけを返してよい（AIP-133、Kubernetes の create と同じ）。画面はそれを選ぶ・移る先の識別にだけ使い、表示の中身は購読から取る。Session Node の ID は、daemon が node を購読に流れる状態に入れた後に返す。こうすると画面は購読に現れるまで待つだけでよく、待ちに独自の期限や not found の失敗を足さない。
2. 起動時に開く worktree の判断、リポジトリ名の切り出し、起動時のリポジトリの一覧への追加は、起動時の判断として daemon が持つ。画面に残すのは、届いた値のとおりに起動時に 1 回だけタブを開く表示の制御だけ。リポジトリの root は client の入力に対する計算なので、購読ではなく単発の呼び出しにする。root があるときは一覧に足し、無いときはタブで開く。この分岐は画面の操作の選択として画面に残す。
3. 読まれない失敗の記録は、画面に出す先を足さず、記録しない。`Retrying` に記録の有無の指定を足す。`Retrying` が usecase にあること自体はマイルストーン #97 で扱うため動かさない。node の起動の失敗は、node の状態と失敗の記録の 2 か所で同じ失敗を表さないため、記録しない。
4. 打ち切りの JoinError は、`client/dispatch.rs:142-146` と同じく `TechnicalFailure` を通して性質に従わせる。
5. 再試行と待ちは、gRPC の再試行の設計（A6）の server pushback と RFC 9110 の Retry-After に合わせ、次の 3 つに分ける。
   - 失敗のやり直し（Notion の 429、SQLite の busy）: common の `attempts`・`RetryLimiter`・`RetryBackoff` で回す。Retry-After は、backoff の計算より優先する相手の指定として common に足す。止めるのは呼び出しの期限と再試行の予算による。
   - 状態が変わるのを待つ確かめのループ（ファイルロック、login shell の終了、前の UI の終了、起動中の UI の socket）: 確かめる間隔は今の値（10 ms・20 ms）のまま `RetryBackoff` の新しい定数として表す。既存の `ITEM`・`RECOVERY`・`CONFLICT` は失敗のやり直し用の値なので転用しない。`RetryLimiter` は掛けない。期限は呼び出しの期限と合成する。呼び出しから始まらない起動時の処理は、今の期限の値を使う。
   - 取り消し可能な待ち: operation_context の `wait`・`sleep` を使う。
6. `RetryBackoff::SERVICE` は消す。provider lifecycle の event の保存は、hook の 1 回の呼び出しごとの 1 件の保存なので、1 件ずつの操作のやり直しの `ITEM` を使う。client のつなぎ直しの規則を daemon の保存の再試行に使わない。Tauri のシェルが proto の値から `RetryBackoff` を組み立てる形（`adaptor/gateway/desktop_client.rs`）は、proto の 1 か所の値を読むので今のまま残す。
7. 呼び出しから始まる処理（worktree の削除、Workspaces の取り直し）は、呼び出しの中で終わりまで待つ。待つ側は operation_context の `spawn_blocking` で期限と取り消しを引き継ぎ、gh の 10 秒は呼び出しの期限と合成する。`watch_paths` は worktrees と repository_root の失敗を返す。
8. HTTP local API の同時実行の枠は、Kubernetes の API Priority and Fairness に合わせ、サーバ全体で 1 つの仕組みに、入口ではなく呼び出しの種類で段を振り分ける。Connect と同じ `PriorityLimits`（64 席）を分け合い、workflow の output の submit・validate・get（Connect の `WorkflowSubmitOutput`・`WorkflowValidateOutput`・`WorkflowGetOutput` と同じ操作）と provider の hook の signal は `workflow`、それ以外は `default` にする。期限と取り消しは Connect と同じ形にする。
9. command の完了の監視は、起動した呼び出しの期限と取り消しを引き継がない。寿命の持ち主は `command_completion_observers` で、command の完了、`shutdown_active_command_execution` などの workflow の操作、daemon の終了で止める。登録と、完了で外す動きは残す。`checkpoint_io` を渡している先が await をまたいで持つ箇所は、`spawn_blocking` の中で取る。
10. 監視の失敗は、一部が読めないときは読めた分を返し、読めなかった分を失敗として示す（AIP-217 の unreachable、Kubernetes の status conditions）。Workspaces の値全体を失敗に置き換えない。repository・worktree 単位の監視の要求（git ディレクトリ等）の失敗は、Workspaces の値の中のその要素の失敗（`usecase/fetched.rs` の `Fetched` の error、`usecase/workspace_tree/list.rs:106` と同じ形）として載せる。購読の対象全体に掛かる監視（workflows のディレクトリ等）の失敗は、その購読の対象に `publish_failure` で届ける。後始末の失敗（`usecase/state_subscription.rs:377-379, :480-482, :497-499`）はログに残し、次の reconcile で止め直す。

## 変えないもの
- `workflow_recovery` の失敗の記録と、Workspace ツリーの要対応への表示。実際に読まれているため。
- 同時実行の枠の段（`interactive`・`workflow`・`default`）の追加と配分（30・40・120、待ち行列 50）。#1894 で決めたもののため。
- Tauri のシェルが proto の `connection_backoff` から `RetryBackoff` を組み立てる形（`adaptor/gateway/desktop_client.rs`）。proto の 1 か所の値を読む形のため。
- 起動時の分類の読み取り専用の接続（`adaptor/gateway/local_event_store/store.rs:162-220` の `open_schema_inspection`）。store を開く手順の一部で、`ReaderPool` ができる前に走り、read-mark を取らず sidecar を変えない別の開き方が要るため。分類以外の読み取りに使う場所を広げない。
- `usecase/workspace_tree/list.rs:104` の `repository_root(..).ok()`。直前の worktrees の読み取りが同じ root の失敗を `scanned.error` に入れて購読に残すため。

## 未確定・リスク
なし
