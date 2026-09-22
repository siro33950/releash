# Design 02

## 開始状態

- 差分の基準は base ブランチ `main`、派生点 `81ec380b`。branch `feat/issues/1840` に commit は無く、Design 01 の実装は未コミットの作業ツリー差分（`src-tauri/` を中心に 52 ファイル、+1821 / -3130）として存在する。
- 直前の Design は `docs/specs/issues-1840/design-01.md`。その「変える部分」は実装済みであり、その実装をこの周の開始状態として扱う。確認した範囲は次のとおり。
  - 起動時の `ProcessExited { failure_reason: "process lost across application restart" }` は `src-tauri/` に存在しない（R-001 / B-001）。
  - `execution_store` と `execution_registry`（メモリ上の active 登録簿）は削除され、`execution_store` / `ExecutionStoreError` / `find_any_by_worktree` / `register_active_execution` への参照は残っていない。
  - `run_startup_recovery` による起動時処理の再試行経路は `src-tauri/src/adaptor/controller/daemon.rs` から無くなり、`WorkflowStartupUsecase::execute`（`src-tauri/src/usecase/workflow/startup.rs:29-56`）は `recovery_lock` で 1 回だけ実行し、実行木ごとの失敗を `abort_startup_failure` で Abort してループを継続する。
  - `WorkflowRuntimeError::AlreadyActive` は `WorktreeActiveExecution`（`worktree_path` / `workflow_name` / `execution_id`）を持ち、文言は `Worktree '<path>' already has running workflow '<name>' (execution '<id>')` である（`src-tauri/src/usecase/workflow/runtime_error.rs:16,41-45`）。
  - worktree 排他の判定元は `WorkspaceQueryService::execution_summaries` であり、起動処理は `workflow_start_lock` で直列化されている（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:642-668`）。
- この周までに解消・見送りとなった Thread: なし。resolve された Thread は無く、`[FIX_POLICY]` が付いた open Thread 6 件がこの周の対象である。
- Requirements・Behavior はこの周で変更していない。R-001〜R-011 と B-001〜B-014 は Design 01 の周と同じである。
- Design 01 の「未確定・リスク」の扱い。`execution_store` の active 登録簿が残るかは、登録簿ごと削除されたことで解消した。control plane commit の競合検出は `commit_control_plane_candidate`（読取時の head を条件とする追記）として実装され、その競合時の扱いが下記 Thread 3 件（`1a575f53`、`ac454646`、`d3d2fbc2`）の対象である。

## 変える部分

- 本番呼び出し元を失った `PreparedWorkflowTransaction::persist_async` と専用テスト 3 件を削除する: commit 経路が同期 `persist` へ集約された結果、`persist_async`（`src-tauri/src/usecase/workflow/runtime_driver.rs:168`）の参照は定義と `:393` / `:419` / `:450` のテストだけになった。根拠: Thread `2b34c2d8-2b5f-4be5-be1f-246025bcb426`。R-002 の変更に付随するテストとして、requirements.md Scope「上記に対応するテスト」に含まれる。stale candidate の拒否・永続化失敗時に current を更新しないこと・永続化成功後だけ effect を解放することは、同期 `persist` の既存テストで検証されている必要がある。ルート: 委任
- `start_workflow` の `workspace_query` 未注入時の具象構築フォールバックを無くす: `workflow_host.rs:642-657` が `SqliteWorkspaceQueryService` / `SqliteWorkspaceTreeRepository` / `ExecutionTreeArchiveFactRepository` を gateway 内で組み立て、`wiring.rs` の配線を複製している。根拠: Thread `ca6a9a6f-de26-463b-bacf-b3de9b09e7c0`。規約 `docs/architecture/README.md:28`・`docs/architecture/CONTROLLER.md:19`（DI 配線は controller の責務であり、gateway へ漏らさない）。R-007、B-008。ルート: Design 01 の D4 を維持し、判定元は既存の `SqliteWorkspaceQueryService::execution_records` のままとする。注入方法は委任
- 記録が先に進んだ状態からの承認と Retry の受理を検証する: 外部の writer が同一実行木の記録を先へ進めた後の受理を確認しているのは `workflow_host_test.rs:1433` の Submit と provider Stop だけで、B-002 が明示する承認と Retry の経路に同じ検証が無い。根拠: Thread `628753e0-ed25-4e3f-bdcf-a6259237fce4`。B-002、R-002、requirements.md Scope「上記に対応するテスト」。ルート: 委任
- Abort の control-plane commit の Conflict を有界に再試行する: `abort_workflow_execution`（`src-tauri/src/adaptor/gateway/workflow/workflow_host/lifecycle_commands.rs:51-64`）は Conflict 後に読み直すが、最新記録で active なら元の Conflict を返すだけで再試行しない。承認・Submit・Retry・Session の Resume・provider Stop は `retry_control_plane_conflicts`（`src-tauri/src/usecase/workflow/control_plane.rs:133` / `:239` / `:390` / `:443` / `:574`）を通る。根拠: Thread `ac454646-d802-4661-a2d1-a3c36dc59620`。判定元を記録へ移した結果 Abort に新しい競合窓が生まれ、変更前に成立していた Abort の成功が失われている。requirements.md Scope「変更する対象」は Abort の成功可否を含まないため、変更前の挙動を保つ位置づけであり、Abort を新しい要求として Requirements へ足してはいない。ルート: 委任
- 起動時の前進の head 競合を、実際の前進失敗と区別する: `reconcile_tree_pass`（`src-tauri/src/adaptor/gateway/workflow/fact_log.rs:1005-1011`）は読取時の head を条件に追記し、競合を `startup advancement commit failed` の `Err` にする。`WorkflowStartupUsecase::execute`（`startup.rs:41-55`）はこれを区別せず `abort_startup_failure` へ渡し、同関数は最新記録を再評価せず head 条件なしで Abort を追記する。根拠: Thread `1a575f53-9e5d-4a04-9bc9-8b91b16c8e3c`。R-004（起動時に行う処理は、途切れた前進を続けることと、保存定義を解釈できない実行の Abort に限る）、R-005（Abort の対象は前進が失敗した実行木）、R-002、B-005、B-006、B-007。ルート: 委任
- command の結果の head 競合時に、最新記録で反映可否を再判定する: `observe_command_completion`（`workflow_host.rs:1499-1514`）は `commit_command_output` の全 `Err` を `fail_current_command_node` へ渡し、同関数（`:1687-1730`）は読み直した対象が current なら `NodeFailed`（`InfrastructureCrash`）を追記するため、head 競合のときに成功した結果が障害へ置き換わる。根拠: Thread `d3d2fbc2-0e7f-4c50-8d68-88f7037b802b`。R-010、B-011、B-014。ルート: Design 01 の D8 を維持し、反映可否の判定は domain の規則 1 つのままとする。競合時の再判定の実現方法は委任

## 固定するルート

- この周に人間が新しく固定した実装上の指定は無い。
- Design 01 で固定した D1〜D8 を維持する。今周の「変える部分」に直接かかるのは D4（`start_workflow` の項）と D8（command の結果の項）であり、どちらも判定元と層の配置は Design 01 のままで、注入方法と競合時の再判定の実現方法だけが委任である。

## 変えないもの

なし。

## 未確定・リスク

- 自動判断: なし。Requirements と Behavior の対応に誤り・不足・矛盾を見つけなかったため、どちらも編集していない。未決のまま残した要求は無く、`[DEFERRED]` で人間へ渡した件も無い。
- R-010 は command の「起動、結果、失敗」を対象とするが、今周の Thread `d3d2fbc2` が指すのは結果の反映だけである。起動の記録 `commit_command_spawned`（`src-tauri/src/adaptor/gateway/workflow/workflow_host/command_preparation.rs:72-110`）も同じ head 競合で `Err` になり、`spawn_command_execution` の呼び出し元（`workflow_host.rs:1429-1441`、`:1302-1310`）は command process を止めて `settle_runtime_failure_for_node` へ渡す。結果側だけを直した場合に R-010 の「起動」が満たされるかは、この周の開始時点で未確定である。
