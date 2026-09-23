# Design 10

## 開始状態

- 差分の基準は base ブランチ `main`、派生点 `81ec380b`。branch `feat/issues/1840` には commit `c0c7c985`（Design 01〜06 の実装と spec 8 ファイル）と `8aa63a4e`（承認待ち Command を記録から承認待ちとして読み戻す）が積まれ、さらに Design 07・08・09 の実装が未コミットの作業ツリー差分（21 ファイル、+929 / -317）として存在する。この 2 commit と作業ツリー差分をこの周の開始状態として扱う。
- 直前の Design は `docs/specs/issues-1840/design-09.md`。その「変える部分」5 項目はいずれも実装済みである。control-plane commit の Conflict 再試行は `retry_runtime_conflicts`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:255`）へ集約され、`workflow_host.rs:1207`・`:1636`・`:1768`、`isolated_worktree.rs:155`、`command_preparation.rs:80`、`lifecycle_commands.rs:66` の 6 経路が呼ぶ。`start_prepared_composite` の Succeeded 再入分岐には `worktree_test.rs:559`・`:569`・`:608`・`:621`・`:634` のテストが付いた。`commit_required_events`（`workflow_host.rs:1921`）は `RuntimeCommitSnapshot` を返す。`WorkflowStartupRepository`（`src-tauri/src/domain/workflow/repository.rs:118-128`）は `WorkflowRevision` を使い raw な行 seq を宣言しない。`OutcomeUnknown` 後の永続化判定は `resolve_unknown_append`（`src-tauri/src/adaptor/gateway/workflow/fact_log.rs:59`）へ集約され、`fact_log.rs:1099`、`workflow_host.rs:353`、`startup_repository.rs:138` の 3 経路が共有する。
- この周までに解消・見送りとなった Thread: resolve 済み 23 件（Design 09 の周までの 14 件に加え、Design 09 の対象 `aa600784-1eaf-4f9f-937c-8b565d8de377`、`2469600b-1262-4c9c-89a3-8fbd16e391e3`、`1f2cd7fe-2857-4797-866d-84afd198fbfb`、`7111e43d-9043-428b-a11e-c7637597306d`、`5591688b-8c7f-4b0e-a178-dfd49a401733` を含む）。`[REJECTED]`・`[DEFERRED]` とした Thread は無い。
- この周の対象は `[FIX_POLICY]` が付いた open Thread 1 件（`cc4b65b0-860f-43ee-942c-41f4e79b2681`）である。
- Requirements・Behavior はこの周で変更していない。R-001〜R-011 と B-001〜B-014、対応表は Design 09 の周と同じである。

## 変える部分

- 自動起動再試行の restart 経路に control-plane commit の Conflict の有界再試行を入れ、1 件の Conflict が同一バッチの後続 Node を打ち切らないようにする: `restart_node_attempt`（`src-tauri/src/adaptor/gateway/workflow/workflow_host/node_startup.rs:211-282`）は `runtime_activation_gate` の下で最新の実行木を読んで candidate を作るが、`commit_control_plane_candidate`（同 `:271`）を `retry_runtime_conflicts` で包まず `?` で伝播する。`commit_workflow_control_plane`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:388-405`）は `NodeRetryRequested` を含む commit のときだけ `runtime_activation_gate` を取るため、通常の commit と AgentSession の直書き（`src-tauri/src/adaptor/gateway/agent_session/agent_session_repository.rs:100-178`）は gate の外で同一実行木へ追記でき、Conflict は実在する。その Err は `retry_failed_nodes`（`src-tauri/src/usecase/workflow/node_startup.rs:28-32`）が即時伝播するため同じバッチの後続 Node は処理されず、`schedule_startup_retries`（`node_startup.rs:56-118`）で warn と `settle_runtime_failure` へ渡るだけで、`settle_runtime_failure_for_node`（`workflow_host.rs:2001-2009`）が Conflict をそのまま返すため失敗も記録されず当該実行木の自動再起動タスクが終了する。変更後は、Conflict のとき最新の記録から再評価して有界に再試行し、再評価の結果その Node が既に再起動不要であれば正常終了として扱い、再起動可能な Node と同一バッチの後続の失敗 Node の処理を継続する。根拠: Thread `cc4b65b0-860f-43ee-942c-41f4e79b2681`。`requirements.md`「Scope / Non-goals」の「変更しない対象」にある「#1839 が入れた Node の起動失敗の自動再試行」。`docs/architecture/README.md:32`「同じ操作の実装は 1 つに集約する」に対し、Design 09 で集約した control-plane commit の Conflict 再試行がこの経路だけ欠けている。#1839 の自動再試行の回数と間隔の扱いは変えない。B-001〜B-014 の観測結果は変わらない。ルート: 委任

## 固定するルート

- この周に人間が新しく固定した実装上の指定は無い。上の 1 件はルートが委任である。restart 経路の Conflict の有界再試行をどこへ置くか（`restart_node_attempt` の commit を `retry_runtime_conflicts`（`workflow_host.rs:255`）で包むか、`gateway.restart` の呼び出し側で再評価を繰り返すか）、再評価で再起動不要と判明した場合をどう正常終了として表すか、`retry_failed_nodes`（`src-tauri/src/usecase/workflow/node_startup.rs:15-40`）が 1 件の Err で打ち切らず同一バッチの後続 Node を処理し続けるようにする形は、いずれも実装側で決める。
- Design 01 で固定した D1〜D8 を維持する。

## 変えないもの

- `settle_runtime_failure_for_node`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:2001-2009`）が Conflict を対象 Node の失敗として記録せずそのまま返す扱いを維持する。Design 07 で固定し Design 09 の「変えないもの」で維持した判断であり、理由は、競合は Node の実行が失敗したことではなく記録が先に進んだことを表すためである。上の変更はこの扱いを前提に、Conflict を再試行で吸収する側で解決する。

## 未確定・リスク

- 自動判断: なし。R-001〜R-011 と B-001〜B-014 は対応表に欠落なく相互に対応しており、誤り・不足・矛盾を見つけなかったため、Requirements・Behavior は変更していない。未決のまま残した要求は無く、`[DEFERRED]` で人間へ渡した件も無い。
