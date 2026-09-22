# Design 06

## 開始状態

- 差分の基準は base ブランチ `main`、派生点 `81ec380b`。branch `feat/issues/1840` に commit は無く、Design 01〜Design 05 の実装は未コミットの作業ツリー差分（`src-tauri/` で 56 ファイル、+3808 / -3328）として存在する。
- 直前の Design は `docs/specs/issues-1840/design-05.md`。その「変える部分」1 項目は実装済みであり、その実装をこの周の開始状態として扱う。`WorkflowRuntimeHost::append_events_at_head`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:320-384`）は `NodeEventWriteError::OutcomeUnknown` を受けたとき `node_events::read_tree_page` で head 以降を読み戻し、全一致なら成功、空なら `OutcomeUnknown`、部分一致なら `Conflict` として扱う。
- この周までに解消・見送りとなった Thread: Design 02 の対象 6 件（`2b34c2d8`、`ca6a9a6f`、`628753e0`、`ac454646`、`1a575f53`、`d3d2fbc2`）、Design 04 の対象 `e6193ac3-a9e2-4aa5-a54d-090f68e6e2e5`、Design 03・04・05 の対象だった `2e74c849-b789-44c6-884a-e96ddd4899e9` は、いずれも resolve 済みである。`[REJECTED]`・`[DEFERRED]` とした Thread は無い。
- この周の対象は `[FIX_POLICY]` が付いた open Thread 1 件（`2d4f0269-e8c9-41c1-bb42-b8e6fe12c88e`）だけである。
- Requirements・Behavior はこの周で変更していない。R-001〜R-011 と B-001〜B-014、対応表は Design 05 の周と同じである。

## 変える部分

- 起動時の Abort 追記が `OutcomeUnknown` を返したとき、その Abort の事実の保存有無を最新の記録で判定する: `StoredWorkflowStartupRepository::append`（`src-tauri/src/adaptor/gateway/workflow/startup_repository.rs:117-139`）は `NodeEventWriteError::Conflict` だけを `WorkflowError::Conflict` へ写し、`OutcomeUnknown` を含むその他を catch-all で `WorkflowError::external` へ写す（`:132-138`）。`OutcomeUnknown` は「admission 失敗または reply 喪失で durable かどうか不明であり、呼び出し側はログから再導出して冪等に再試行する」状態である（`src-tauri/src/adaptor/gateway/local_event_store/writer.rs:65-75`）。呼び出し元の `abort_startup_failure`（`src-tauri/src/usecase/workflow/startup.rs:75-88`）は `retry_control_plane_conflicts` で包まれるが、この helper は `WorkflowError::Conflict` だけを再試行する（`src-tauri/src/usecase/workflow/command/mod.rs:28-33`）。さらに `WorkflowStartupUsecase::execute` は `recovery_lock` による一回限りの guard を先に立てるため（`startup.rs:30-34`）、同一プロセスで再実行されない。結果、`AbortRequested` が未保存のまま失敗扱いになると実行木は active のまま残り、保存済みで reply だけ失われた場合は成功した Abort を失敗として記録する。同じ `append` は `abort_unavailable_definition`（`startup.rs:102`）も通る。根拠: Thread `2d4f0269-e8c9-41c1-bb42-b8e6fe12c88e`。R-005（起動時の前進が失敗した実行木は理由を伴って Abort される）、B-006、および同一経路を通る R-004（完了または Abort の事実が無く保存定義を解釈できない実行を理由付きで Abort する）、B-013。ルート: 委任

## 固定するルート

- この周に人間が新しく固定した実装上の指定は無い。
- Design 01 で固定した D1〜D8 を維持する。
- `OutcomeUnknown` の保存有無をどの読戻しで判定するか、判定を `StoredWorkflowStartupRepository::append` の内側に置くか呼び出し元の usecase 側に置くか、既存の `src-tauri/src/adaptor/gateway/workflow/fact_log.rs:1013` および `src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:344` の照合と共通化するかは委任である。

## 変えないもの

なし。

## 未確定・リスク

- 自動判断: なし。Requirements と Behavior の対応に誤り・不足・矛盾を見つけなかったため、どちらも編集していない。未決のまま残した要求は無く、`[DEFERRED]` で人間へ渡した件も無い。
