# Design 01

## 開始状態

- 差分の基準は base ブランチ `main`、派生点 `81ec380b`（branch `feat/issues/1840`、`origin/main` と同一）。未コミット差分は `docs/specs/issues-1840/` の Requirements・Behavior だけで、`src-tauri/` は未変更である。
- 初回の周であり、既存の `design-NN.md` は無い。開始時点の挙動は `docs/specs/issues-1840/requirements.md` の Current Behavior を参照する。
- この周までに解消・見送りとなった Thread: なし（open Thread は無い）。
- 開始状態で既に満たされている要求（「変える部分」に入れない）:
  - R-011 / B-012。`shutdown_all_active_commands`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:1922-1987`）は `node_processes.active_commands`（`src-tauri/src/adaptor/gateway/workflow/node_process.rs:15`）から停止対象を作り、`executions` を読まない。#1835（`6cc1572a`）で既にこの形であり、Issue #1840 本文が現状として挙げた「メモリ上の登録簿だけから作られる」は #1835 のマージ前の記述である。
  - R-006 / B-007。`WorkflowStartupUsecase::execute`（`src-tauri/src/usecase/workflow/startup.rs:29-51`）は実行木ごとの失敗を `first_error` に記録してループを継続するため、1 本の失敗で他の実行木の前進は止まらない。R-005 の変更でこの継続を失わないことが条件である。
  - R-004 の一部。完了・Abort の事実が無く保存定義を解釈できない実行を起動時に理由付きで Abort する処理（`startup.rs:36-41` の `abort_unavailable_definition`、`src-tauri/src/domain/workflow/entities/workflow_execution/terminal_replay.rs:39-54`）は #1836 で入っており、そのまま残る（B-013）。

## 変える部分

- 起動時のプロセス喪失の記録を外す: `reconcile_tree_pass`（`src-tauri/src/adaptor/gateway/workflow/fact_log.rs:940-977`）が attach / spawn 済みの leaf へ追記する `ProcessExited { failure_reason: "process lost across application restart" }` をやめる。根拠: R-001「起動時の処理は、プロセスを失ったことを表す事実を記録しない」、B-001。ルート: D2
- エンジンの操作の判定元を記録へ移す: 承認・Submit・Retry・Session の Resume・provider の Stop が読む実行木を、`load_control_plane_execution`（`workflow_host.rs:279-284`）による `executions` 参照から記録の読み直しへ変える。Retry の在否確認が `executions` から Node を引く経路（`workflow_host.rs:902-922`）と、一覧に無いときだけ `recover_startup()` を呼ぶ穴埋め（`src-tauri/src/usecase/workflow/control_plane.rs:598-601`）も同じ形に揃える。根拠: R-002「判定は、AgentSession が書いた事実を含む記録の最新の状態に基づく」、R-003、B-002、B-003、B-004。ルート: D1、D6
- 起動時にメモリ上の一覧と active 登録簿へ登録する処理を外す: `reconcile_tree`（`workflow_host.rs:621-695`）の `executions.insert` と `execution_store.reconcile_orphan_from_projection` / `register_active_execution`、および `startup.rs:35-38` の `is_registered_or_reserved` による skip を、起動時の処理から外す。根拠: R-002、R-004「起動時に行う処理は、途切れた前進を続けることと、…理由付きで Abort することである」、B-002、B-005。ルート: D1
- 起動時の前進の失敗を Abort で収束させる: `reconcile_tree` / `reconcile_tree_pass` の失敗した実行木を、失敗の理由を伴って Abort する。根拠: R-005「起動時の前進が失敗した実行木は、失敗の理由を伴って Abort され」、B-006。ルート: 委任
- 起動時の前進を再試行しない: `WorkflowStartupUsecase::execute` が `first_error` で `Err` を返し、`run_startup_recovery`（`src-tauri/src/adaptor/controller/daemon.rs:312-329`、`daemon.rs:468-502`）が上限なく再試行する経路をやめる。根拠: R-005「その実行木に対して前進は再試行されない」、B-006。ルート: 委任
- worktree 排他の判定元を記録へ移す: `reserve_workflow_execution`（`workflow_host.rs:495-530`）が `execution_store` の in-memory `by_worktree` で判定し、`insert_workflow_execution`（`workflow_host.rs:564-619`）が `find_any_by_worktree`（`src-tauri/src/adaptor/gateway/workflow/workflow_host/execution_registry.rs:10-18`）でメモリ上の一覧を引く形をやめ、起動する時点の記録から計算する。根拠: R-007「同一の worktree で実行中の workflow の実行木は 1 件までである。この判定は、起動する時点の記録から計算される」、B-008。ルート: D4、D7
- 同一 worktree への同時起動要求の扱いを置き直す: 現在は `executions` の Mutex と `execution_store` の Mutex が検査と登録を原子的にしている（`workflow_host.rs:606-618`）。判定元を記録へ移した後の同時起動の排除を、起動処理の直列化で成立させる。根拠: R-008「同一の worktree に対する起動要求が同時に複数行われた場合、成功するのは 1 件だけである」、B-009。ルート: D3、D7
- 排他で拒否したときのエラーの中身と文言を変える: `WorkflowRuntimeError::AlreadyActive(workflow.name)`（`workflow_host.rs:521-523`、`runtime_start_guard.rs:14-25`）が起動しようとした側の名前を持つ形をやめ、worktree を塞いでいる実行を識別できる情報を持たせる。文言（`src-tauri/src/usecase/workflow/runtime_error.rs:41-43` の `Workflow '<name>' is already running for this session`）も排他の単位が worktree であることを示す形にする。根拠: R-009、B-010。ルート: D7
- command の起動・結果・失敗の反映を記録ベースにし、反映しない理由をログへ残す: `command_execution_still_current`（`workflow_host.rs:1706-1712`）、`commit_command_output`（`workflow_host.rs:1714-1835`）、`fail_current_command_node`（`workflow_host.rs:1837-1874`）が `executions` を引いて、無ければ・current でなければ `Ok(())` で黙って返す形をやめる。判定は記録から読み直した実行木に対して行い、反映しない場合は反映しなかったことと理由をログに残す。根拠: R-010「記録の最新の状態に基づいて対象の実行木へ反映される。反映できない場合は、反映しなかったことと理由がログに記録される」、B-011、B-014。ルート: D8

## 固定するルート

- D1: エンジンの作業用の実行木の一覧は、必要になった時点で記録から読み込む。メモリ上の一覧を正本として扱わない。範囲はエンジンが操作の可否と結果を判定する経路全般。粒度は取得元の指定のみで、読み込み方・単位・頻度は委任。理由はメモリ上の状態と記録のずれを無くすこと。関係: R-002、R-003、B-002、B-003、B-004
- D2: プロセスが居るかどうかは、#1839 が入れた判定（`NodeProcessPresence`、`src-tauri/src/adaptor/gateway/workflow/node_process.rs`、`src-tauri/src/domain/workflow/value_objects/node_execution.rs`）を表示と操作のときに使う。範囲は表示と操作の可否の判定。粒度は既存の判定を使うことの指定。理由は起動時に「プロセスを失った」を記録しない代わりの読み取り口であること。関係: R-001、B-001
- D3: 同一 worktree への同時起動要求は、起動処理の直列化で防ぐ。範囲は同一 worktree に対する同時の起動要求。粒度は手段の指定のみで、直列化の単位と実装箇所は委任。理由は記録から計算する方式では検査と登録の間に隙間ができること。関係: R-008、B-009
- D4: worktree の排他の判定元には、既存の問い合わせ `SqliteWorkspaceQueryService::execution_records`（`src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs:50-99`）を使い、新しい問い合わせを作らない。範囲は排他の判定元。粒度は既存の問い合わせを使うことの指定。理由は同じ backend-owned state を再利用すること。関係: R-007、B-008
- D5: アプリケーション終了時に止める command の対象は、実行中の command を載せている表（`node_processes.active_commands`）から作る。範囲は終了時の停止対象。粒度は取得元の指定。理由は #1835 と揃えること。開始状態で既にこの形であり、この周では変更しない。関係: R-011、B-012
- D6: 着手時に、エンジンの全ての経路が「必要になった時点で記録から読み込む」形で成立するかを洗い出して確認する。範囲はエンジンの全経路。粒度は手順の指定。理由は調査時点で未確認であること。関係: R-002
- D7: worktree の排他（同一 worktree で実行中の workflow の実行木は 1 件まで）という規則と、拒否のときに塞いでいる実行を示す結果を、domain 層の型で表現する。判定に渡す候補は記録から集める。usecase の `validate_start`（`src-tauri/src/usecase/workflow/runtime_start_guard.rs:14-25`）は domain への委譲だけにし、`WorkflowRuntimeError::AlreadyActive` の中身も domain 側の結果から作る。範囲は排他の規則と拒否の結果。粒度は層の配置の指定のみで、型の形は委任。理由は、R-009 でエラーの中身を変える以上に判定と結果を作り直すことになり、規則を usecase に残すと判定元・規則・候補集めが 3 層に散ること。関係: R-007、R-008、R-009、B-008、B-009、B-010
- D8: command の起動・結果・失敗を対象の実行木へ反映できるかの判定を、domain 層の規則として 1 つにする。呼び出し側（`command_execution_input_is_current`（`src-tauri/src/adaptor/gateway/workflow/workflow_host/command_preparation.rs:19-32`）と `fail_current_command_node`）は、記録から読み直した実行木を渡して結果を受け取る形にする。範囲は反映可否の判定。粒度は層の配置と規則を 1 つにすることの指定のみで、規則の細部は委任。理由は、現在 2 通りの規則が 2 箇所にあり（`status == Running` か `status.is_active()` か、`node_name` と `attempt` を見るか）、どちらで判定した結果が観測されるかが決まらないこと。関係: R-010、B-011、B-014

## 変えないもの

維持する条件は Requirements の「Scope / Non-goals」の「変更しない対象」が正であり、ここで再掲しない。

## 未確定・リスク

- 自動判断: なし。R-001〜R-011 と B-001〜B-014 は対応表に欠落なく相互に対応しており、誤り・不足・矛盾を見つけなかったため、Requirements・Behavior は変更していない。Assumptions / Open Questions も「なし」で、未決のまま残した要求は無い。`[DEFERRED]` で人間へ渡した件も無い。
- control plane の commit は `commit_snapshot_is_current`（`workflow_host.rs:264-272`）で、操作前に取った snapshot と commit 時点の in-memory aggregate の一致を競合検出に使う。判定元を記録の読み直しへ移すとき、この競合検出に相当するものが未確定である。置き換え方によっては、R-002・R-003 を満たしても同時操作で記録が壊れうる。
- `execution_store` の active 登録簿は worktree 排他以外（reservation と cancel、rollback snapshot、terminal 遷移、projection 更新）にも使われ、`workflow_host.rs` と submodule（`workflow_host/lifecycle_commands.rs`、`workflow_host/runtime_commit.rs`、`workflow_host/isolated_worktree.rs`）から参照される。排他の判定元を記録へ移した後にこの登録簿が残る場合、登録簿と記録のずれという同じ性質の不整合がどこまで残るかが未確定である。
