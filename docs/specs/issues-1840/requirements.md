# Context

- 正本: [#1840 \[workflow\] 起動時の処理と、メモリに常駐する Workflow の状態を整理する](https://github.com/siro33950/releash/issues/1840)
- 所属: [マイルストーン #90「01. Workflow 操作と状態の簡素化」](https://github.com/siro33950/releash/milestone/90)。ISSUE 本文は着手順 3・`#1839` に依存、マイルストーン側は着手順 5・`#1839` と `#1836` に依存と書く。いずれの先行 ISSUE も close・merge 済みであり、着手可否は変わらない
- この ISSUE に統合済み: [#1837](https://github.com/siro33950/releash/issues/1837)（起動時の登録失敗による無制限の再試行）、[#1828](https://github.com/siro33950/releash/issues/1828)（AgentSession 側が書いた事実がエンジンに届かない）。どちらも not planned で close 済みで、調査記録は各 ISSUE に残る
- 先行して merge 済みの変更: `9b30f626`（#1838）、`b642f94d`（#1839）、`482dbdda`（#1826）、`81ec380b`（#1836）、`6cc1572a`（#1835）
- 永続化は event store（事実ログ）であり、実行木と Node の状態は記録された事実からの導出である（`AGENTS.md`、`docs/glossary/DOMAIN.md`）
- [#1839](https://github.com/siro33950/releash/issues/1839) により、Node の状態には中断も失敗も無く、プロセスが居るかどうかは状態と別の 1 項目として読める（`NodeProcessPresence`、`src-tauri/src/adaptor/gateway/workflow/node_process.rs`）。また Node の起動失敗は規定回数まで自動で起動し直される
- [#1836](https://github.com/siro33950/releash/issues/1836) により、実行木の完了は事実として記録され、完了または Abort の事実が無く保存定義を解釈できない実行は起動時に理由付きで Abort される
- 事実ログへ書くのはエンジンだけではない。AgentSession も、プロセス終了・再開・Session の紐づけなどの事実を同じ記録へ直接書く（`src-tauri/src/adaptor/gateway/agent_session/agent_session_repository.rs:100-178`）
- 最初の周の調査基準は branch `feat/issues/1840` の `81ec380b`（`origin/main` と同一）である

# Outcome

対象者は、Releash で workflow を実行し、止まった実行木を操作する利用者と、Releash のコードを読み変更する開発者である。

現在、エンジンはメモリ上に実行木の一覧を持ち、それを操作の判定の正本として扱う。一方、画面・CLI・API は記録から計算した状態を表示する。さらに起動時に、その一覧をすべての実行木について作り直し、その過程で「プロセスを失った」事実を記録する。

この結果、次が起きている。利用者が Session を再開しても、エンジンのメモリ上の状態は古いままで、Submit と provider の Stop が黙って落ちる。起動時の登録が 1 件でも失敗すると処理全体が失敗し、上限なく繰り返されてログと CPU を消費し続ける。command の結果は、メモリ上の一覧に対象が無ければエラーも出さずに捨てられる。worktree の排他はメモリ上の登録簿が判定し、起動を拒否したときのエラーは、塞いでいる実行ではなく起動しようとした workflow の名前を示す。

変更後は、エンジンは操作のたびに、AgentSession が書いた事実を含む記録の最新の状態に基づいて判定する。起動時に行うのは、途切れた前進を続ける処理だけであり、失敗しても再試行せず、1 本の失敗が他の実行木へ波及しない。worktree の排他は起動する時点の記録から計算し、拒否のエラーは塞いでいる実行を示す。command の結果は、反映されないまま黙って捨てられない。

# Current Behavior

調査時点は 2026-09-22、branch `feat/issues/1840`、`81ec380b`。

## 起動時に行っていること

- 起動時の処理は `WorkflowStartupUsecase::execute`（`src-tauri/src/usecase/workflow/startup.rs:30-52`）である。全 tree_id を回し、メモリに登録済みまたは予約済みならスキップし、そうでなければ #1836 の `abort_unavailable_definition` を行ってから `reconcile_tree` を呼ぶ
- `reconcile_tree`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:621-687`）は `reconcile_tree_pass`（`src-tauri/src/adaptor/gateway/workflow/fact_log.rs:886-1035`）を呼ぶ。`reconcile_tree_pass` は次の 3 つを行う
  1. attach / spawn まで記録されていて実行中のままの leaf へ、`ProcessExited`（`failure_reason: "process lost across application restart"`）を追記する（`fact_log.rs:966-976`）
  2. 喪失を含めて再度 fold し、未実行の前進を適用して事実を追記する（`fact_log.rs:977-1030`）
  3. 起動すべき leaf を返す
- `reconcile_tree` は続けて、Workflow として起こされた実行木を `execution_store` の active 登録簿へ `register_active_execution` し、`self.executions`（`tree_id` をキーにした in-memory の HashMap）へ aggregate を insert し、返ってきた leaf を `start_nodes` する。`start_nodes` が失敗すると `executions` から取り除いて `Err` を返す
- `WorkflowStartupUsecase::execute` は実行木ごとの失敗を `first_error` へ記録し、最後に `Err` を返す
- daemon はこの処理を spawn し（`src-tauri/src/adaptor/controller/daemon.rs:312-331`）、`run_startup_recovery`（`daemon.rs:468-502`）が `Err` のたびに 50ms から倍々・最大 1 秒の間隔で上限なく再試行する。成功時は常に `Ok(0)` を返すため、空パス 2 回で `Quiescent` として終了する
- 起動時の処理は spawn されるだけで（`daemon.rs:312-331`）、画面・API の提供開始はその完了を待たない

## メモリ上の一覧が操作の正本になっている

- `WorkflowRuntimeHost::executions` の読み口は `load_control_plane_execution`（`workflow_host.rs:279-284`）だけで、`load_active_execution`（`src-tauri/src/adaptor/gateway/workflow/runtime_command_gateway.rs:153-161`）を経て control plane が使う
- 承認、Submit、Retry、Session の Resume、provider の Stop はこの経路で実行木を読む（`src-tauri/src/usecase/workflow/control_plane.rs:38-41` の port と各操作）。一覧に無ければ `Active node execution not found: <id>`、一覧にあっても対象 Node が active でなければ `active node execution '<id>' was not found`（`src-tauri/src/usecase/workflow/output_submission.rs:56`）になる
- AgentSession は `SessionAttached` / `ProcessExited` / `ResumeRequested` / `ArchiveRequested` などを事実ログへ直接書く（`agent_session_repository.rs:100-178`）。エンジンのメモリ上の一覧はこれによって更新されない
- 記録からメモリへ載せ直す `register_started_execution_tree`（`workflow_host.rs:314-362`）はあるが、呼ばれるのは AgentSession の新規作成（`src-tauri/src/usecase/agent_session/agent_session_launch.rs:629`）と Archive の解除（`src-tauri/src/usecase/workflow/execution_archive.rs:98`）であり、Session の再開経路には無い
- 最小の再現は、workflow の Session Node が動いている状態で daemon を再起動し、起動時に記録された `ProcessExited` の後に利用者が AgentSession の Resume で Session を再開し、その Node へ Submit することである。Resume は `ResumeRequested` を記録へ書くがメモリ上の aggregate は古いままで、Submit は拒否される。実測は 2026-09-17、PJT-2451、Node `d0dffbbc` で、`active node execution … was not found` が繰り返され、対応する `stop_received` も記録されなかった（旧 #1828）
- 旧 #1837 の実測は 2026-09-19〜20 で、同じフォルダに「実行中」が 3 件あり、WARN が 1 秒に 3 行出続け、10MB のログが約 2.8 時間で埋まり、daemon が無操作でも 1 コアの 2 割前後を使い続けた

## worktree の排他

- 起動は `reserve_workflow_execution`（`workflow_host.rs:495-529`）で `execution_store` へ登録し、`ExecutionStoreError::WorktreeAlreadyActive` を `WorkflowRuntimeError::AlreadyActive(workflow.name)` へ写す（`workflow_host.rs:522-523`）。この `workflow.name` は起動しようとした側の名前である
- `insert_workflow_execution`（`workflow_host.rs:564-618`）は `find_any_by_worktree`（`src-tauri/src/adaptor/gateway/workflow/workflow_host/execution_registry.rs:10-18`）でメモリ上の一覧から同じ worktree の Workflow 実行木を探し、`validate_start`（`src-tauri/src/usecase/workflow/runtime_start_guard.rs:14-25`）へ渡す
- 文言は `Workflow '<name>' is already running for this session`（`src-tauri/src/usecase/workflow/runtime_error.rs:41-43`）で、排他の単位が worktree であることを示していない
- 記録から同じ worktree の実行中の一覧を返す問い合わせは `SqliteWorkspaceQueryService::execution_records`（`src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs:50-99`）にある

## command の結果

- `commit_command_output`（`workflow_host.rs:1714-1741`）は、`executions` に実行木が無ければ `Ok(())` を返し、対象が current でなければ `Ok(())` を返す。どちらもエラーを出さない
- `fail_current_command_node`（`workflow_host.rs:1837-1873`）も同じ判定で `Ok(())` を返す
- `command_execution_still_current`（`workflow_host.rs:1706-1712`）も同じ一覧を読む

## 終了時に止める対象

- `shutdown_all_active_commands`（`workflow_host.rs:1922-1987`）は、実行中の command を載せている `node_processes.active_commands`（`src-tauri/src/adaptor/gateway/workflow/node_process.rs:15`）から対象を作る。経路は `DaemonShutdownGateway::stop_commands`（`src-tauri/src/adaptor/gateway/application_lifecycle.rs:66-72`）である
- `shutdown_active_commands_for_execution`（`workflow_host.rs:1904-1920`）は `active_command_executions` から対象を作る

# Scope / Non-goals

## 変更する対象

- 起動時に「プロセスを失った」事実を記録する処理
- 起動時に実行木をメモリ上の一覧と active 登録簿へ登録する処理
- 起動時の処理が失敗したときの再試行と、1 本の失敗が他の実行木へ及ぶ範囲
- エンジンの操作（承認、Submit、Retry、Session の Resume、provider の Stop）が可否と結果を判定するときに読む実行木の取得元
- command の起動、結果、失敗の記録が対象を判定するときに読む取得元と、対象でない場合の扱い
- worktree の排他の判定元、同一 worktree への同時起動要求の扱い、起動を拒否したときのエラーの内容と文言
- アプリケーション終了時に止める command の対象の作り方
- 上記に対応するテスト

## 変更しない対象

- 事実ログの記録先と形式
- AgentSession が事実ログへ直接書くこと、および AgentSession の `open` / `paused` / `archived` の lifecycle
- #1839 が入れた Node の起動失敗の自動再試行、プロセス在否の判定とその読み取り、Node の状態の値と導出規則
- #1836 が入れた実行木の完了の事実と終端状態の判定、および保存定義を解釈できない実行を起動時に理由付きで Abort すること
- #1826 が入れた実行木単位の Archive と、消えた worktree の片付け
- 実行中の実行木を記録から返す問い合わせの新設（既存の問い合わせを使う）
- 起動時の処理の完了を待ってから画面・API を提供すること。現状どおり、起動時の処理の完了を待たずに提供を開始する

# Requirements

- R-001: 起動時の処理は、プロセスを失ったことを表す事実を記録しない
- R-002: エンジンが操作（承認、Submit、Retry、Session の Resume、provider の Stop）の可否と結果を判定するとき、その判定は、AgentSession が書いた事実を含む記録の最新の状態に基づく。実行木がメモリ上の一覧に無いこと、またはメモリ上の一覧が記録より古いことを理由に、操作が拒否されない
- R-003: Session を再開した後、その Session Node への Submit と provider の Stop は受け付けられ、対応する事実が記録される
- R-004: 起動時に行う処理は、途切れた前進を続けることと、完了または Abort の事実が無く保存定義を現行コードで解釈できない実行を理由付きで Abort することである
- R-005: 起動時の前進が失敗した実行木は、失敗の理由を伴って Abort され、その実行木に対して前進は再試行されない
- R-006: 1 つの実行木で起動時の前進が失敗しても、他の実行木の前進は行われる
- R-007: 同一の worktree で実行中の workflow の実行木は 1 件までである。この判定は、起動する時点の記録から計算される
- R-008: 同一の worktree に対する起動要求が同時に複数行われた場合、成功するのは 1 件だけである
- R-009: 起動が worktree の排他によって拒否されたとき、返るエラーは、その worktree を塞いでいる実行を識別できる情報を含み、排他の単位が worktree であることを示す
- R-010: command の起動、結果、失敗は、記録の最新の状態に基づいて対象の実行木へ反映される。反映できない場合は、反映しなかったことと理由がログに記録される
- R-011: アプリケーションの終了時に止める command の対象は、実行中の command を保持している表から作られる。実行木がメモリ上の一覧に載っているかどうかは対象の決定に影響しない

# Assumptions / Open Questions

なし。
