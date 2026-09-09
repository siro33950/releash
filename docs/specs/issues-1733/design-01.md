# Design 01

## 開始状態

初回。base は `main`、派生点は `a31937204`（#1732 取り込み後）。branch `feat/issues/1733` は派生点と同一 commit で、未コミットの変更は `docs/specs/issues-1733/` の新規文書だけである。差分の基準は `docs/specs/issues-1733/requirements.md` の Current Behavior 節が記録する実装状態で、直前の Design はなく、open Thread もない。

Current Behavior が記録する挙動の現行の所在は次のとおりである。

- `worktree` field は domain の `NodeDefinition`（`src-tauri/src/domain/workflow/value_objects/definition.rs`）に `Option<String>` として保持される。`services/validation.rs` の未解禁検査が `UnsupportedWorktreeField` を出し、`adaptor/gateway/workflow/diagnostics.rs` が `WFU002`（stage `resolve`）に写す。Lua は `adaptor/gateway/workflow/lua/mod.rs` の4 builder が `reject_unknown` で許容 field を列挙しており、`worktree` は含まれない。LuaLS stub は `lua/stubs.rs` が生成し、`completion` の `ReleashCompletionModule` と handle `r.completion.approval` が Node 共通 field の handle 方式の先例である。
- Command の cwd と `RELEASH_WORKTREE_PATH` は `workflow_host.rs` と `workflow_host/command_preparation.rs` が実行木の `worktree_path` から渡す。Session は `adaptor/gateway/agent_session/provider_agent_launch_gateway.rs` が起動 worktree を受け取り、`RELEASH_BASE_BRANCH` を `repository/git_config.rs` の `resolve_effective_base_branch` で解決する（branch 設定 `releash-base` → `releash.base` → 既定 branch の順に fall back）。
- 命名規則は `domain/workflow/value_objects/worktree_origin.rs` の `isolated_worktree_branch` / `isolated_worktree_path` / `matches_isolated_identity_rule` にあり、同じファイルに台帳 `IsolatedWorktreeLedgerSnapshot`、区分 `WorktreeManagementKind`、`IsolatedWorktreeRecoveryCause` が同居する。
- 台帳の実装は、`value_objects/node_fact.rs` の事実3種、`domain/workflow/repository.rs` の `IsolatedWorktreeLedgerRepository`、`services/worktree_reconciliation.rs`、`services/fact_replay.rs` の recovery 導出、`adaptor/gateway/workflow/worktree_ledger_repository.rs`、`usecase/repository_query_service.rs` の分類、`workspace_tree` の projection と query service の `recovery_reason`、`adaptor/protocol/workflow.rs` の `recovery_reason`、frontend の `src/hooks/useWorktreeList.ts`（`cleanup_candidates`）、`src/components/workspace/WorkspaceList.tsx`（掃除候補の節）、`src/components/panels/NodeContentView/NodeContentView.tsx`（`recoveryReason`）、`src/types/{workflow,workspace-tree,git}.ts` に分散する。関連テストは `fact_log_test.rs`、`fact_replay_test.rs`、`fact_replay_recovery_test.rs`、`node_fact_test.rs`、`workflow_host.rs` 内、`workspace_tree` の test、`agent_session_tui_acceptance.rs`、`workflow_control_plane_acceptance.rs` にある。
- worktree 実体の生成は `adaptor/gateway/repository/worktree.rs` の `WorktreeRepository::create` が既存で、`adaptor/gateway/workflow/worktree_gateway.rs` のテストが命名規則の path / branch でそれを呼んでいる。workflow 側の port は `domain/workflow/gateway.rs` の `ManagedWorktreeGateway`（path 解決）と `WorktreeInventoryGateway`（照会）だけで、生成の port はない。
- Fanout の slot は `entities/workflow_execution/mod.rs` の `start_fanout_child_instance` で slot ごとに新しい node_execution_id を採番し、attempt は slot が所有する。
- Thread の worktree は `cli/review.rs` が `--session-id` から AgentSession の `worktree_path` を、無ければ `RELEASH_WORKTREE_PATH` をそのまま使う。frontend は `src/lib/agentSessionEvents.ts` が `agent-session-changed` の payload `worktreePath` を購読側（`AgentSessionPanel.tsx`、`useWorkspaceNodeDetail.ts`、`useWorkspaceTreeNodes.ts`）へ渡す。

## 変える部分

- YAML 表面の `worktree` の受理: `WFU002` を撤廃し、`worktree: shared` / `worktree: isolated` を全4種の Node で受理する。省略は `shared`。値域外の値は load 時の Error Diagnostic にする。根拠: R-001 / B-001 / B-002。ルート: 委任。
- Lua 表面の `worktree` の受理: 4 builder に `worktree?` を足し、`r.worktree.shared` / `r.worktree.isolated` の handle だけを受理する。文字列を含む handle 以外の値は Error Diagnostic にする。生成 stub も同じ形にする。根拠: R-001 / B-002「Lua の Node に `r.worktree.*` の handle 以外の値（`"isolated"` などの文字列を含む）」、R-013 / B-021「Lua API の表に `r.worktree.shared` / `r.worktree.isolated` の handle と各 Node builder の `worktree?` field」。ルート: 委任。
- attempt 開始時の branch / worktree の生成と cwd の適用: `isolated` の NodeExecution の attempt 開始で親 worktree の HEAD から branch と worktree を生成し、Command の cwd と `RELEASH_WORKTREE_PATH`、Session の起動 worktree と `RELEASH_BASE_BRANCH` の解決元を隔離 worktree にする。attempt が進むたびに新しい branch / worktree を作る。根拠: R-002 / B-003、R-004 / B-009、R-006「worktree は repository root の内側には作られない」。ルート: 委任（生成手段、既存 gateway の利用可否、生成タイミング、Command / Session への受け渡し）。
- 継承と隔離の範囲: 親 worktree は「その Node を子として扱う実行の worktree」とし、`shared`（省略を含む）はそれを引き継ぐ。Sequence / Fanout に宣言すると合成子の実行が1つの隔離 worktree を持ち children は全員そこで動く。Fanout の children エントリに宣言すると slot ごとに独立した隔離 worktree になる。入れ子の `isolated` は直近の外側の隔離 worktree の HEAD から branch する。根拠: R-002 / B-007 / B-008、R-003 / B-004 / B-005 / B-006。ルート: 委任。
- 生成失敗の扱い: branch / worktree の生成に失敗した attempt を、Command の process 起動不能と同じ Node failure にし、children エントリの `on_failure`（`retry` は新しい branch / worktree で新 attempt、`ignore` は除外して続行、宣言なしは中断して手動 Retry）に乗せる。合成子の生成失敗も合成子の failure にする。根拠: R-002 / B-025。ルート: 委任。
- 台帳の削除: 事実 `IsolatedWorktreeCreated` / `IsolatedWorktreeReleased` / `IsolatedWorktreeLost`、`IsolatedWorktreeLedgerSnapshot`、起動時の reconciliation、Worktree 管理の `isolated_owned` / `cleanup_candidate` / `untracked_cleanup_candidate` の区分と掃除候補の提示、実体喪失の recovery reason と resume / Retry の事前拒否を、実装・read model・frontend の表示・テストから削除する。根拠: R-005「生成・解放・喪失の事実、台帳、起動時の突合は存在しない」「隔離環境喪失の recovery reason も存在しない」、B-010 / B-011。ルート: 委任（削除範囲の具体）。
- 隔離 worktree の一覧非表示: 命名規則に一致する worktree を、所有 Node の実行中・終了後・Releash 再起動後のいずれでも Worktree 管理の一覧（作業の場の一覧と、それ以外のどの節にも）に出さない。根拠: R-005 / B-010。ルート: 委任（命名規則による一覧非表示への置き換え方法）。
- 実体喪失時の再開: 実体を失った隔離 worktree で attempt を再開しようとした場合を process の起動失敗として Node failure にし、手動 Retry が新しい branch / worktree で新しい attempt を始める。根拠: R-005 / B-011。ルート: 委任。
- Artifact への `worktree` キーの合成: `isolated` な Node の Artifact に engine が `worktree: { branch, path }` を4種別で同じ形で足す。`artifact` を宣言しない `isolated` な Node も `worktree` キーだけを持つ Artifact を産出し、Sequence の統合 map に現れ、Fanout の map で `null` にならない。根拠: R-007 / B-012 / B-013 / B-014。ルート: 委任（合成方法）。
- `worktree` 予約キーの検査: `artifact` に参照した Contract の直下に `worktree` field がある定義を、その Node の `worktree` 宣言の有無に関わらず load 時の Error Diagnostic にする。根拠: R-008 / B-015「その Node に `worktree: isolated` または `worktree: shared` を宣言しても結果は同じである」。ルート: 委任（検査の配置）。
- 参照解決: 配線 `inputs`、Command の `env`、テンプレート `{{ }}`、`fanout.items` の field path から `worktree.branch` / `worktree.path` を宣言なしに解決し、合成子の map 経由（`<sequence>.<child>.worktree.branch` など）も同じ規則で解決する。`artifact` を宣言しない `isolated` な Session を供給元と段にできるようにする。根拠: R-009 / B-016 / B-017。ルート: 委任。
- NodeExecution の read model への branch / path の露出: attempt 開始時点から、UI の Node 詳細、local API の execution 取得、CLI `releash workflow status --json` で隔離 branch / path を読めるようにし、Artifact を産出せずに終わった attempt でも読めるようにする。Artifact 産出後は既存の Artifact 表示・取得経路で `worktree.branch` / `worktree.path` を読める。根拠: R-010 / B-018 / B-023。ルート: 委任（導出方法、項目名）。
- Thread の Workspace 解決: `isolated` な Node からの `releash review`（Command の `RELEASH_WORKTREE_PATH` 経由、Session の `--session-id` 経由とも）が、隔離 worktree を所有する実行木が属する Workspace の Thread を対象にするようにし、隔離 worktree の path を Workspace とする Thread 集合を作らない。frontend の `agent-session-changed` の購読側の扱いもこれに合わせる。根拠: R-014 / B-024。ルート: 委任（隔離 path または実行 ID から所有実行木を引く方法、frontend filter の扱い）。
- 正本サンプルへの `isolated` 適用: `workflows/examples/full-cycle-development.yml` の `implement_and_verify` / `fix_and_verify` に `worktree: isolated` を宣言し、`implement_all` / `fix_all` の各 slot を独立した隔離 worktree で並走させる。根拠: R-012 / B-020。ルート: 委任（文面）。
- `docs/glossary/WORKFLOW.md`: Node 共通 field の表の `worktree` 行、「予約語と未解禁 field」節の `worktree` の段落、「Contract / schemas」節の Artifact の有無の記述、「Lua API」の表、Diagnostic の `WFU002` の記述を、解禁後の値域・省略時の意味・種別ごとの隔離の範囲・Artifact の `worktree` キー・予約キーの検査・Artifact の有無の規則・Lua の handle に合わせる。根拠: R-013 / B-021。ルート: 委任（文面）。
- `docs/glossary/DOMAIN.md`: 用語表の「隔離 worktree」行と「隔離 worktree」節を、生成の規則（attempt ごと、親 worktree の HEAD から）、命名が識別の根拠であること、engine が統合を行わないこと、逐次 Node での成果が後続 Node から見えないことの記述にし、台帳・lifecycle fact・recovery fence・掃除候補と「未解禁」の記述を残さない。根拠: R-013 / B-022、R-011 / B-019。ルート: 委任（文面）。
- `docs/specs/milestone-85/design.md` §3.2: 台帳・reconciliation・事実3種を実装済みの前提として挙げる記述を、命名規則と本 Issue の生成・観測の規則に置き換える。根拠: R-013 / B-026。ルート: 委任（文面）。

## 固定するルート

固定する実装上の指定なし。

## 変えないもの

- 隔離 worktree の命名規則と配置。branch `releash/isolated/<node_execution_id>-a<attempt>`、path `<repository root の親>/<repository 名>-worktrees/.releash-isolated/<node_execution_id>-a<attempt>`。理由: #1467 で確定した規則を、台帳を削除した後の隔離 worktree の識別と所有者判定の唯一の根拠として維持すると決めた（R-006）。
- 実行木の所属。WorkflowExecution が属する worktree は root の Worktree のままで、隔離 worktree は NodeExecution の attempt の実行コンテキストであり Workspace にならない。理由: #1467 の明確化と Q-004 の決定（Workspace は engine でなく読み側の概念）。
- 既存の失敗経路。Command の process 起動不能を Node failure にし、children エントリの `on_failure` と手動 Retry で復帰する経路を、生成失敗と実体喪失の受け皿としてそのまま使う。合成子に手動 Retry を足さず、新しい中断理由や resume 経路を足さない。理由: Q-005 / Q-006 の決定。
- Node の process に root worktree の path を渡す予約環境変数を増やさない。理由: Q-004 の決定。
- builtin 8本の定義本文。理由: Q-007 の決定。Diagnostic ゼロで load できることの確認だけを行う。

## 未確定・リスク

- Session Node の AgentSession が持つ `worktree_path` の意味。現行は workspace tree の projection、`agent-session-changed` の payload、`cli/review.rs` の `--session-id` 解決が AgentSession の `worktree_path` を Workspace の identity として使う。`isolated` な Session の provider を隔離 worktree で起動しつつ（R-002）、Thread と一覧の Workspace を root のままにする（R-014 / B-024、R-005 / B-010）には、AgentSession の `worktree_path` に何を記録するかを一つに決め、それを読む全経路で同じ導出を通す必要がある。委任範囲の列挙は review CLI と frontend の filter を挙げるが、workspace tree の projection と `broadcast_state` の worktree は挙がっていない。想定が外れると B-024 または B-010 を満たせない。
- read model での branch / path の導出に要る repository root。命名規則の path は main repository の root から導く。実行木が持つのは root worktree の `worktree_path` で、root worktree が linked worktree のときは repository root と一致しない。R-010 の read 経路（UI、local API、CLI。実行木の終了後を含む）で導出するには、読み側で repository root を解決できるか、attempt 開始時に branch / path を実行木の状態に残すかのどちらかが要る。想定が外れると B-023 を満たせない。
- Session の起動失敗の受け皿（未検証）。B-011 と B-025 は、隔離 worktree の実体喪失後の resume と生成失敗で「process は起動されず Node failure になり、手動 Retry または `on_failure` が新しい attempt を始める」ことを求める。Command の process 起動不能が Node failure になることは用語集が定めるが、Session の provider 起動失敗と resume 時の起動失敗が同じ Node failure に落ちるかは現行実装で未確認である。落ちない場合は B-011 と B-025 を満たせない。
