# Design 03

## 開始状態

- 差分の基準は base ブランチ `main`、派生点 `81ec380b`。branch `feat/issues/1840` に commit は無く、Design 01・Design 02 の実装は未コミットの作業ツリー差分として存在する。
- 直前の Design は `docs/specs/issues-1840/design-02.md`。その「変える部分」6 項目は実装済みであり、その実装をこの周の開始状態として扱う。確認した範囲は次のとおり。
  - `persist_async` への参照は `src-tauri/src/` に残っていない。
  - `start_workflow` は注入済みの `workspace_query` を使い（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:121`、`:414-424`、`:438-453`、`:651`）、gateway 内で具象を組み立てるフォールバックは無い。
  - 記録が先に進んだ状態からの承認と Retry の受理は `src-tauri/src/adaptor/gateway/workflow/workflow_host_test.rs:1859` と `:1918` で検証されている。
  - `abort_workflow_execution`（`src-tauri/src/adaptor/gateway/workflow/workflow_host/lifecycle_commands.rs:50-62`）は control-plane commit の Conflict を `CONTROL_PLANE_MAX_ATTEMPTS` まで再試行する。
  - `WorkflowStartupUsecase::execute`（`src-tauri/src/usecase/workflow/startup.rs:48-53`）は `WorkflowError::Conflict` を warn + continue として前進失敗から分離する。
  - `finish_command_execution`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:1490-1501`）は `commit_command_output` の Conflict を warn + return とし、`settle_runtime_failure_for_node` へ渡さない。
- この周までに解消・見送りとなった Thread: Design 02 の対象 6 件（`2b34c2d8`、`ca6a9a6f`、`628753e0`、`ac454646`、`1a575f53`、`d3d2fbc2`）はいずれも resolve 済みである。この周の対象は `[FIX_POLICY]` が付いた open Thread 1 件（`2e74c849-b789-44c6-884a-e96ddd4899e9`）だけである。
- Requirements・Behavior はこの周で変更していない。R-001〜R-011 と B-001〜B-014、対応表は Design 02 の周と同じである。
- Design 02 の「未確定・リスク」に挙げた R-010 の「起動」側は、開始状態で解消している。`spawn_command_execution` の commit（`src-tauri/src/adaptor/gateway/workflow/workflow_host/command_preparation.rs:111-122`）は Conflict を有界に再試行し、尽きたら反映しなかったことと対象を warn に残して `Ok(false)` を返す。

## 変える部分

- 起動時の前進 commit が `OutcomeUnknown` を返したとき、前進事実の保存有無を最新の記録で判定し、保存済みの実行木を Abort しない: `reconcile_tree_pass`（`src-tauri/src/adaptor/gateway/workflow/fact_log.rs:1008-1021`）は `append_node_events_at_head_blocking` の `NodeEventWriteError::Conflict` だけを `WorkflowError::Conflict` へ写し、`OutcomeUnknown` は `startup advancement commit failed` の `WorkflowError::external` になる。`OutcomeUnknown` は「書込みが durable か不明で、ログから再導出して冪等に再試行する」状態であり（`src-tauri/src/adaptor/gateway/local_event_store/writer.rs:71-74`）、writer が処理済みで行が保存されていても reply 喪失で返る（`src-tauri/src/adaptor/gateway/local_event_store/node_events_test.rs:367-388`）。`WorkflowStartupUsecase::execute`（`startup.rs:48-65`）は Conflict 以外を前進失敗として `abort_startup_failure` へ渡し、同関数（`startup.rs:77-83`）は読み直した実行木に対し `abort_with_reason` の `is_active` 判定（`src-tauri/src/domain/workflow/entities/workflow_execution/terminal_replay.rs:47-50`）だけで `AbortRequested` を追記するため、保存済みの前進に Abort が重なる。根拠: Thread `2e74c849-b789-44c6-884a-e96ddd4899e9`。R-005（理由付き Abort の対象は前進が失敗した実行木）、R-004（起動時に行う処理の範囲）、R-002（判定は記録の最新の状態に基づく）、B-006。B-005 の途切れた前進の継続と B-007 の他の実行木への非波及は維持する。ルート: 委任

## 固定するルート

- この周に人間が新しく固定した実装上の指定は無い。
- Design 01 で固定した D1〜D8 を維持する。今周の「変える部分」に直接かかるのは D1（エンジンの作業用の実行木は必要になった時点で記録から読み込み、メモリ上の一覧を正本にしない）であり、`OutcomeUnknown` 後に前進事実の保存有無を再導出する位置（`fact_log` 側か startup usecase 側か）、判定の方法、Abort 追記時に head 条件を付けるかは委任である。

## 変えないもの

なし。

## 未確定・リスク

- 自動判断: なし。Requirements と Behavior の対応に誤り・不足・矛盾を見つけなかったため、どちらも編集していない。未決のまま残した要求は無く、`[DEFERRED]` で人間へ渡した件も無い。
