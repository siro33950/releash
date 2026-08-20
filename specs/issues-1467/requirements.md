# Context

- 要求の正本: [Issue #1467](https://github.com/siro33950/releash/issues/1467)「[統一 Node モデル] worktree 出自の台帳と起動時 reconciliation」（OPEN・milestone 86 の wave 7・comment なし）。
- 設計の正本: [`specs/unified-node-model/decisions.md`](../unified-node-model/decisions.md) の §Worktree 実行コンテキスト、§Worktree 出自の台帳と突合、§永続化。
- 補助資料:
  - [milestone 86](https://github.com/siro33950/releash/milestone/86)「統一 Node モデル」— wave 順序（本 Issue は wave 7、#1466 の後）と、MS87 から継承する前提。
  - [milestone 85](https://github.com/siro33950/releash/milestone/85)「Session delegate と Node worktree 隔離」— 隔離実行環境の生成機構（branch + worktree 生成）と `worktree: shared | isolated` の意味論・宣言場所の所有者。本 Issue のスコープ外境界。
  - [#1466](https://github.com/siro33950/releash/issues/1466)（CLOSED・commit `ee86b1e91`）— 事実ログ（`node_events`）と冪等 reconciliation ループの実装。本 Issue の突合はこのループへ組み込む。
- 確定済みの背景と制約（後続の Behavior・Design が従う）:
  - worktree の出自は2種で、ライフサイクルと操作主体が異なる。① 人間が作る作業の場（root Node を植える先。人間が第一級に選択・作成する。長寿命）。② `isolated` 宣言により生まれる隔離実行環境（実行ごとに親の worktree HEAD から自動生成される ephemeral。その実行が所有する）。
  - worktree は Node が親から継承する実行コンテキストであり、木の構造ではない。②で実行された子があっても、実行木の所属は root の Worktree に固定される。
  - 台帳は事実ログに記録された worktree 関連の事実（隔離環境の生成・解放・喪失）とその導出であり、実行木の状態の一部である。永続化された別台帳（ファイル・テーブル・スナップショット）を新設しない。delegate の child の②も同じ台帳に載せ、第二の台帳を作らない。
  - 起動時の突合は #1466 の冪等 reconciliation ループの1周として実行する。復旧専用経路を別に作らない。
  - 突合はイベント追記と read model 更新だけを行い、worktree 実体・branch への削除系操作を一切持たない。
  - ①/②判定の一次根拠は台帳である。②の専用パス + branch 命名規則は、可読性と、台帳が読めない異常時のフォールバック判定のために defensively 定義する。フォールバックで一致した worktree は①に混ぜない。
  - 「1 worktree に active WorkflowExecution は1つ」制約は「shared worktree 上の実行中 Node 同士の書き込み競合の扱い」として再定義する。**再定義後も開始の拒否は維持する**（要求元が確定）。単独 Session の実行木と、同一実行木内で shared worktree を共有する複数 Node の並走は拒否の対象にしない。
  - `worktree: shared | isolated` の YAML 受理と解禁は milestone #85 のスコープであり、本 Issue では受理しない。
  - 品質ゲートは本リポジトリの既定（`cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`、`pnpm lint` / `pnpm test` / `pnpm build`、`pnpm test:integration`）。

# Outcome

- 対象者: Releash で workflow を実行し、同じ Workspace で worktree を管理する開発者。および Releash 本体の保守者。
- 現在の問題: Releash は worktree の出自を記録していない。実行が所有する隔離実行環境（②）と、人間が作った作業の場（①）を区別する根拠が無いため、②が導入されると Worktree 管理 UI の一覧に人間が作っていない worktree が混ざり、再起動後は出自をディスク上の形からしか推測できない。さらに、記録と実体がずれた場合（実体だけが消えている、所有者だけが終わっている）に何が起きるかが定義されておらず、成果が統合されていない worktree を保護する根拠も無い。
- 変更後に実現する状態: worktree の出自は事実ログに記録され、①/②の判定はその記録とその導出から復元される。起動時の突合で「実体喪失」「所有者終了済み」「記録なし」の3状態がそれぞれ定義された結果になり、②は Worktree 管理 UI の通常一覧に混ざらず、掃除候補は人間へ提示されるだけで機械的には削除されない。Releash が人間の指示なく worktree を取得する経路は存在しない。

# Current Behavior

commit `ee86b1e918e51c497df12f664d74033728e2aef1`（branch `feat/issues/1467`）の worktree で、以下をコード調査により確認した。調査範囲は、worktree の作成・列挙・削除経路（`src-tauri/src/adaptor/gateway/repository/`、`src-tauri/src/usecase/repository_usecase.rs`、`src-tauri/src/adaptor/controller/command/repository/`）、workflow の事実ログと reconciliation（`src-tauri/src/domain/workflow/`、`src-tauri/src/adaptor/gateway/workflow/`）、および worktree を一覧する frontend（`src/App.tsx`、`src/components/workspace/`）である。

## ②（隔離実行環境）は生成されず、宣言も受理されない

- `worktree` フィールドを宣言した node を含む定義は Diagnostic `WFU002` で拒否される（`src-tauri/src/domain/workflow/services/validation.rs:1114-1124`、`src-tauri/src/adaptor/gateway/workflow/diagnostics.rs:927`）。メッセージは ``node '<name>' declares `worktree`, which is not supported yet (#85)``（`validation.rs:388-393`）。
- したがって現在ディスク上に存在する worktree はすべて①であり、②を判定する必要が生じる状態が発生しない。

## 事実ログに worktree 出自の事実が無い

- `NodeFact` の語彙は14種（`src-tauri/src/domain/workflow/value_objects/node_fact.rs:44-74`）で、隔離環境の生成・解放・喪失に対応する事実は存在しない。
- worktree への参照を持つのは木の root 事実だけである（`WorkflowRootFact.worktree_path` / `SessionRootFact.worktree_path`。`node_fact.rs:97-122`）。Node 単位の worktree 参照は無い。

## 起動時 reconciliation は git の worktree 実体と突合しない

- `WorkflowRuntimeHost::reconcile_startup`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:535-662`）は木ごとに `reconcile_tree_pass`（`src-tauri/src/adaptor/gateway/workflow/fact_log.rs:610-715`）を呼ぶ。同関数は (1) attach / spawn まで記録された実行中 leaf に `process_exited`（`exit_code: None`、`failure_reason: "process lost across application restart"`）を追記し、(2) 未実行の前進を適用して leaf を起動し直す。
- 参照する worktree は木の root 事実由来の `worktree_path` だけで（`workflow_host.rs:582`）、その実体が存在するかを確認する経路は無い。`git worktree list` 相当の照会も行わない。

## Worktree 一覧は git 上の worktree / branch を無条件に全件出す

- `list_worktrees`（`src-tauri/src/adaptor/gateway/repository/worktree.rs:113-158`）は main worktree と、`validate()` に成功した linked worktree を全件返す。出自による除外は無い。frontend は起動時の初期タブ決定でこれを呼ぶ（`src/App.tsx:141-147`）。
- Workspace の一覧は branch カード（`list_branches_with_status` → `BranchCardDto`。`src-tauri/src/adaptor/gateway/repository/branch_card.rs`）であり、worktree を持つ branch には `worktree_path` が付く（`src/components/workspace/WorkspaceList.tsx:538-563`）。ここでも出自による除外は無い。
- 「掃除候補」という区分と、それを人間へ提示する経路は存在しない。

## worktree の作成・削除は人間の明示操作だけを起点とする

- 作成経路は Tauri command `create_worktree`（`src-tauri/src/adaptor/controller/command/repository/worktree.rs:55-68`）だけで、実装は `RepositoryUsecase::create_worktree`（`src-tauri/src/usecase/repository_usecase.rs:192-222`）。パスは `derive_worktree_path`（`src-tauri/src/domain/repository/value_objects/worktree_path.rs:20-26`）が「repo の親ディレクトリ / `<repo 名>-worktrees` / branch 名の `/` を `-` に置換した名前」として決める。呼び出し元は frontend の `CreateWorktreeModal` のみで、workflow 実行経路からの呼び出しは無い。
- workflow の開始は既存 worktree を受け取るだけで作成しない。`RepositoryManagedWorktreeGateway::resolve`（`src-tauri/src/adaptor/gateway/workflow/worktree_gateway.rs:87-99`）が「設定済み repository の git worktree であること」を検証し、一致しなければ `worktree_path is not a configured git worktree` で失敗する（`worktree_gateway.rs:56`）。
- 削除経路は Tauri command `remove_worktree`（`src/components/workspace/WorkspaceList.tsx:1609`）と、branch 削除時の連鎖削除（`src-tauri/src/usecase/repository_usecase.rs:86-128`）だけである。後者は削除前に `prune_invalid` を呼ぶが、対象は `validate()` に失敗した壊れた linked worktree に限られ（`src-tauri/src/adaptor/gateway/repository/worktree.rs:95-105`）、起点は人間の branch 削除操作である。`create_worktree` も同じ prune を事前掃除として呼ぶ（`worktree.rs:171`）。
- 成果が統合されているかどうかを削除の可否に反映する判定は無い。

## 同一 worktree の2つ目の workflow 実行は拒否される

- `ExecutionStore` は `by_worktree` で worktree → active execution を1対1に保ち、別 `execution_id` が来ると `WorktreeAlreadyActive { worktree_path, existing_execution_id }` を返す（`src-tauri/src/adaptor/gateway/workflow/execution_store.rs:725-742`、`:1719-1723`）。メッセージは `worktree <path> already has active execution <id>`。
- `workflow_host.rs:418-422` がこれを `WorkflowRuntimeError::AlreadyActive` へ写像し、開始要求が失敗する。
- 単独 Session の実行木はこの制約の対象外である。`reconcile_startup` は `TreeRootFact::Workflow` 以外の木を execution_store に登録せず（`workflow_host.rs:574-576`）、`AgentSessionLifecycleUsecase::open`（`src-tauri/src/usecase/agent_session/agent_session_lifecycle.rs:76-90`）にも worktree 単位の排他は無い。
- 同一実行木内で shared worktree を共有する複数 Node（fanout の子）の並走も、この制約の対象外である。

## resume 不可の明示は木単位の capability として存在する

- `WorkspaceWorkflowCapabilitiesDto`（`src-tauri/src/usecase/workflow/workspace_tree.rs:83-90`）が `can_resume` と `resume_unavailable_reason` を持つ。
- Node 単位の capability（`WorkspaceNodeCapabilitiesDto`。`workspace_tree.rs:63-67`）は `can_approve` / `can_retry` / `can_close` のみで、隔離環境の喪失を表す語は無い。

# Scope / Non-goals

## Scope

- worktree 出自（①/②）を事実ログの事実として記録し、その導出で判定できるようにすること。
- 起動時 reconciliation への台帳と worktree 実体の突合の組み込み、および3分岐（実体喪失 / 所有者終了済み / 記録なし）の結果の定義。
- ②の専用パスと branch 命名規則の defensive な定義、および台帳が読めない異常時のフォールバック判定。
- Worktree 管理 UI（worktree 一覧・branch カード一覧）から②を分離すること、および掃除候補を人間へ提示すること。
- 「1 worktree に active WorkflowExecution は1つ」制約の再定義（拒否の維持）と、その文書化。
- Releash が人間の明示操作なしに worktree を取得しないことの確認と回帰。

## Non-goals

- ②の生成機構（branch + worktree の生成）と解放操作の実装。milestone #85 が所有する。
- `worktree: shared | isolated` の YAML 受理・解禁、および Fanout ブロックでの宣言場所の確定。milestone #85 が所有する。
- delegate（親 Session の Submit による child 起動）の実装。milestone #85 が所有する。
- 掃除候補の自動削除、および worktree・branch に対する新しい削除系操作の追加。
- ①の worktree のライフサイクル（作成・削除の UI と操作）の変更。
- 「1 worktree に active WorkflowExecution は1つ」制約を警告へ緩めること。
- 実行木 UI の統一と文法正本化（milestone 86 の wave 8 = #1468）。
- 事実ログのスキーマ移行、および既存の永続データの変換。

# Requirements

- R-001: 隔離実行環境（②）と作業の場（①）の判定は、事実ログに記録された worktree 関連の事実とその導出だけを一次根拠とする。アプリを再起動しても判定結果は変わらず、ディスク上の worktree の形（存在・パス・branch 名）からの推測に判定が依存しない。
- R-002: 台帳が②として記録している worktree の実体が存在しない場合、起動時の突合でその②を所有する Node が「隔離環境喪失」として観測でき、その Node の再開は理由付きで拒否される。別の worktree で黙って再開しない。
- R-003: ②の実体が存在し、それを所有する Node の実行が終了している（実行中でも、再開待ち・承認待ちでもない）場合、またはその②が台帳上で解放済みである場合、その worktree は掃除候補として人間へ提示される。Releash はそれを自動的に削除しない。
- R-004: 台帳に記録が無く、②の専用パス・branch 命名規則にも一致しない worktree は、変更前と同じく①として Worktree 管理 UI の通常一覧に現れる。
- R-005: 台帳に記録が無いが、②の専用パス・branch 命名規則に一致する worktree は、①の通常一覧に混ぜず「台帳外・掃除候補」として提示される。
- R-006: 台帳が②として記録し、その②が解放されておらず、所有 Node の実行も終了していない worktree は、Worktree 管理 UI の通常一覧に現れない。
- R-007: Releash が人間の明示操作を起点とせずに worktree を作成・取得する経路が存在しない。
- R-008: 起動時の突合は、worktree の実体と branch に対する削除・prune・移動などの変更操作を行わない。成果が統合されていない worktree が、突合の結果として削除されない。
- R-009: 互換性要件 — ある worktree に active な workflow 実行木が既に登録されている状態で、その worktree に別の workflow 実行木を開始する要求は、その実行木が実行中 Node を持つかどうかに関わらず、変更前と同じく拒否される。単独 Session の実行木と、同一実行木内で shared worktree を共有する複数 Node の並走は、変更前と同じく拒否されない。
- R-010: 互換性要件 — `worktree` フィールドを宣言した workflow 定義は、変更前と同じく未対応として拒否される。
- R-011: 台帳の読み取りに失敗した場合でも、①/②の判定はディスク上の worktree の形からの推測へ切り替わらない。Worktree 管理 UI では、②の専用パス・branch 命名規則に一致する worktree だけを「台帳外・掃除候補」として提示し、一致しない worktree は①として扱う。起動時の突合は、台帳を読めないことを理由に隔離環境喪失を確定させず、ディスク上の形だけを根拠に Node を再開もしない。

# Assumptions / Open Questions

なし。
