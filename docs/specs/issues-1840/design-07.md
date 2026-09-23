# Design 07

## 開始状態

- 差分の基準は base ブランチ `main`、派生点 `81ec380b`。branch `feat/issues/1840` には commit `c0c7c985`（Design 01〜06 の実装と spec 8 ファイル）と `8aa63a4e`（承認待ち Command を記録から承認待ちとして読み戻す。`src-tauri/src/adaptor/gateway/workflow/fact_log.rs`、`src-tauri/src/domain/workflow/services/fact_replay.rs`、`src-tauri/tests/desktop_update.rs`）が積まれ、作業ツリーは clean である。この 2 commit の内容をこの周の開始状態として扱う。
- 直前の Design は `docs/specs/issues-1840/design-06.md`。その「変える部分」1 項目は実装済みであり、`StoredWorkflowStartupRepository::append`（`src-tauri/src/adaptor/gateway/workflow/startup_repository.rs:117-129`）は `NodeEventWriteError::OutcomeUnknown` を受けたとき、追記しようとした行が保存されているかを問い合わせて成否を判定する。
- この周までに解消・見送りとなった Thread: resolve 済み 9 件（`2b34c2d8`、`ca6a9a6f`、`628753e0`、`ac454646`、`1a575f53`、`d3d2fbc2`、`e6193ac3-a9e2-4aa5-a54d-090f68e6e2e5`、`2e74c849-b789-44c6-884a-e96ddd4899e9`、Design 06 の対象 `2d4f0269-e8c9-41c1-bb42-b8e6fe12c88e`）。`[REJECTED]`・`[DEFERRED]` とした Thread は無い。
- この周の対象は `[FIX_POLICY]` が付いた open Thread 3 件（`bd5c77a5-8907-4654-9474-4b7a849631fe`、`b96be5ac-cf64-41b3-b189-5381a298ab73`、`0eaf1411-cf6f-42a3-b0b0-4ed00b96fd10`）である。
- Requirements・Behavior はこの周で変更していない。R-001〜R-011 と B-001〜B-014、対応表は Design 06 の周と同じである。

## 変える部分

- 定義を解釈できない実行の起動時 Abort に、読取時点の head との競合検出を付ける: `abort_unavailable_definition`（`src-tauri/src/usecase/workflow/startup.rs:90-103`）だけが `repository.append` の `expected_head` へ `None` を渡し、失敗側の `abort_startup_failure`（同 `:75-87`）は `Some(record.head)` を渡す。`StoredWorkflowStartupRepository::append`（`src-tauri/src/adaptor/gateway/workflow/startup_repository.rs:117-128`）は `expected_head` が `None` なら head 条件を付けないため、`load` から `append` までの間に別 writer が完了または Abort の事実を書いても競合を検出せず、`AbortRequested` を重ねる。起動時の処理は画面・API の提供開始を待たないため、この窓は実在する。根拠: Thread `bd5c77a5-8907-4654-9474-4b7a849631fe`。R-004「完了または Abort の事実が無く保存定義を現行コードで解釈できない実行を理由付きで Abort する」、B-013。ルート: 委任
- 隔離合成子の child-start commit が再試行上限後も競合したとき、同じ batch で準備済みの兄弟処理を失わせない: `prepare_isolated_starts`（`src-tauri/src/adaptor/gateway/workflow/workflow_host/isolated_worktree.rs:129-139`）の failures ループは `settle_runtime_failure_for_node` を `?` で受けるため、`prepared.leaves` と `prepared.injections` が呼び出し元へ返らない。`start_nodes`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:1010-1023`）は `start_nodes_once` が `Err` なら `schedule_startup_retries` へ到達せず、`prepared.failed` の自動再試行も行われない。結果、Running の事実だけを持ち実プロセスも失敗の事実も無い leaf が残る。根拠: Thread `b96be5ac-cf64-41b3-b189-5381a298ab73`。R-004「途切れた前進を続ける」、B-005、および Requirements の Scope / Non-goals が「#1839 が入れた Node の起動失敗の自動再試行」を変更しない対象としている点。ルート: 委任
- 起動時の前進の append 条件と `OutcomeUnknown` の読戻しの起点を、読取時点の raw head にする: `reconcile_tree_pass` は head を domain record の末尾 seq から作る（`src-tauri/src/adaptor/gateway/workflow/fact_log.rs:971`）が、`decode_stored_fact`（同 `:800-808`）が `isolated_worktree_created` / `isolated_worktree_released` / `isolated_worktree_lost` を `None` として落とすため、末尾 raw 行がこの 3 つのいずれかだと、store 側が比較する raw 行の `MAX(seq)`（`src-tauri/src/adaptor/gateway/local_event_store/store.rs:738-744`）とずれる。競合が無くても前進の commit（`fact_log.rs:1020-1025`）が常に `Conflict` になり、起動時の前進失敗として理由付き Abort の対象になる。同じ head は `OutcomeUnknown` の読戻し（同 `:1026-1035` の `read_tree_page`）の起点でもある。影響は旧形式の行を末尾に持つ既存 DB の active な実行木に限られる（この 3 つの event_type を書く現行コードは `src-tauri/src` に無い）。根拠: Thread `0eaf1411-cf6f-42a3-b0b0-4ed00b96fd10`。R-004、B-005。ルート: 委任

## 固定するルート

- この周に人間が新しく固定した実装上の指定は無い。上の 3 件はいずれもルートが委任である。head 条件を呼び出し元 usecase 側で付けるか repository の `append` 側で付けるか、競合上限到達時に準備済みの兄弟を失わせない実現方法と競合した composite のその後の扱い、読取時点の raw head をどこでどう取得して append 条件と読戻しへ渡すかは、いずれも実装側で決める。
- Design 01 で固定した D1〜D8 を維持する。

## 変えないもの

- `settle_runtime_failure_for_node`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:2041-2050`）が競合をそのまま返し、対象 Node の失敗として記録しない現在の扱いを維持する。理由は、競合は Node の実行が失敗したことではなく記録が先に進んだことを表すためである。

## 未確定・リスク

- 自動判断: なし。R-001〜R-011 と B-001〜B-014 は対応表に欠落なく相互に対応しており、誤り・不足・矛盾を見つけなかったため、Requirements・Behavior は変更していない。未決のまま残した要求は無く、`[DEFERRED]` で人間へ渡した件も無い。
