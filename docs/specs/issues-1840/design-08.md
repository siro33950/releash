# Design 08

## 開始状態

- 差分の基準は base ブランチ `main`、派生点 `81ec380b`。branch `feat/issues/1840` には commit `c0c7c985`（Design 01〜06 の実装と spec 8 ファイル）と `8aa63a4e`（承認待ち Command を記録から承認待ちとして読み戻す）が積まれ、さらに Design 07 の実装が未コミットの作業ツリー差分（`src-tauri/` で 9 ファイル、+269 / -4）として存在する。この 2 commit と作業ツリー差分をこの周の開始状態として扱う。
- 直前の Design は `docs/specs/issues-1840/design-07.md`。その「変える部分」3 項目はいずれも実装済みである。`abort_unavailable_definition`（`src-tauri/src/usecase/workflow/startup.rs:102`）は `expected_head` へ `Some(record.head)` を渡す。`prepare_isolated_starts`（`src-tauri/src/adaptor/gateway/workflow/workflow_host/isolated_worktree.rs:129-139`）は child-start commit の競合を warn で記録して次へ進み、準備済みの兄弟を呼び出し元へ返す。`reconcile_tree_pass`（`src-tauri/src/adaptor/gateway/workflow/fact_log.rs:914-928`）は head を raw 行の末尾 seq から作る。
- この周までに解消・見送りとなった Thread: resolve 済み 12 件（`2b34c2d8`、`ca6a9a6f`、`628753e0`、`ac454646`、`1a575f53`、`d3d2fbc2`、`e6193ac3-a9e2-4aa5-a54d-090f68e6e2e5`、`2e74c849-b789-44c6-884a-e96ddd4899e9`、`2d4f0269-e8c9-41c1-bb42-b8e6fe12c88e`、および Design 07 の対象 `bd5c77a5-8907-4654-9474-4b7a849631fe`、`b96be5ac-cf64-41b3-b189-5381a298ab73`、`0eaf1411-cf6f-42a3-b0b0-4ed00b96fd10`）。`[REJECTED]`・`[DEFERRED]` とした Thread は無い。
- この周の対象は `[FIX_POLICY]` が付いた open Thread 2 件（`3b9a9400-85b8-42d4-a94c-b7542ad45ed9`、`bbd6377c-4e9c-4bb3-8db9-92ef12f1a111`）である。
- Requirements・Behavior はこの周で変更していない。R-001〜R-011 と B-001〜B-014、対応表は Design 07 の周と同じである。

## 変える部分

- 承認要求付き Command が完了して承認待ちへ入る 2 経路にテストを付ける: `fact_log.rs:364-377` は `WorkflowEvent::ApprovalRequested` を `kind == Command` のときだけ `ProcessExited(exit_code: Some(0))` の行へ写し、`fact_replay.rs:432-455` はその行と `decide_completion_disposition == RequestApproval` から `mark_node_waiting_approval` を導出する。どちらにもテストが無い。`fact_log_test.rs` で `ApprovalRequested` を使う唯一のテスト（`:526` の `test_写像_実行完了だけ終端事実として記録する`、`:539`）は Session Node で Command 分岐を通らず、`fact_log_test.rs` の承認待ち復元テストと `fact_replay_test.rs` の approval テスト（`:1453`、`:1501`、`:1542`）はいずれも Session Node を使い、`fact_replay_test.rs` に `ApprovalRequested` の参照は無い。`tests/desktop_update.rs` と `daemon_smoke.rs:515` の承認付き Command は push 通知だけを確認する。作業ツリーの未コミット差分にも当該テストは無い。根拠: Thread `3b9a9400-85b8-42d4-a94c-b7542ad45ed9`。R-002「エンジンが操作の可否と結果を判定するとき、その判定は、AgentSession が書いた事実を含む記録の最新の状態に基づく」、B-002。`docs/architecture/TEST.md` は `domain/` と `adaptor/gateway/` のテストを必須とする。ルート: 委任
- control plane の commit 排他を対象ごとにキー化する: `commit_lock` は `WorkflowRuntimeHost` の `Arc<Mutex<()>>` 1 個（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:103`、`:482`）で、`:846` から SQLite の実行木読込と追記を、`:1413` から最新 Node 確認・外部 process の spawn・process 登録を同じロックの下で行う。`commit_control_plane_candidate` は control plane 操作・command 結果・command 起動準備・起動時前進・lifecycle command の各経路から共通に呼ばれるため、無関係な実行木の commit と command 起動が相互に待たされる。`workflow_start_locks`（`:102`）は WorkspaceIdentity、`runtime_activation_locks`（`:110`）は execution_id でキー化されている一方、このロックだけは対象を区別しない。同一実行木で、最新 Node 確認から process 登録までの間に stop が割り込まない現在の順序は維持する。根拠: Thread `bbd6377c-4e9c-4bb3-8db9-92ef12f1a111`。R-007・R-008 の worktree 排他は `workflow_start_locks` が担うため、キー化しても B-008・B-009 は変わらず、B-011・B-014 の command 結果の反映も変わらない。ルート: 委任

## 固定するルート

- この周に人間が新しく固定した実装上の指定は無い。上の 2 件はいずれもルートが委任である。承認要求付き Command のテストをどのファイルにどの粒度で置くか、`commit_lock` をどのキー（実行木 / worktree など）で分割し、競合した場合の取得順や既存の `workflow_start_locks`・`runtime_activation_locks` との関係をどう組むかは、いずれも実装側で決める。
- Design 01 で固定した D1〜D8 を維持する。

## 変えないもの

- `settle_runtime_failure_for_node`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs`）が競合をそのまま返し、対象 Node の失敗として記録しない扱いを維持する。Design 07 で固定した判断であり、理由は、競合は Node の実行が失敗したことではなく記録が先に進んだことを表すためである。

## 未確定・リスク

- 自動判断: なし。R-001〜R-011 と B-001〜B-014 は対応表に欠落なく相互に対応しており、誤り・不足・矛盾を見つけなかったため、Requirements・Behavior は変更していない。未決のまま残した要求は無く、`[DEFERRED]` で人間へ渡した件も無い。
