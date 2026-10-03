# Context

- 入力文書: https://github.com/siro33950/releash/issues/1897（マイルストーン #99「02. UI と daemon の間の通信の仕組みを一本化する」の [15]）
- 依存の ISSUE（#1878〜#1980 のうち本文の「依存」に挙がったもの）は全て main に入っている。開始時点は main `4533af2f`
- マイルストーン #99 の方針: daemon が持っている状態は daemon が配信し、client は購読する。単発の呼び出しは、状態を変える操作と、client の入力に対する計算だけにする。同じことをする処理は、全ての呼び出し元が同じコードを使い、別の実装を残さない。client と daemon の間の規則は proto 側に 1 か所で定義する。確立した標準（Kubernetes の list + watch と API Priority and Fairness、gRPC の接続状態・再試行・期限、tokio-rusqlite の作り）に合わせる
- 規約: `AGENTS.md`「アーキテクチャ原則」、`docs/architecture/`（`USECASE.md`「購読の配信の口」「失敗」、`GATEWAY.md`「出ていく側の横断的関心事」）
- 共通の部品として、期限と取り消しは `src-tauri/src/common/operation_context.rs`、待ち時間の計算と再試行は `src-tauri/src/common/retry.rs`（`RetryBackoff`・`RetryLimiter`・`attempts`）、同時実行の枠は `src-tauri/src/common/priority.rs`・`concurrency.rs` にある。Connect の入口は、同時実行の枠（`interactive`・`workflow`・`default` の 3 段、64 席、待ち行列 50。#1894）、既定の期限 120 秒、取り消しを掛けている
- client のつなぎ直しの規則は proto の `ClientService` の option（`connection_backoff` ほか）にあり、画面と Tauri のシェルはそこから読む
- 失敗の記録（`FailureRecordRepository`）はプロセス内のメモリにあり、読み手は Workspace ツリーの要対応だけで、node の id・node_execution_id・execution_id で引く
- 利用者のいない段階であり、互換性を理由に変更を止めない

# Outcome

対象者: Releash の画面と CLI の利用者、および daemon と画面・Tauri のシェルの通信を保守する開発者。

現在の問題: マイルストーン #99 の各 ISSUE の後も、通信の土台となる仕組みについて、共通の実装を通らない箇所がコード全体に残っている。状態を変える操作の戻り値や画面側の推定が表示の出どころになっている。購読が 1 回だけの取得として使われている。利用者の入力や操作の失敗が表示されずに捨てられている。読まれない失敗の記録や、解けない失敗の記録がある。同じ失敗が入口によって違うステータスコードになる。独自の再試行と待ちのループや、呼び出しから切り離されて止まらない処理がある。HTTP local API には期限・取り消し・同時実行の枠が掛かっていない。監視が失敗しても古い値が今の値として出続ける。また、通信の原則が文書に無い。

変更後の状態: 状態は購読だけで画面に届き、単発の呼び出しは状態を変える操作と client の入力に対する計算だけになる。入力や操作の失敗は画面に出る。失敗は入口に関係なく同じステータスコードになる。待ち・再試行・期限・取り消し・同時実行は、全ての呼び出し元が共通の実装を通る。通信の原則が `AGENTS.md` に原則として書かれている。

# Current Behavior

開始時点（main `4533af2f`）で確認した挙動。パスは `src-tauri/src` からの相対（`src/` で始まるものと `proto/` を除く）。

状態を画面へ届ける仕組み

- `SaveWorkflowSource` の戻り値の `WorkflowDto` で選択中の workflow の表示を書き（`src/hooks/useAutomation.ts:143-147`）、購読（`:86-88`）も同じ値を書く。`delete_workflow`（`:153-163`）と `delete_facet`（`:230-231`）は、呼び出しの成否だけで画面側で選択を外す
- `AgentSessionPanel` は、`OpenAgentSession`・`RestoreAgentSession` の戻り値（`src/components/panels/AgentSessionPanel/AgentSessionPanel.tsx:102-140`）、失敗時の画面側の推定（`:141-148`）、`DeleteAgentSession` の成功（`:159-163`）、購読の `session.lifecycle`（`:178-185`）の 4 つから、同じ表示状態を書く
- `update_provider_executable` の戻り値を、購読の値と突き合わせて表示を決める（`src/hooks/useProviderAvailabilitySettings.ts:109-130,207-215`）
- `delete_notion_config` の成功後に、購読（`src/hooks/useNotionSettings.ts:90`）とは別に、画面側で設定と下書きを書き換える（`:200-210`）
- Session を restore・resume・create した後、購読 `session-node` を 1 回だけ読んで node の ID を引き当てる（`src/components/workspace/WorkspaceList.tsx:813,862,994`）。最初の値が null なら「… Session Node was not found」の失敗になり、後から届く値を待たない（`src/lib/client.ts:579-595` の `firstState`）
- 起動時に、購読 `startup-repository` と `worktrees` を 1 回だけ読み、画面側で起動時のリポジトリを一覧に足し（`src/App.tsx:130`、`src/hooks/useRepoList.ts:30-32`）、「worktree が 1 つだけならそのタブを開く」を判断する（`src/App.tsx:123-148`）
- リポジトリを足すとき、利用者が選んだディレクトリの root を、購読 `repository-root` を 1 回だけ読んで求める（`src/App.tsx:150-163`）
- `create_worktree` の戻り値の branch とリポジトリ名を、作った worktree のタブの表示に使う（`src/components/workspace/CreateWorktreeModal.tsx:174-191`）
- `gitRefreshKey`・`refreshGit`（`src/screens/useWorktreeState.tsx:65-67,144-145`）は値を増やすだけで、読む箇所が無い

接続の状態と、入力・操作の失敗

- terminal で、プロセスが動いていないときの打鍵（`src/hooks/useTerminal.ts:769`）と、起動前にためた入力のうち起動の完了時にプロセスが動いていない分（`:514-516`）を、何も表示せずに捨てる。起動前の入力のバッファの上限を超えた分は、開発者向けのログに警告を出して捨てる（`:290-294`）
- 何も表示せずに捨てる、または開発者向けのログに出すだけの失敗: メニューの有効・無効の切り替え（`src/App.tsx:232-234`）、パスのコピー（`src/components/panels/DiffFileTree.tsx:134-138,140-144`）、レビューのスレッドの削除（`src/components/panels/DiffCommentList.tsx:146-147`）、terminal のリンクを開く操作（`src/lib/terminalLinkActivation.ts:4-6`）
- 更新の確認の失敗が「更新なし」と同じ表示になる（`src/hooks/useUpdateChecker.ts:55-57`）
- workspace の状態の購読が失敗すると、失敗を画面に出した後、「状態が無い」と同じ値として扱う（`src/hooks/useWorkspaceStateCache.ts:44-63`）
- worktree の作成の失敗で、原因を捨てて「Failed to create: <branch>」だけを出す（`src/components/workspace/CreateWorktreeModal.tsx:182-183`）

失敗の記録

- 次のキーの失敗の記録は、target が node の ID ではないため、どこからも読まれない: `repository_scan`（`adaptor/controller/repository_scan.rs:42`）、`terminal_checkpoint`（`adaptor/controller/terminal_checkpoint.rs:63`）、`provider_session_title_list`・`provider_session_title`（`adaptor/controller/provider_session_title.rs:18,36`）、`provider_lifecycle_append`・`provider_lifecycle_resolution`（`adaptor/gateway/provider_lifecycle/event_repository_impl.rs:66-69,207-210`）、`workflow_recovery_list`（`adaptor/controller/workflow_startup.rs:21`）、`client_request_limit`（`adaptor/controller/api/client_priority.rs:53`）。`workflow_recovery`（`adaptor/controller/workflow_startup.rs:29`）は target が execution_id で、読まれている
- `workflow_node_start` の失敗の記録（`adaptor/gateway/workflow/workflow_host.rs:1399-1402`、`usecase/workflow/node_startup.rs:145-148`）を解く経路が無い。一時的な失敗のやり直しでは node_execution_id が替わるため、古い ID の記録が残る。node 自身も失敗の状態になる（`workflow_host.rs:1410-1417`）ため、同じ失敗が 2 か所で表される

失敗の分類とステータスコード

- HTTP local API は、`WorkflowError::Store`（`adaptor/presenter/api_error.rs:41-45`）と `ProviderLifecycleIngressUsecaseError::Store`・`StorageUnavailable`（`:118-123`）を性質を見ずに 503 に、`WorkflowError::Technical`（`:63-67`）・`Editor`（`:68-72`）・`ProviderLifecycleIngressUsecaseError::Technical`（`:106-110`）を性質を見ずに 500 にする。Connect は同じ失敗を性質に従ったコードにする（`adaptor/presenter/connect.rs:239-256,498-499`）
- presenter の外で、どの種類のステータスコードにするかを選んでいる: `adaptor/controller/api/client.rs:61-68`、`adaptor/controller/api/provider_lifecycle.rs:64,149`、`adaptor/controller/api/workflow.rs:169,338`、`adaptor/controller/client/dispatch.rs:85,132,149`
- `spawn_blocking` の JoinError を、性質を見ずに INTERNAL にする: `adaptor/controller/terminal_surface.rs:124`、`adaptor/controller/client/repository/mod.rs:36`、`client/git_host/mod.rs:15`、`client/comment/commands.rs:16`、`client/app_config/commands.rs:7`、`client/code/mod.rs:29`、`client/external_editor/commands.rs:20`、`client/notion/commands.rs:9`、`client/workflow/facet.rs:32,48,65`、`client/workflow/definition.rs:24,32,41,52,77`。`client/dispatch.rs:142-146` は性質に従う
- Workspaces の監視のパスを求めるとき、worktree の一覧と root の読み取りの失敗を捨てる（`usecase/workspace_tree/list.rs:158-166`）

再試行と待ち

- 共通の待ち時間の実装を使わない、独自の再試行と待ちのループがある: Notion の 429 の再送（`adaptor/gateway/notion/service_impl.rs:70-108`。Retry-After を待ち、最大 2 回）、SQLite の busy handler（`adaptor/gateway/local_event_store/connection.rs:47-57`。自前の 2 秒の期限、1 ms ごと）と起動時の `busy_timeout` の直接指定（`adaptor/gateway/local_event_store/store.rs:215,289`）、ファイルロックの 10 ms ごとの確認（`infrastructure/file_lock.rs:21-41`）、login shell の終了の 10 ms ごとの確認（`infrastructure/process/search_path.rs:72-89`）、前の UI の終了の 20 ms ごとの確認（`infrastructure/platform/desktop_restart.rs:48-57`）、起動中の UI の socket の 20 ms ごとの確認（`infrastructure/platform/single_instance.rs:19-24`）、取り消しを見ない待ち（`adaptor/gateway/application_lifecycle.rs:101-103`）、独自の取り消しの待ち（`adaptor/gateway/workflow/workflow_host/node_startup.rs:79-90`）
- `common/retry.rs:15` の `RetryBackoff::SERVICE` が、proto の `connection_backoff`（`proto/client.proto:2663-2669`）と同じ値を持つ。使い手は provider lifecycle の event の保存の再試行（`adaptor/gateway/provider_lifecycle/event_repository_impl.rs:70,214`）だけ

期限・取り消し・同時実行

- worktree の削除は、削除を待たずに成功を返し、削除の失敗を捨てる。期限と取り消しの文脈を渡さない（`usecase/repository_usecase.rs:307-318`）
- Workspaces の取り直しは、PR の状態の取り直しを待たずに返し、gh は自前の 10 秒の期限だけで動く。PR の取り直しと走査の失敗はログだけに残る（`usecase/workspace_tree/list.rs:128-152`、`adaptor/gateway/git_host/github.rs:13,61`）
- 期限と取り消しの文脈を落とす `tokio::task::spawn_blocking` の直接の使用が 19 か所ある（本文の一覧と一致）
- login shell の PATH の取得が、呼び出しの期限と合成せず、固定の 5 秒の期限だけで動く（`infrastructure/process/search_path.rs:12,32,49-101`）
- HTTP local API は認証だけを掛け、期限・取り消し・同時実行の枠が掛かっていない（`adaptor/controller/api/mod.rs:26-55`、`infrastructure/local_api/server.rs:113-150`）
- sync から `block_on` で async を動かしている: `adaptor/controller/client/repository/worktree.rs:20-33`、`adaptor/gateway/workflow/workflow_host.rs:1569-1580`、呼び出しごとにランタイムを作る `adaptor/gateway/notion/service_impl.rs:619-630` と `infrastructure/process/output.rs:10-35`、`adaptor/gateway/terminal_surface/runtime_gateway_impl.rs:888,1345`

監視の失敗

- Workspaces の更新で監視を張り直すのに失敗しても、ログに残すだけで、読み取りの値を今の値として配信する（`usecase/state_subscription.rs:317-330`）。購読の開始のとき（`:229-232`）だけは止めて失敗を返す
- 張った後のファイルと git ディレクトリの監視が壊れても、ログに残すだけで、キャッシュした状態が出続ける（`adaptor/gateway/repository/state.rs:102,145-151`）

満たしていることを確認した事項

- daemon の状態の変化を画面へ届ける経路は購読だけである。stream を返す rpc は `OpenStateStream`（`proto/client.proto:2676`）だけで、画面の Tauri の event と Channel（`src/hooks/useMenuEvents.ts:27`、`src/hooks/useNativeFileDrop.ts:56`、`src/components/panels/TerminalPanel.tsx:119`、`src/hooks/useUpdateChecker.ts:73`、`src/components/DaemonBoundary.tsx:53-60`）はどれも Tauri のシェル由来で、daemon の状態ではない
- 画面が daemon に張る stream は client ごとに 1 本で（`src/lib/client.ts:358-440`）、terminal の出力も同じ stream に載る（`src/lib/client.ts:270-272`、`adaptor/controller/api/client.rs:224-242`）
- 単位時間あたりのやり直しの回数の上限（`RetryLimiter`）の実体は、daemon に 1 つだけである（`adaptor/controller/daemon.rs:69`）
- 全体で 1 つの同時実行の枠は無い。`Semaphore` を作るのは段ごとの `common/concurrency.rs:53,56` だけで、`PriorityLimits` は `adaptor/controller/daemon.rs:31` で 1 つ作られ、入口の包みとして掛かる

文書

- 「サーバは状態を配信し、client は購読する。client からサーバへの単発の呼び出しは、状態を変える操作と、client の入力に対する計算だけ」という原則は、`AGENTS.md`、`docs/architecture/`、`docs/glossary/` のどこにも無い

# Scope / Non-goals

変更するもの:

- Current Behavior に挙げた全ての箇所（画面、daemon、CLI の HTTP の呼び出し、Tauri のシェルの待ちのループ）と、同じ種類でこの開発の作業中に足されたコード
- 起動時に開く worktree の判断と起動時のリポジトリの一覧への追加を daemon に移すこと、リポジトリの root を求める単発の呼び出しを足すこと、使い手の無くなる購読の対象（`session-node`・`repository-root`）を消すこと
- `AGENTS.md` の「アーキテクチャ原則」への原則の追加

変更しないもの:

- 規約の置き場所そのものを決め直す必要があるもの（マイルストーン #97 で扱う）: 一時的な失敗のやり直しが usecase にあること（`usecase/retry.rs` の `Retrying`、`AttemptProgress` を引数に取る Usecase）と controller で再試行を回していること、計測と取り消しの確認が処理の中に書かれていること、部品の表に無い種類の trait（`PerformanceOutput`・`TelemetryPort`）、gateway が presenter の転送の型を使っていること、domain の各エラーの enum が技術的な失敗の変種を複数持つこと、workflow host の gateway が状態の変化を知らせていること、provider の文字列化の重複、Notion の設定の保存の入力を presenter が domain の型に変えていること、workflow の一覧が不正・重複の定義を説明文で表していること
- Tauri のシェルの監督（#1904 で扱う）: 監督のループと `DesktopClient` の作り直し、接続の確立の固定の期限、`adaptor/gateway/daemon_supervision.rs` の待ちのループ、`desktop.rs:83` の `block_on`
- ログイン項目の表示（`src/hooks/useAppSettings.ts:40,75-82`）。Tauri のシェルのコマンドで、daemon の状態ではないため
- 計測用の画面の切り替え（`src/main.tsx:32`、`src/lib/terminalPerformanceSwitches.ts:21` の `performance-switches`）
- 認証・Origin・未知の経路・identity の HTTP のステータスコード（`adaptor/controller/api/auth.rs`、`adaptor/controller/api/mod.rs:39`、`infrastructure/local_api/server.rs:146`）
- 起動時に store を開く手順の中の、既存のファイルの分類のための読み取り専用の接続（`adaptor/gateway/local_event_store/store.rs:162-220` の `open_schema_inspection`）。store を開く前の手順であり、読み書きの入口の対象外とする。分類以外の読み取りには使わない
- 同時実行の枠の段の追加と配分（30・40・120、待ち行列 50）
- 監視の後始末の失敗（`usecase/state_subscription.rs:377-379,480-482,497-499`）を画面に出すこと。届け先の購読がもう無いため
- 本文の「確かめること」を確かめた結果、開始時点で満たしていない次の事項。この ISSUE では直さない
  - 画面の各画面に、自分で取り直す処理が無いこと: terminal の画面が、購読の失敗・processed の失敗・適用の失敗・入力の失敗を合図に、購読を張り直して取り直す（`src/hooks/useTerminal.ts:470-473,543-557,584-620,743`）
  - 購読の上限と後始末が種類ごとに無いこと: 普通の購読と terminal の購読が、別々の登録表と後始末を持つ（`usecase/state_subscription.rs:114-118,486-500`、`usecase/terminal_surface/subscription.rs:33,68-104`、`adaptor/controller/api/client.rs:154-178`）。上限は terminal の配信だけが持つ（`adaptor/presenter/terminal_subscription.rs:111`）
  - 接続の状態を持つのが client の 1 か所だけであること: Tauri のシェルの daemon への client が、接続・つなぎ直し・無通信の検知を画面とは別に持つ（`adaptor/gateway/desktop_client.rs:153-155,197-220,339-372`）
  - 呼び出し元で失敗の分類を判定し直さず、分類を捨てて別の分類に置き換えず、失敗を文字列にして落とさないこと（`docs/architecture/USECASE.md:57-62`）: usecase の `usecase/workflow/workspace_tree.rs:249,279,308,337`、`usecase/workflow/mod.rs:324`、`usecase/review_usecase.rs:1063-1064`、`usecase/workflow/control_plane.rs:519`、`usecase/app_data_gc/mod.rs:386`、`usecase/workflow/runtime_command.rs:219-221`、`usecase/workflow/execution_archive.rs:266-278`、`usecase/desktop_update.rs:14-55`、`usecase/repository_state/service.rs:247-250`、`usecase/agent_session/provider_availability.rs:226-229`、`usecase/agent_session/usecase.rs:95,115,119,175,234,249,264,278,295,311`、`usecase/agent_session/agent_session_lifecycle.rs:272,346,364,388,435,470,568,576,747`、`usecase/agent_session/agent_session_rename.rs:63`、`usecase/agent_session/agent_session_launch.rs:431,485,489,606,715,722,879,913,967`、`usecase/provider_lifecycle/mod.rs:86,118`。gateway・presenter の `adaptor/gateway/workflow/node_session_boundary.rs:105,329,341` と `adaptor/gateway/workflow/runtime_command_gateway.rs:112-113`、`adaptor/gateway/workflow/workflow_host/lifecycle_commands.rs:58`、`adaptor/gateway/workflow/workflow_host.rs:608,614,730`、`adaptor/gateway/workflow/workflow_host/isolated_worktree.rs:91`、`adaptor/presenter/terminal_subscription.rs:99-118`、`adaptor/gateway/shared/error_handling.rs:11-29`、`adaptor/gateway/code/error.rs:11-29` ほか外部ライブラリの失敗を文字列で持つ箇所、`adaptor/gateway/workflow/diagnostics.rs:1141-1157`（エラーメッセージの文字列で診断コードを判定）。controller の `adaptor/controller/repository_scan.rs:175`、`adaptor/controller/agent_session_wiring.rs:103,166,188,210`、`adaptor/controller/command/desktop_lifecycle.rs:94-126`、`adaptor/controller/daemon.rs:144,222,401,441,605`。画面の `src/hooks/useTerminal.ts:741-745`
  - daemon の中の繰り返し処理を、controller と infrastructure の駆動部から Usecase を呼ぶ形だけにすること: gateway が tokio のタスクを起動し、その中で Usecase のやり直しを呼ぶ（`adaptor/gateway/workflow/workflow_host/node_startup.rs:117`）
  - 購読の配信の口（`docs/architecture/USECASE.md:22-27`）を Usecase への入力の経路にしないこと: `StateSubscriptionDelivery` の `claim`・`start` の値で Usecase が分岐し（`usecase/state_subscription.rs:34-41,50,181-183`）、配信の失敗で購読を止める（`:236-241`）。`TerminalSurfaceStateSink`・`TerminalSurfaceEventSink`（`usecase/terminal_surface/output.rs:44-59`）の `initialize` の結果で起動の巻き戻しを決め（`usecase/terminal_surface/spawn_usecase.rs:80-84`）、`remove` の値を gateway が使い（`adaptor/gateway/terminal_surface/runtime_gateway_impl.rs:1062-1072`）、`wait_output` で配信の流量制御が PTY の読み取りを止める

# Requirements

状態を画面へ届ける仕組み

- R-001: daemon の状態の表示は、購読から届く値だけで決まる。状態を変える操作の戻り値、操作の成否からの画面側の推定、1 回だけの購読の読み取りを、表示の出どころにしない
- R-002: 状態を変える操作が返してよいのは、作ったもの・対象の識別子（Session Node の ID、worktree の path 等）だけである。画面はそれを、選ぶ・移る先の識別にだけ使う
- R-003: Session を restore・resume・create する操作は、その Session Node の ID を返す。ID が返るのは、その node が購読に流れる daemon の状態に入った後である。画面は返った ID の node を選び、その node が購読に現れるまで待って表示する。「Session Node was not found」の失敗にしない
- R-004: 起動時に開く worktree（path・branch・リポジトリ名）と、起動時のリポジトリを一覧に入れることは、daemon が決める。画面は、購読で届いた値のとおりに起動時に 1 回だけタブを開き、リポジトリの一覧は購読で受け取る
- R-005: 利用者が選んだディレクトリのリポジトリの root は、そのディレクトリに対する計算の単発の呼び出しで求める。結果は root がある・無いのどちらかで、失敗は画面に出る
- R-006: 使い手の無い購読の対象と、値を増やすだけで読まれない画面の状態が残っていない

接続の状態と、入力・操作の失敗

- R-007: terminal のプロセスが動いていない間は、terminal は入力を受け付けず、プロセスが動いていないことを terminal に表示する。打鍵を表示無く捨てない
- R-008: 起動前にためた terminal の入力を、起動の完了時にプロセスが動いていないために送れないときは、起動できなかったことを画面に出す。起動前の入力のバッファの上限を超えた分を捨てるときは、捨てたことを画面に出す
- R-009: 利用者の操作（メニューの有効・無効の切り替え、パスのコピー、レビューのスレッドの削除、terminal のリンクを開く、worktree の作成を含む）の失敗は、原因とともに画面に出る。表示無く捨てず、開発者向けのログだけにしない
- R-010: 更新の確認が失敗したときは、更新が無いときと区別して画面に出る
- R-011: 購読の読み取りの失敗を、「値が無い」と同じ値として扱わない

失敗の記録

- R-012: どこからも読まれない記録先に失敗を記録しない。`repository_scan`・`terminal_checkpoint`・`provider_session_title_list`・`provider_session_title`・`provider_lifecycle_append`・`provider_lifecycle_resolution`・`workflow_recovery_list`・`client_request_limit` の失敗は記録せず、ログにだけ残る。画面の表示は今と変わらない。`workflow_recovery` の失敗は今のまま記録され、Workspace ツリーの要対応に出る
- R-013: workflow の node の起動の失敗は、失敗の記録に書かず、node 自身の失敗の状態と、一時的な失敗のやり直しで表す。Workspace ツリーの要対応として残り続けない

失敗の分類とステータスコード

- R-014: 同じ失敗は、Connect と HTTP local API のどちらの入口でも、失敗の分類（業務の失敗の種類、技術的な失敗の性質）で決まる同じ種類のステータスコードになる。技術的な失敗は、HTTP local API でも性質で決まる
- R-015: 処理の打ち切り（取り消し・panic 等）による失敗は、性質に従ったステータスコードになり、一律の INTERNAL にならない
- R-016: 監視のパスを求めるときの worktree の一覧と root の読み取りの失敗を捨てず、監視の失敗として R-028 のとおり画面に届く

再試行と待ち

- R-017: daemon と Tauri のシェルの再試行と待ちの間隔は、Tauri のシェルの監督（#1904 の範囲）を除き、全て common の 1 つの実装で計算される。独自の間隔や回数のループが残っていない。状態が変わるのを待つ確かめのループの間隔は今の値（10 ms・20 ms）から変わらず、再試行の予算は掛からない
- R-018: 相手が待ち時間を指定する失敗（Notion の 429 の Retry-After）は、その指定を待ち時間の計算より優先して待つ。やり直しを止めるのは、呼び出しの期限と再試行の予算であり、独自の回数の上限ではない
- R-019: SQLite の busy の待ちは、起動時を含めて全て同じ待ちの処理を通り、待ち時間は common の実装で計算され、期限は呼び出しの期限と合成される
- R-020: client の再試行とつなぎ直しの規則は proto 側の 1 か所にあり、同じ値の規則を daemon の中に別に持たない。provider lifecycle の event の保存の再試行は、1 件ずつの操作のやり直しの規則で行う

期限・取り消し・同時実行

- R-021: 呼び出しから始まる処理は、呼び出しから切り離されたまま止まらずに残らない。期限と取り消しの文脈を落として処理を起動しない。呼び出しより長く続く処理（command の完了の監視）は、寿命の持ち主に登録され、command の完了・workflow の操作・daemon の終了で止まる
- R-022: worktree の削除の呼び出しは、削除が終わるまで待って結果を返す。削除の失敗は呼び出しの失敗として画面に出る。成功・失敗のどちらでも、削除中の表示は購読の上で解ける
- R-023: Workspaces の取り直しの呼び出しは、走査と PR の状態の取り直しが終わるまで待つ。走査と PR の状態の取り直しの失敗は、読み取りの失敗として購読に載り、画面に出る
- R-024: 処理の先（外部プロセス、HTTP、gh、login shell 等）の期限は、呼び出しから引き継いだ期限と合成され、先に来る方が採られる。呼び出しから始まらない起動時の処理は、今の期限の値で動く
- R-025: HTTP local API の呼び出しには、Connect と同じ形で、既定の期限、期限と取り消しの引き継ぎ、同時実行の枠が掛かる。同時実行の枠は Connect と同じ枠（64 席）を分け合い、呼び出しの種類で既存の段に振り分けられる。workflow の output の submit・validate・get と provider の hook の signal は `workflow`、それ以外は `default` である
- R-026: daemon の実行時のコード（入口の main を除く）に、sync から `block_on` で async の処理を動かす箇所と、呼び出しごとにランタイムを作る箇所が残っていない。Tauri のシェルの監督（#1904 の範囲）は対象外である
- R-027: store の読み込みと書き込みの入口は、開いた後の store について、それぞれ async の 1 本ずつだけである

監視の失敗

- R-028: 監視を張る・張り直すのに失敗したときや、張った後の監視が壊れたときは、その監視が支える単位（repository・worktree の要素、または購読の対象全体）の読み取りの失敗として購読に届き、画面に出る。読めている他の要素の値は失われない。古い値を今の値として出し続けない。次の張り直しや走査が成功したら、失敗は解ける

満たしたままの事項

- R-030: daemon の状態の変化が画面へ届く経路は、購読だけのままである
- R-031: 画面が daemon に張る stream は、client ごとに 1 本だけのままである
- R-032: 単位時間あたりのやり直しの回数の上限の実体は、daemon に 1 つだけのままである
- R-033: 全体で 1 つの同時実行の枠を持たず、枠は受け手の側の包みとして掛かったままである

文書

- R-029: `AGENTS.md` の「アーキテクチャ原則」に、「サーバは状態を配信し、client は購読する。client からサーバへの単発の呼び出しは、状態を変える操作と、client の入力に対する計算だけ」が原則として書かれている。個々の実装の詳細（型名、定数、呼び出しの一覧）と、原則から導ける個別の規則を書かず、`docs/architecture/`・`docs/glossary/DOMAIN.md` の内容を複製しない

# Assumptions

なし
