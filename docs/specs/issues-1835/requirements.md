# Context

- 要求の正本: Issue #1835「[shutdown] 終了処理が途中で切れると、次回起動からアプリが使えなくなる」。
- 背景資料
  - Issue #1834: 2026-09-19 の障害の引き金。v0.4.13 の更新後の再起動で、終了処理の完了を待たずにプロセスが終了した。原因経路は v0.4.14 で除去済みであり、既に残った未完了の記録の復旧は #1835 が扱うと記載されている。
  - Issue #1640 / PR #1646: 2026-08-13 の同種障害と、その修正。修正は、残った記録を完了させる再試行の経路を追加するものだった。
  - `docs/specs/issues-1499/`: 終了手続きを store に記録し、未解決の間は新しい書き込みを受け付けない現行の仕組みを定めた closed Issue の spec。記録として扱い、改訂しない。本変更は同 spec の application quit と shutdown target の解決に関する要求を置き換える。
- 終了処理は daemon が行う。UI の終了操作（アプリケーションメニューの Quit、Cmd+Q、Dock、トレイの Quit、画面の Quit ボタン）、更新を伴わない再起動、更新の適用は、どれも UI から daemon へ終了要求を送り、同じ終了処理を起動する（`src-tauri/src/adaptor/gateway/daemon_supervision.rs:225-256`）。
- workflow が起動する command は、独立したセッション・プロセスグループで動く（`src-tauri/src/infrastructure/process/child_process.rs:8-22`）。止めずに daemon が落ちると、Releash の終了後も動き続ける。
- 起動時には既存の処理が、実行中のまま残った Session / Command Node のうちプロセスを起動済みだったものに、プロセスの喪失（`process lost across application restart`）を記録する（`src-tauri/src/adaptor/gateway/workflow/fact_log.rs:835-866`、`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:645-660`）。強制終了に備えて既に必要な処理であり、本変更は workflow の状態をこの処理に委ねる。
- 既知の限界（Issue #1835 に記載）: 終了処理が途中で打ち切られた場合、command が孤児プロセスとして残ることがある。現在クラッシュしたときと同じ挙動である。

# Outcome

- 対象者は Releash の利用者である。
- 現在、Releash は終了のたびに終了手続きの記録（止める対象の一覧と対象ごとの進捗）を store に書き、その記録が完了するまで Session の open・新規起動を含む書き込みを拒否する。終了処理の途中でプロセスが落ちると記録は未完了のまま残り、次回起動後も拒否が続く。再起動しても直らず、アプリの操作では復旧できない。画面には原因の分からない文言だけが出て、ログにも残らない。2026-08-13 と 2026-09-19 の 2 回発生し、どちらも store の直接操作で復旧した。
- 変更後は、終了時に command の停止とターミナルの状態の保存をその場で行い、プロセスを終了する。終了処理のどこで失敗しても、途中で打ち切られても、次回起動後の操作は妨げられない。既に未完了の記録が残っている store も、この変更を含む版でそのまま使える。

# Current Behavior

最初の周の開始時点（`feat/issues/1835`、`2774f470`）で、コードを読んで確認した挙動。アプリケーションの起動・テストの実行による確認は行っていない。障害時の実データは Issue #1834 / #1835 / #1640 の記録による。

## 終了処理の流れ

- UI は終了要求のたびに新しい要求 ID を生成し、保存しない（`src-tauri/src/adaptor/gateway/daemon_supervision.rs:227`）。
- daemon は終了要求の受理から準備 13 秒・決定 15 秒の期限を持つ（`src-tauri/src/usecase/shutdown_coordinator.rs:36-37`）。
- 受理時に、終了操作の記録、終了手続きの記録（`shutdown_plans`、phase `Prepared`）、現在の終了手続きを指す `store_metadata.current_shutdown_id`、実行中の WorkflowExecution ごとの対象（`shutdown_targets`）、recovery snapshot、終了要求の呼び出し記録（`caller_attempts`、kind `application_quit`）を一つの commit で保存する（`shutdown_coordinator.rs:3162-3404`）。
- 続いて phase を `Activated` にし、対象を一件ずつ、進捗 `EffectReserved` の保存 → command の停止 → `Completed` または `ReconciliationRequired` の保存、の順に処理する（`shutdown_coordinator.rs:3550-3692`）。
  - command の停止は、5 秒待ち、プロセスグループへ SIGTERM を送って 5 秒待ち、SIGKILL を送る（`src-tauri/src/infrastructure/process/child_process.rs:5-6`、`:24-34`）。workflow の事実は記録せず、`workflow_shutdown_effect_v1` の obligation 記録だけを残す（`src-tauri/src/adaptor/gateway/workflow/runtime_command_gateway.rs:446-520`）。
- 全対象が成功した場合だけ、provider exit observer の停止、ターミナルの停止と状態の保存、local API の停止を行う（`src-tauri/src/adaptor/controller/application_lifecycle.rs:133-142`、`:238-258`）。
  - ターミナルの停止は PTY の子プロセスへ SIGHUP を送り、出力が尽きるまで上限なく待つ。保存するのはエミュレータの画面と scrollback であり、`<data_dir>/terminal-surfaces/` 配下のファイルに書く（`src-tauri/src/usecase/terminal_surface/application.rs:558-593`）。
  - ターミナルの停止が失敗すると、local API の停止は行われない（`application_lifecycle.rs:139`）。
- 最後に `Completed`（`current_shutdown_id` を消す）または `ReconciliationRequired` を保存する。`Activated` に達した後は結果にかかわらず daemon は終了する（`shutdown_coordinator.rs:236-248`）。daemon は終了コードを受けると local API を止め（最大 5 秒）、`releash-shutdown-complete` を出力して終了する（`src-tauri/src/adaptor/controller/daemon.rs:16-24`）。
- UI は Quit の場合だけ、終了要求から 15 秒で daemon を強制終了する（`src-tauri/src/domain/daemon_supervision.rs:2`、`:436-443`）。強制終了では daemon が子孫プロセスをすべて止めて終了し、ターミナルの状態は保存されない（`src-tauri/src/infrastructure/process/parent_lifetime.rs:10-31`）。再起動と更新には UI 側の期限がない。

## 終了処理が途中で切れた後

最小の再現: 終了処理の `Activated` 以降、`Completed` の保存前に daemon のプロセスが終了する（Issue #1834 の実例では、対象 16 件のうち 0〜9 が `completed`、10 が `effect_reserved`、11〜15 が `prepared` の時点で終了した）。次回起動後の結果は次のとおり。

- `store_metadata.current_shutdown_id` が未完了の終了手続きを指したまま残る。書き込みの受付判定は、この終了手続きの phase が終端でない限り、Session の操作を含む利用者の書き込みと workflow・projection の書き込みの大半を拒否する（`src-tauri/src/adaptor/gateway/local_event_store/commit.rs:663-777`、`src-tauri/src/domain/local_event/commit_admission.rs:17-33`）。判定は「いま終了中」と「前回の記録が残っているだけ」を区別しない。拒否はログに出ない。
- Session の open・再開・復元・新規作成は、`Releash could not access saved AgentSession data. Try again.`（`AGENT_SESSION_STORAGE_UNAVAILABLE`）で失敗する（`src-tauri/src/adaptor/controller/client/agent_session/provider_tui.rs:198-200`）。
- 画面上部に終了のバナーが出る（`src/components/layout/ApplicationShutdownBanner.tsx`）。表示は `Application shutdown: {phase}` と `Retry quit` ボタン、`Application shutdown outcome unknown: {operation_id} — {exit|restart} ({code})`、`Shutdown target {kind} requires reconciliation` と `Retry same effect` ボタンである。バナーは 2 秒ごとに未解決の終了要求の呼び出し記録と現在の終了手続きを読み、localStorage（`releash:application-quit-attempt:v1`）に保存された要求を再送する（`src/hooks/useApplicationShutdownSupervision.ts`）。
- 終了・再起動・更新を要求すると、daemon は前回の終了手続きの解決が必要だと応答し、UI は `Previous shutdown requires a decision before switching.` を表示する（`src-tauri/src/adaptor/gateway/daemon_supervision.rs:340`、`src-tauri/src/usecase/shutdown_coordinator.rs:3024-3026`）。Issue #1835 は「起動時の分岐」と記すが、この表示は起動時ではなく終了要求への応答で出る。Quit は 15 秒後に UI が daemon を強制終了する。再起動・更新は UI 側に期限がなく、切替が進まない（コード読解、実機未確認）。
- 記録を完了させる手段は 3 つあるが、前回起動分の記録にはどれも通らない（Issue #1835）。
  1. 同じ終了要求の再送: 要求 ID が保存されていない。
  2. 新しい終了要求での続行: 前回の期限が切れているため、何も進まず何も保存されない。
  3. 対象ごとの `Retry same effect`: ボタンが画面に出ない。API から直接実行しても未着手だった対象は必ず失敗し、1 回ごとに daemon が落ちる。
- 2 回の障害はいずれも、`store_metadata.current_shutdown_id` を NULL にし、未解決の終了要求の呼び出し記録を `cleared` にする store の直接操作で復旧した。

# Scope / Non-goals

Scope は次のとおり。

- daemon の終了処理（Quit、更新を伴わない再起動、更新の適用のいずれによる終了も含む）の内容と時間の上限
- 終了手続きの記録、対象ごとの進捗、終了要求の呼び出し記録の廃止
- 前回の終了の記録を理由とする書き込みの拒否の廃止
- 終了処理に関する画面（`Retry same effect`、`Retry quit`、outcome unknown の表示）と、前回の終了の解決を求めて終了・再起動・更新を止める分岐（`Previous shutdown requires a decision before switching.`）の廃止

Non-goals は次のとおり。

- 終了処理が途中で打ち切られた場合に command が孤児プロセスとして残ること（既知の限界）
- 未完了の記録が残った store を使えるようにするための起動時の処理（拒否の判定を無くすことで使えるようになるため、追加しない）
- 終了処理の途中でプロセスが終了した引き金の経路（Issue #1834。v0.4.14 で除去済み）
- `docs/specs/issues-1499/` の改訂

# Requirements

- R-001: 終了処理が行うのは、workflow が起動した command の停止、ターミナルの状態の保存、プロセスの終了だけである。終了処理のためにターミナルを止めたことを、AgentSession のプロセス終了として記録しない。
- R-002: 終了処理のどの段階が失敗しても、失敗をログに出力して次の段階へ進み、プロセスを終了する。
- R-003: 終了処理の全体に 15 秒の時間の上限を設け、上限を超えた場合は残りの処理を打ち切ってプロセスを終了する。
- R-004: 終了処理は、終了手続きの記録、止める対象ごとの進捗、終了要求の呼び出し記録を保存しない。command の停止とターミナルの状態の保存は終了処理の中で行い、次回起動へ持ち越さない。
- R-005: 前回の終了処理がどの段階で途切れていても、次回起動後の Session の open・新規起動を含む書き込み操作は、前回の終了を理由に拒否されない。この変更より前の版で未完了の終了手続きの記録が残った store も、この変更を含む版で利用者の操作や store の直接操作なしに同じく使える。
- R-006: 終了処理に関する画面（`Retry same effect`、`Retry quit`、outcome unknown の表示）を持たない。終了・再起動・更新の要求は、前回の終了の解決を求めて止まらない（`Previous shutdown requires a decision before switching.` を表示しない）。
- R-007: 終了時に実行中だった workflow の状態は、終了処理では記録しない。次回起動時に、既存の起動時の処理がプロセスの喪失として記録する。

# Assumptions / Open Questions

- 自動判断: R-001 の「プロセスの終了」には、プロセスを終了するときの local API の停止（`src-tauri/src/adaptor/controller/daemon.rs:16-24`）を含める。入力文書は local API の停止を残すかを定めていないため、既存の挙動を維持する最小の解釈をとった。
- 自動判断: R-006 の outcome unknown の表示は、Current Behavior に記録した終了のバナーの `Application shutdown outcome unknown: …` を指す。UI の daemon 監督が、再起動・更新の要求の後に daemon が終了完了（`releash-shutdown-complete`）を出力せずに終了したときに出す `Daemon shutdown outcome is unknown; switching is blocked.`（`src-tauri/src/domain/daemon_supervision.rs:280-283`、`:378-388`）は含めない。入力文書が挙げるのは前者だけであるため、既存の挙動を維持する最小の解釈をとった。
