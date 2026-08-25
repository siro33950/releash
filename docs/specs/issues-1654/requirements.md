# Context

要求の正本は Issue #1654（https://github.com/siro33950/releash/issues/1654 、`[terminal surface] workflow node session の provider プロセスが完了後も残留し per_worktree_cap を食い潰してセッション起動不能になる`、OPEN、label: `bug`）。

Issue が「関連」として挙げる #1652（`[object Object]` 表示）と #1653（spawn 失敗理由の破棄・ロガー未初期化）は別 Issue であり、本変更の要求には含めない。

根拠となる既存実装:

- `src-tauri/src/domain/terminal_surface/value_objects/terminal_surface_lifecycle_config.rs`
- `src-tauri/src/domain/terminal_surface/entities/terminal_surface_registry.rs`
- `src-tauri/src/usecase/terminal_surface/lifecycle_usecase.rs`
- `src-tauri/src/adaptor/gateway/agent_session/provider_agent_terminal_gateway.rs`
- `src-tauri/src/usecase/agent_session/agent_session_launch.rs`
- `src-tauri/src/usecase/agent_session/agent_session_lifecycle.rs`
- `src-tauri/src/usecase/agent_session/agent_session_interrupt.rs`
- `src-tauri/src/adaptor/gateway/workflow/node_session_boundary.rs`
- `src-tauri/src/adaptor/gateway/workflow/workflow_host/lifecycle_commands.rs`
- `src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs`

確定済みの背景と制約:

- terminal surface には worktree ごとの上限がある。`TerminalSurfaceLifecycleConfig::default()` は `per_worktree_cap: 32` / `max_panes_total: 64`。
- 上限判定は「プロセスが exited でない surface の数 + 予約数」で行う。プロセスが生きている限り枠は解放されない。
- 既存の停止操作は意味が異なる3種類がある。`interrupt` は PTY へ Ctrl-C を書くだけでプロセスも枠も残る。`stop_preserving_checkpoint` はプロセスを終了し terminal checkpoint を残したまま枠を解放する。`delete` / `kill` はプロセス終了に加えて checkpoint も破棄する。
- AgentSession は `provider_session_id` を持つ場合、プロセス終了後に provider CLI 自身の再開機能（Claude は `claude --resume <provider_session_id>`、Codex は `codex resume <provider_session_id>`）で復帰できる。`provider_session_id` は provider hook 経由で取得される。
- node execution の状態は `Running` / `Paused` / `WaitingApproval` が active、`Succeeded` / `Failed` / `Aborted` が終端。
- execution の abort は active な node execution を `Aborted` へ遷移させる。execution の stop は node を `Paused` に留め、resume 可能な状態として残す。

停止の定義（provider プロセスを終了して terminal surface の枠を解放し、terminal checkpoint と `provider_session_id` は保持する）と、停止の時点（node execution の終端時点）は、本 spec 作成時に利用者が本 Session 内で明示的に指示した確定事項である。Issue の「対応」節が対応方針を未定としているのはこの指示より前の記述であり、R-001 と Scope / Non-goals はこの指示に従う。

# Outcome

対象は、同一 worktree で workflow を繰り返し実行する Releash 利用者。

現在の問題は、workflow の session node が起動した provider プロセスが node 終了後も終了されず、worktree ごとの terminal surface 上限を残留プロセスが埋めていくことである。上限に達すると、その worktree では workflow の node activation も手動の session 作成もすべて失敗し、UI 上は当該 worktree に実行中のものが何も表示されていないのに新規実行が一切できなくなる。利用者から残留プロセスは見えず、復旧手段は Releash の外から残留プロセスを手動で kill することしかない。

変更後は、session node execution が終端した時点でその node の provider プロセスが終了し、terminal surface の枠が解放される。同一 worktree で workflow を何度実行しても終端済み node の分が枠を占有せず、上限到達による起動不能が発生しない。停止された session は破棄されず、後から開けば provider の会話を引き継いで復帰できる。

# Current Behavior

## 上限と失敗の経路

- `TerminalSurfaceLifecycleConfig::default()` が `per_worktree_cap: 32` / `max_panes_total: 64` を返す（`terminal_surface_lifecycle_config.rs:9-12`）。
- `TerminalSurfaceRegistry::reserve_spawn_slot` は、対象 worktree の「exited でない surface 数 + 予約数」が `per_worktree_cap` 以上のとき `WorktreeCapReached` を返す（`terminal_surface_registry.rs:165-173`）。
- `WorktreeCapReached` は `ProviderAgentTerminalGatewayError::Unavailable` に丸められ（`provider_agent_terminal_gateway.rs:31`）、`AgentSessionLaunchUsecaseError::TerminalUnavailable` として workflow へ伝わる。workflow 側では `activate Workflow AgentSession '<id>': TerminalUnavailable` の形になる（`node_session_boundary.rs:146-153`）。

## provider プロセスが終了しない箇所

- node 完了（Submit）経路 `WorkflowControlPlane::submit_output_once`（`usecase/workflow/control_plane.rs:205-338`）には、terminal surface / provider プロセスを停止する処理が無い。
- workflow の stop / abort 経路は `interrupt_workflow_agent_session` を呼ぶだけで（`lifecycle_commands.rs:234-236`、`:686-689`）、その実体は PTY への Ctrl-C 書き込みである（`agent_session_interrupt.rs:40-44`）。プロセスは終了せず、枠も解放されない。`lifecycle_commands.rs:708` には「AgentSession と PTY は Workflow 終端後もユーザー操作のため保持する」と明記されている。
- execution のアーカイブは実行木の枝を表示から隠すだけで（`domain/workspace_tree/services.rs:66-84`）、プロセスや surface には作用しない。
- 時間経過や idle に基づく terminal surface の回収は存在しない。

## 枠が解放される既存の経路

- provider プロセスが実際に終了し、Releash がそれを検知したとき（`agent_session.rs:386-397` で AgentSession が `Paused` になる）。
- 利用者が session を archive / delete したとき（`agent_session_lifecycle.rs:272-274`、`:536-538`）。ただし workflow node session は execution をアーカイブすると実行木から見えなくなるため、この操作に到達できない。
- workflow node の launch rollback（`agent_session_launch.rs:514-517`）。
- worktree 自体を削除したときの `kill_by_worktree`（`usecase/repository_usecase.rs:243`）。

## 実障害での観測（2026-08-19、PJT-2308 worktree）

再現手順は「同一 worktree で session node を含む workflow を繰り返し実行する」。当日は 02:23 / 02:43 / 08:38 / 08:46 / 10:39 / 10:53 の実行が該当した。

観測された事実:

- 障害時点で、Releash の直接子プロセスのうち cwd が当該 worktree のものがちょうど 32 個残留していた（claude 14、codex/node 17、zsh 1）。
- 11:46:59 / 11:47:22 に `05_review-fix`（execution `e47d4ec8`）の `create_fix_plan` attempt 2 / 3 が次の出力で失敗した。

  ```
  workflow runtime activation failed: activate Workflow AgentSession '...': TerminalUnavailable
  ```

- 11:47:26 に `execution_aborted`。
- 11:48:25 / 11:48:29 / 11:48:35 の手動 session 新規作成 3 回（claude 2、codex 1）が即時失敗し rollback した（`binding_armed` → `binding_expired` → `tombstoned`、`launch_observed` なし）。
- 同時刻に別 worktree（11:48:46、11:48:55）では成功した。
- UI 上は当該 worktree に「No sessions or workflows」と表示されていた。
- 残留プロセス 32 個を手動 kill した結果、Releash が exit を検知して 13:23:02 に該当 session 群が `paused` へ遷移し、枠が解放されて復旧した。

# Scope / Non-goals

変更する対象:

- workflow の session node が起動した AgentSession について、node execution が終端した時点で provider プロセスを終了し terminal surface の枠を解放する責務。
- 停止した workflow node session を後から復帰できる状態に保つこと。
- 停止した AgentSession の resume 可否を、provider session identity の確定状態を含めて利用者へ示すこと。
- provider Stop の canonical workflow facts 確定後に発生する provider lifecycle commit 失敗を、受理済みの Stop を覆さない post-commit 失敗として扱うこと。

変更しない対象:

- `per_worktree_cap` / `max_panes_total` の値。
- workflow に紐づかない手動作成 session のライフサイクル。
- execution が終端した時点、および execution をアーカイブした時点での追加の停止責務。
- 本変更より前に既に残留しているプロセスの回収、および起動時 reconciliation による回収。
- 上限到達時の失敗理由の保持と表示（#1653）、`[object Object]` 表示（#1652）。
- command node が起動するプロセスのライフサイクル。
- canonical workflow facts 確定後に失敗した provider lifecycle commit の retry、および診断情報の永続化。

# Requirements

- R-001: session node execution が終端状態（`Succeeded` / `Failed` / `Aborted`）へ遷移した時点で、その node execution が起動した provider プロセスは終了し、対応する terminal surface は worktree の枠を占有しなくなる。
- R-002: R-001 で停止され、`provider_session_id` が確定している AgentSession は、利用者が後から開いたときに provider の会話を引き継いで復帰できる。terminal checkpoint と `provider_session_id` は破棄されない。
- R-003: node execution が active（`Running` / `Paused` / `WaitingApproval`）である間は、その node の provider プロセスを停止しない。execution stop 後の resume と、承認待ちからの再指示は現行どおり行える。
- R-004: 同一 worktree で session node を含む workflow を繰り返し実行しても、終端済み node execution の session が `per_worktree_cap` を占有せず、`WorktreeCapReached` に起因する `TerminalUnavailable` で node activation および手動 session 作成が失敗しない。
- R-005: R-001 の停止に失敗しても、node execution の終端という確定済みの事実は覆らない。停止失敗を理由に Submit / 承認 / abort が失敗として利用者へ返らない。
- R-006: provider Stop の canonical workflow facts が確定した後に provider lifecycle の記録が失敗しても、その失敗は post-commit 失敗として扱い、Stop の受理を覆してはならない。
- R-007: 停止された AgentSession が `Paused` でも `provider_session_id` が未確定なら、利用者に Resume を提示してはならない。AgentSession と terminal checkpoint は保持し、停止状態は確認できなければならない。

# Assumptions / Open Questions

Assumptions（利用者が明示的に受け入れた前提）:

- 本文書における「停止」は、provider プロセスを終了して terminal surface を registry から解放し、terminal checkpoint と `provider_session_id` は保持することを指す。プロセスの凍結と復元ではなく、復帰は provider CLI の再開機能によるプロセス再起動である。
- 停止の時点は node execution の終端時点とする。execution の終端時点およびアーカイブ時点は採用しない。

Open Questions: なし。
