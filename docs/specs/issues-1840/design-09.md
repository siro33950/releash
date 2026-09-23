# Design 09

## 開始状態

- 差分の基準は base ブランチ `main`、派生点 `81ec380b`。branch `feat/issues/1840` には commit `c0c7c985`（Design 01〜06 の実装と spec 8 ファイル）と `8aa63a4e`（承認待ち Command を記録から承認待ちとして読み戻す）が積まれ、さらに Design 07 と Design 08 の実装が未コミットの作業ツリー差分（13 ファイル、+470 / -22）として存在する。この 2 commit と作業ツリー差分をこの周の開始状態として扱う。
- 直前の Design は `docs/specs/issues-1840/design-08.md`。その「変える部分」2 項目はいずれも実装済みである。承認要求付き Command が承認待ちへ入る 2 経路には `fact_log_test.rs:562-592` と `fact_replay_test.rs` のテストが付いた。control plane の commit 排他は `commit_locks: Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:103`、`:482`）と `commit_lock(execution_id)`（同 `:512-519`）になり、`:857-858` と `:1425-1426` が実行木ごとのロックを取る。
- この周までに解消・見送りとなった Thread: resolve 済み 14 件（`2b34c2d8`、`ca6a9a6f`、`628753e0`、`ac454646`、`1a575f53`、`d3d2fbc2`、`e6193ac3-a9e2-4aa5-a54d-090f68e6e2e5`、`2e74c849-b789-44c6-884a-e96ddd4899e9`、`2d4f0269-e8c9-41c1-bb42-b8e6fe12c88e`、`bd5c77a5-8907-4654-9474-4b7a849631fe`、`b96be5ac-cf64-41b3-b189-5381a298ab73`、`0eaf1411-cf6f-42a3-b0b0-4ed00b96fd10`、および Design 08 の対象 `3b9a9400-85b8-42d4-a94c-b7542ad45ed9`、`bbd6377c-4e9c-4bb3-8db9-92ef12f1a111`）。`[REJECTED]`・`[DEFERRED]` とした Thread は無い。
- この周の対象は `[FIX_POLICY]` が付いた open Thread 5 件（`aa600784-1eaf-4f9f-937c-8b565d8de377`、`2469600b-1262-4c9c-89a3-8fbd16e391e3`、`1f2cd7fe-2857-4797-866d-84afd198fbfb`、`7111e43d-9043-428b-a11e-c7637597306d`、`5591688b-8c7f-4b0e-a178-dfd49a401733`）である。
- Requirements・Behavior はこの周で変更していない。R-001〜R-011 と B-001〜B-014、対応表は Design 08 の周と同じである。

## 変える部分

- control-plane commit の Conflict 再試行を 1 つの実装へ集約する: `attempts` を増やして `WorkflowRuntimeError::Conflict` のあいだだけ `crate::usecase::workflow::command::CONTROL_PLANE_MAX_ATTEMPTS` まで継続する同一の制御が `adaptor/gateway/workflow` 配下の 7 箇所（`isolated_worktree.rs:185`、`command_preparation.rs:112`、`lifecycle_commands.rs:74`、`workflow_host.rs:1267`・`:1771`・`:1834`・`:2067`）に手書きされている。usecase 側には `WorkflowError` 向けの `retry_control_plane_conflicts`（`src-tauri/src/usecase/workflow/command/mod.rs:21-35`）が既にある。集約後も各経路は呼び出しと上限到達時の固有処理だけを持つ。根拠: Thread `aa600784-1eaf-4f9f-937c-8b565d8de377`。`docs/architecture/README.md:32`「同じ操作の実装は 1 つに集約する」。B-006・B-007・B-011・B-014 の観測結果は変わらない。ルート: 委任
- `start_prepared_composite` の Succeeded 再入分岐に domain テストを付ける: `src-tauri/src/domain/workflow/entities/workflow_execution/worktree.rs:80-107` は、対象 composite が Succeeded のときに祖先列から `derive_pending_advances` の `AfterChild` を探して `apply_pending_advance` を返すか、`pending_delegate_injection` から `NodeStart::InjectDelegate` を返す分岐である。`worktree_test.rs` で `start_prepared_composite` を呼ぶ 6 箇所（`:95`、`:145`、`:302`、`:467`、`:502`、`:526`）はいずれも対象 Node が Succeeded になる前の初回呼び出しであり、Succeeded の aggregate へ再度呼んだ結果を検証するものは無い。根拠: Thread `2469600b-1262-4c9c-89a3-8fbd16e391e3`。この分岐が復元する前進は R-004「起動時に行う処理は、途切れた前進を続けること」と B-005 が要求する結果であり、`docs/architecture/TEST.md:20-24` は `domain/` のテストを必須とする。ルート: 委任
- 1 回の commit あたり `RuntimeCommitSnapshot` の生成を 1 回にする: `commit_control_plane_candidate` は candidate から `RuntimeCommitSnapshot` を生成して返す（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:844`）が、`commit_required_events` は同 `:1996` の `.map(|_| ())` でこれを捨てており、呼び出し元 3 経路（同 `:1252` の session attach、同 `:1750` の command 完了、`isolated_worktree.rs:170` の isolated composite）が同じ candidate から重ねて生成している。`RuntimeCommitSnapshot::from_execution`（`src-tauri/src/usecase/workflow/runtime_snapshot.rs:37-`）は `node_history`・`workflow_definition`・`artifacts`・`node_executions` を clone して実行木の状態全体を作り直す。根拠: Thread `1f2cd7fe-2857-4797-866d-84afd198fbfb`。`AGENTS.md`「full-retention 設計を避ける。…workflow state 全体を clone / store / recompute / resend しない」および「レビュー観点」の「full-retention / full-recompute 経路を増やしていないか」。`finalize_after_commit` と各経路の戻り値が使う snapshot の内容は変えない。Requirements への性能要求の追加は行わない。ルート: 委任
- `WorkflowStartupRepository` の port から永続化行の seq を除く: `src-tauri/src/domain/workflow/repository.rs:109-125` は `WorkflowStartupRecord.head: i64`（同 `:113`）と `append(..., expected_head: Option<i64>)`（同 `:124`）を宣言し、`adaptor/gateway/workflow/startup_repository.rs:66-72` はこの `head` へ `SELECT COALESCE(MAX(seq), 0) FROM node_events` の結果をそのまま入れ、同 `:119-125` で store の expected tree head へ渡している。根拠: Thread `7111e43d-9043-428b-a11e-c7637597306d`。`docs/architecture/DOMAIN.md:96-100`「domain 層に置く port は、ドメインの言語だけで書く。引数・戻り値・エラーに外部世界の語彙（…SQL の行…）を出さない」。R-004・R-005 の起動時の前進と理由付き Abort、R-007・R-008 の worktree 排他について、B-005・B-006・B-007・B-008・B-009・B-013 の観測結果は変わらない。Requirements の Non-goals「事実ログの記録先と形式」は変更しない。ルート: 委任
- `OutcomeUnknown` 後の永続化判定を 1 つの実装へ集約する: 追記行が永続化済みかを判定して durable / Conflict / `OutcomeUnknown` のどれに落とすかを決める同一の操作が 3 箇所にある。`fact_log.rs:1036-1073` と `workflow_host.rs:343-378` はいずれも `read_tree_page` の結果と pending 行の 10 フィールド（`tree_id`、`node_execution_id`、`parent_id`、`node_name`、`kind`、`attempt`、`event_type`、`session_id`、`detail`、`timestamp_ms`）を突き合わせ、`startup_repository.rs:131-171` は同じ 10 フィールドを raw SQL の `EXISTS` で突き合わせる。根拠: Thread `5591688b-8c7f-4b0e-a178-dfd49a401733`。`docs/architecture/README.md:32`「同じ操作の実装は 1 つに集約する」。同じ永続状態に対して経路ごとに異なる結果にならないようにし、B-005・B-006・B-013 の観測結果は変えない。ルート: 委任

## 固定するルート

- この周に人間が新しく固定した実装上の指定は無い。上の 5 件はいずれもルートが委任である。Conflict 再試行をどこへ集約するか（usecase の `retry_control_plane_conflicts` を `WorkflowRuntimeError` へ広げるか、gateway 側に別の共通実装を置くか）と各経路が上限到達時の固有処理をどう受け取るか、Succeeded 再入分岐の domain テストをどのファイルにどの粒度で置きどの workflow 定義で `AfterChild` と delegate injection の 2 つの返却を作るか、`commit_required_events` が snapshot を返す形にするか呼び出し元が事前生成をやめる形にするか、raw seq を永続化境界へ閉じるための port の型と append の競合条件の表し方、永続化行の照合と判定をどこへ集約し 3 経路がどう共有するかは、いずれも実装側で決める。
- Design 01 で固定した D1〜D8 を維持する。

## 変えないもの

- `settle_runtime_failure_for_node`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs`）が競合をそのまま返し、対象 Node の失敗として記録しない扱いを維持する。Design 07 で固定した判断であり、理由は、競合は Node の実行が失敗したことではなく記録が先に進んだことを表すためである。Conflict 再試行を集約した後もこの扱いは変わらない。
- 隔離合成子の child-start commit が再試行上限後も競合したときに warn を記録して次の兄弟へ進む扱いを維持する。Design 07 で固定した判断である。

## 未確定・リスク

- 自動判断: なし。R-001〜R-011 と B-001〜B-014 は対応表に欠落なく相互に対応しており、誤り・不足・矛盾を見つけなかったため、Requirements・Behavior は変更していない。未決のまま残した要求は無く、`[DEFERRED]` で人間へ渡した件も無い。
