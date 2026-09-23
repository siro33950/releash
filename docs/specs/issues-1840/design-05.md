# Design 05

## 開始状態

- 差分の基準は base ブランチ `main`、派生点 `81ec380b`。branch `feat/issues/1840` に commit は無く、Design 01〜Design 04 の実装は未コミットの作業ツリー差分（`src-tauri/` で 56 ファイル、+3531 / -3320）として存在する。
- 直前の Design は `docs/specs/issues-1840/design-04.md`。その「変える部分」2 項目は実装済みであり、その実装をこの周の開始状態として扱う。確認した範囲は次のとおり。
  - `abort_startup_failure`（`src-tauri/src/usecase/workflow/startup.rs:75-88`）は `repository.append(&record.root, &fact, timestamp, Some(record.head))`（`:85`）として読取時 head を条件に追記する。`StoredWorkflowStartupRepository::append`（`src-tauri/src/adaptor/gateway/workflow/startup_repository.rs:117-129`）は `expected_head` を `(tree_id, head)` として append へ渡す。
  - 起動と Abort の直列化は同一 worktree に戻っている。`workflow_start_locks` は `HashMap<String, Weak<Mutex<()>>>`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:102`）であり、`workflow_start_lock`（`:462-472`）が `WorkspaceIdentity` をキーに lock を引く。`start_workflow`（`:660-661`）と `abort_workflow_execution`（`src-tauri/src/adaptor/gateway/workflow/workflow_host/lifecycle_commands.rs:55-56`）はどちらも対象 worktree の lock だけを取得する。
- この周までに解消・見送りとなった Thread: Design 02 の対象 6 件（`2b34c2d8`、`ca6a9a6f`、`628753e0`、`ac454646`、`1a575f53`、`d3d2fbc2`）は resolve 済みである。Design 04 の対象のうち `e6193ac3-a9e2-4aa5-a54d-090f68e6e2e5` は resolve 済みである。Design 03・Design 04 の対象だった `2e74c849-b789-44c6-884a-e96ddd4899e9` は `[STILL_OPEN]` を経て対象範囲が変わり、open のまま今周の対象である。
- この周の対象は `[FIX_POLICY]` が付いた open Thread 1 件（`2e74c849-b789-44c6-884a-e96ddd4899e9`）だけである。
- Requirements・Behavior はこの周で変更していない。R-001〜R-011 と B-001〜B-014、対応表は Design 04 の周と同じである。
- Design 03 で対応した `reconcile_tree_pass` 側の `OutcomeUnknown` 読み直し（`src-tauri/src/adaptor/gateway/workflow/fact_log.rs:1007-1060`）は開始状態で維持されている。

## 変える部分

- 起動時の前進で SessionAttached の commit が `OutcomeUnknown` を返したとき、前進事実の保存有無を最新の記録で判定し、保存済みの実行木を Abort しない: 起動時の前進で起こされる Session Node の `SessionAttached` は `start_nodes_once`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:1172-1230`）で一括 commit され、`commit_required_events`（`:1916-1946`）→ `commit_control_plane_candidate`（`:782-851`）→ `append_events_at_head`（`:320-348`）を通る。`append_events_at_head` は `NodeEventWriteError::Conflict` だけを `WorkflowRuntimeError::Conflict` へ写し、`OutcomeUnknown` を含むその他は保存有無を確認せず `WorkflowRuntimeError::SessionStore` へ写す（`:342-347`）。`OutcomeUnknown` は「admission 失敗または reply 喪失で durable かどうか不明であり、呼び出し側はログから再導出して冪等に再試行する」状態である（`src-tauri/src/adaptor/gateway/local_event_store/writer.rs:71-74`）。`reconcile_tree`（`:593-627`）は `start_nodes` の `Err` をそのまま返し、`StoredWorkflowStartupRepository::reconcile_tree`（`startup_repository.rs:29-34`）が Conflict 以外を `WorkflowError::external` へ写すため、`WorkflowStartupUsecase::execute`（`startup.rs:48-69`）が前進失敗として `abort_startup_failure` へ渡す。`abort_startup_failure` の head 条件は load と append の間の競合窓を閉じるだけであり、`SessionAttached` が保存済みなら読み直した記録にもそれが含まれて実行木は active のままのため、成功済みの前進に `AbortRequested` が重なる。根拠: Thread `2e74c849-b789-44c6-884a-e96ddd4899e9`。R-005（理由付き Abort の対象は前進が失敗した実行木）、R-004（起動時に行う処理の範囲）、R-002（判定は記録の最新の状態に基づく）、B-006。B-005 の途切れた前進の継続と B-007 の他の実行木への非波及は維持する。ルート: 委任

## 固定するルート

- この周に人間が新しく固定した実装上の指定は無い。
- Design 01 で固定した D1〜D8 を維持する。今周の「変える部分」に直接かかるのは D1（エンジンの作業用の実行木は必要になった時点で記録から読み込み、メモリ上の一覧を正本にしない）である。
- `SessionAttached` の commit が `OutcomeUnknown` を返したときに保存有無をどこで（`append_events_at_head` 側か、`commit_control_plane_candidate` 側か、startup usecase 側か）どう判定するか、および共通の control-plane commit 経路を変えるか起動時経路だけに限定するかは委任である。
- `fact_log.rs:1007-1060` の `OutcomeUnknown` 読み直しと `abort_startup_failure` の head 条件は、この周の変更対象に含めない。

## 変えないもの

なし。

## 未確定・リスク

- 自動判断: なし。Requirements と Behavior の対応に誤り・不足・矛盾を見つけなかったため、どちらも編集していない。未決のまま残した要求は無く、`[DEFERRED]` で人間へ渡した件も無い。
