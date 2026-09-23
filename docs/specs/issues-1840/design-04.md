# Design 04

## 開始状態

- 差分の基準は base ブランチ `main`、派生点 `81ec380b`。branch `feat/issues/1840` に commit は無く、Design 01〜Design 03 の実装は未コミットの作業ツリー差分（`src-tauri/` で 54 ファイル、+3126 / -3314）として存在する。
- 直前の Design は `docs/specs/issues-1840/design-03.md`。その「変える部分」1 項目は実装済みであり、その実装をこの周の開始状態として扱う。確認した範囲は次のとおり。
  - `reconcile_tree_pass`（`src-tauri/src/adaptor/gateway/workflow/fact_log.rs:1007-1060`）は `append_node_events_at_head_blocking` の `NodeEventWriteError::OutcomeUnknown` を受けて `node_events::read_tree_page(tree_id, offset=head, limit=rows.len())` で読み直す。全行一致なら保存済みとして seq 群を返して前進を続け、0 件なら `OutcomeUnknown` のまま前進失敗とし、部分一致なら `Conflict` として `WorkflowError::Conflict` へ写す。
- この周までに解消・見送りとなった Thread: Design 02 の対象 6 件（`2b34c2d8`、`ca6a9a6f`、`628753e0`、`ac454646`、`1a575f53`、`d3d2fbc2`）は resolve 済みである。Design 03 の対象 `2e74c849-b789-44c6-884a-e96ddd4899e9` は `[STILL_OPEN]` を経て対象範囲が変わり、open のまま今周の対象である。
- この周の対象は `[FIX_POLICY]` が付いた open Thread 2 件（`2e74c849-b789-44c6-884a-e96ddd4899e9`、`e6193ac3-a9e2-4aa5-a54d-090f68e6e2e5`）である。
- Requirements・Behavior はこの周で変更していない。R-001〜R-011 と B-001〜B-014、対応表は Design 03 の周と同じである。

## 変える部分

- 起動時の前進が失敗した実行木への Abort を、追記時点の記録の状態に基づいて決める: `abort_startup_failure`（`src-tauri/src/usecase/workflow/startup.rs:71-84`）は `repository.load` で読んだ snapshot に対し `abort_with_reason` の `is_active` 判定（`src-tauri/src/domain/workflow/entities/workflow_execution/terminal_replay.rs:47-50`）だけで `AbortRequested` を作り、`StoredWorkflowStartupRepository::append`（`src-tauri/src/adaptor/gateway/workflow/startup_repository.rs:104-113`）経由の `fact_log::append_single_fact`（`fact_log.rs:846-852`）は expected head を渡さない。requirements.md の「変更しない対象」が起動時処理の完了を待たずに画面・API の提供を開始すると定めるため、load と append の間に承認・Stop 等が記録を前進させる窓が実在し、その最新状態を評価しないまま古い snapshot から Abort を追記できる。根拠: Thread `2e74c849-b789-44c6-884a-e96ddd4899e9`。R-005（理由付き Abort の対象は前進が失敗した実行木であり、その実行木に対して前進は再試行されない）、R-002（判定は記録の最新の状態に基づく）、B-006。修正対象はこの load から append までの窓に限り、`fact_log.rs:1007-1060` の `OutcomeUnknown` 読み直しは変更対象に含めない。B-005 の途切れた前進の継続と B-007 の他の実行木への非波及は維持する。ルート: 委任
- 起動と Abort の直列化の範囲を同一 worktree に戻す: `workflow_start_lock` は `src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:103`・`:444` の単一 `Arc<Mutex<()>>` である。`start_workflow` は `workflow_host.rs:649` で取得し、`execution_summaries` による候補取得・`validate_start`・`insert_workflow_execution`・`ExecutionStarted` を含む required batch の append を経て `:708` の `drop(start_guard)` まで解放しない。`abort_workflow_execution`（`src-tauri/src/adaptor/gateway/workflow/workflow_host/lifecycle_commands.rs:40`）は同じ lock を取得し、`:44-48` の cancellation acknowledgement 待機、`:51-62` の commit 有界再試行、`:70-74` の `cancel_startup_retries`・`shutdown_active_commands_for_execution`・`finalize_after_commit` を含む関数末尾 `:86` まで保持するため、別 worktree の起動が Abort の完了を待たされる。変更前（`81ec380b`）の `execution_store.rs:251-301` `reserve_active_interruption` は `worktree_path` をキーに予約するだけで別 worktree の起動を妨げなかった。根拠: Thread `e6193ac3-a9e2-4aa5-a54d-090f68e6e2e5`。R-007（同一の worktree で実行中の workflow の実行木は 1 件まで）、R-008（同一の worktree に対する同時起動要求で成功するのは 1 件だけ）、B-008、B-009。同一 worktree に対する Abort と起動、および同一 worktree への同時起動要求は直列化を維持し、別 worktree に対する起動は進行中の Abort の完了を待たずに記録へ進める。ルート: 委任

## 固定するルート

- この周に人間が新しく固定した実装上の指定は無い。
- Design 01 で固定した D1〜D8 を維持する。今周の「変える部分」に直接かかるのは D1（エンジンの作業用の実行木は必要になった時点で記録から読み込み、メモリ上の一覧を正本にしない）と D3（同一 worktree への同時起動要求は起動処理の直列化で防ぐ。直列化の単位と実装箇所は委任）である。
- `abort_startup_failure` が追記時点の記録の状態に基づいて Abort を決める方法（append に head 条件を付けるか、load と append を有界に再試行するか、判定を startup usecase 側と repository 側のどちらに置くか）は委任である。
- 直列化を同一 worktree へ戻す方法（worktree 単位の lock 集合を持つか、lock の保持範囲を commit までに縮めるか、既存の予約機構へ戻すか）は委任である。

## 変えないもの

なし。

## 未確定・リスク

- 自動判断: なし。Requirements と Behavior の対応に誤り・不足・矛盾を見つけなかったため、どちらも編集していない。未決のまま残した要求は無く、`[DEFERRED]` で人間へ渡した件も無い。
