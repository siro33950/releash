# 定義が受理した fanout の並列度を terminal surface の同時上限が実行できない

## 現象

同一 worktree 上で、Session child を 33 個持つ fanout を含む workflow 定義を実行する。定義検証はこの定義を受理し、実行が始まる。先行する 32 個の Session leaf が生存したまま 33 個目を activate する時点で、33 個目の NodeExecution が Failed になる。

再現実行では 33 番目の child（`review-32`）だけが Failed となり、`failure_reason` は `workflow runtime activation failed`、内訳は `kind=per_worktree_cap`、`worktree_path` は先行 32 個と同一だった。定義が受理した並列度が、実行前に不受理になるのでもなく、実行中に個別 Node の失敗として現れる。

## 原因

定義検証が受理した fanout は、その並列度を実行できるか実行前に不受理になるべきところ、定義側は最大 64 children（さらに `items` × `children` の展開）を受理する一方、runtime は 33 個目を `WorktreeCapReached` として NodeFailed にしている。

- 定義側上限は `src-tauri/src/domain/workflow/value_objects/definition.rs:11-12` の `MAX_NODES_PER_WORKFLOW = 256` / `MAX_FANOUT_CHILDREN = 64`。`src-tauri/src/domain/workflow/services/validation.rs:1547-1576` の `collect_node_count_errors` は `fanout.children.len() > MAX_FANOUT_CHILDREN` だけを拒否し、実行側の同時上限を参照しない。
- `worktree` フィールドは `src-tauri/src/domain/workflow/services/validation.rs:1167-1176` の `collect_unsupported_errors` が `UnsupportedWorktreeField` として弾くため、現行定義から child を別 worktree へ分散できない。全 child は親と同じ worktree で走る。
- `src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs:995-1100,1161-1171` の `expand_fanout_scope` は `items` × `children` の全座標を一度に生成し、Session child を全て `LeafStart` に積む。
- `src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:1429-1455` の `start_leaves` はそれらを同じ execution の `worktree_path` で全 prepare した後、child の完了を待たずに順番に activate する。
- activate は `src-tauri/src/usecase/agent_session/agent_session_launch.rs:438-475,742-770` から terminal spawn へ入り、`src-tauri/src/usecase/terminal_surface/spawn_usecase.rs:257-277` が owner の worktree で `reserve_spawn_slot` を呼ぶ。
- `src-tauri/src/domain/terminal_surface/entities/terminal_surface_registry.rs:155-177` は `effective_alive_for_worktree + reserved_spawn_for_worktree >= per_worktree_cap` を total 判定より先に評価し、即時 `WorktreeCapReached` を返す。既定値は `src-tauri/src/domain/terminal_surface/value_objects/terminal_surface_lifecycle_config.rs:7-12` の per-worktree 32 / total 64。spawn usecase はこの cap error を待機も再試行もせずそのまま返す。
- `src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:1523-1558` はこの activation error を `settle_runtime_failure_for_node` で対象 NodeExecution の runtime failure として確定する。

上限値には並列度の根拠がない。#1215 の requirements A2 と design は per-worktree 32 を手動 UI の 4 panes × 8 tabs から採り、total 64 を「暫定・A2 に明示なし」としている。加えて導入コミット `2dad22e54` にあった LRU eviction と idle sweep は `5287c462b` で `PtyEvictReason` / `idle_timeout` / `sweep_interval` ごと削除され、現行設定は cap 2 値だけである。上限に達しても回収されず、拒否だけが残っている。

## 棄却した仮説

- #1654 と同じく、終端済み Session の残留累積が今回の直接原因である: 現行 `TerminalSurfaceRegistry` の effective count は exited surface を除外し、`3c4baefad`（#1654）が Session Node 終端時に provider process を停止して枠を解放している。過去実行の累積は修正済みで、同時に Running な fanout children が 32 枠を占める経路が別に残っている。
- fanout 親も terminal を 1 枠使うため、64 children と親の合計 65 が total cap 64 を超えることが最初の失敗原因である: aggregate は Fanout を `ScopeRuntime` として保持し、`LeafStart` に積むのは Command / Session だけである。さらに全 child が同一 `worktree_path` で起動され、registry は total より先に per-worktree を判定するため、現行コードでは 33 個目が per-worktree cap 32 で先に失敗する。
- cap 到達時に導入時の LRU eviction または idle sweep が働き、新規 child の枠を回収できる: `2dad22e54` の旧 `reserve_spawn_slot` は idle target を選び `PtyEvictReason::CapExceeded` を返していたが、`5287c462b` でその型と idle lifecycle 設定が削除された。現行 registry は cap 到達時に既存 surface を選ばず直ちに `WorktreeCapReached` / `TotalCapReached` を返す。

## 再現

- `src-tauri/tests/workflow_control_plane_acceptance_test.rs`
- `test_fanout_受理した33個sessionは同一worktreeで全て起動するか実行前に拒否する`

## 期待

- E-001: 同一 worktree 上で 33 個以上の Session child を持つ fanout を含む定義を実行したとき、全 child の NodeExecution が起動する。同時起動数の上限到達を理由に Failed になる NodeExecution が存在しない。
- E-002: 同時起動数の上限到達を表す失敗（`kind=per_worktree_cap` / `kind=total_cap`）が、workflow 実行経路でも手動 terminal 経路でも発生しない。
- E-003: OS が terminal の spawn を拒否したとき、対象 NodeExecution は Failed になり、その失敗理由に OS が返したエラー文言が欠落せず含まれる。
- E-004: `MAX_FANOUT_CHILDREN` / `MAX_NODES_PER_WORKFLOW` を超える定義は、引き続き検証時点で不受理になる。

## 修正方針

- P-001: 実行側の同時上限（per-worktree / total）を撤廃する。上限値を並列度の根拠で決め直す案、および上限到達時の回収（LRU eviction / idle sweep）を復活させる案は採らない。機械資源の限界は OS 側の spawn 失敗として現れる形にする。
- P-002: 定義側の上限は据え置く。`items` 展開後の座標数に新たな上限を設けず、展開時点での実行前拒否を導入しない。
- P-003: OS 由来 spawn 失敗の究明可能性は、既存の伝播経路（gateway のエラー文言を NodeExecution の失敗理由へ載せる）をそのまま担保対象とする。資源枯渇のための新しい失敗分類・telemetry・表示といった観測面を増設しない。
- P-004: cap を注入手段にしていた回帰テストを削除せず、OS 由来の spawn 失敗を実経路で注入する形へ差し替えて E-003 を固定する。注入には provider 実行ファイルの実行権限剥奪など `access()` 起因で spawn が失敗する手段を使う。存在しない cwd は `CommandBuilder::as_command` が home へフォールバックさせるため注入点にならない。
