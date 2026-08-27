# Context

## 入力文書

- 要求の正本: [Issue #1702](https://github.com/siro33950/releash/issues/1702)「fix(workspace): fanout 子の Workspace node id が外側ループの周回間で衝突し tree 読み出しが corrupt_stored_state で全滅する」（state: OPEN、label なし、milestone なし、2026-08-27 作成）
- 調査で参照した実装
  - `src-tauri/src/domain/workspace_tree/entities/mod.rs` — semantic key 導出（`fanout_child_occurrence_key`、`fanout_dynamic_child_occurrence_key`、`sequence_child_occurrence_key`、`fanout_branch_occurrence_key`、`workflow_occurrence`）と `validate` による不変条件検査、`apply_node_started` の分岐
  - `src-tauri/src/domain/workspace_tree/projection.rs` — fact 列からの全量 projection（`runtime_snapshot_nodes`）
  - `src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs` — fanout 子 NodeExecution の開始。子の kind に関わらず `ExecutionParentRef::fanout_child` を親参照にする
  - `src-tauri/src/adaptor/gateway/workspace_tree/repository.rs` / `query_service.rs` — 読み出し経路と error 分類
  - `src-tauri/src/domain/workspace_tree/value_objects/mod.rs:276`、`src-tauri/src/domain/workflow/error.rs:28`、`src-tauri/src/adaptor/controller/api/error.rs:85` — error 文言と表出
  - `src/components/panels/NodeContentView/NodeContentView.tsx:92` — 読み出し失敗時の UI 表示
  - `workflows/examples/full-cycle-development.yml` — 後方辺で fanout を再通過する定義例
- 語彙は `docs/glossary/DOMAIN.md` を正とする。

## 確定済みの背景と制約

- Workspace tree は永続化された read model ではなく、fact log を毎回 fold して導出する projection である。永続化される event に Workspace node id を保持する列は存在しない。
- Workspace node id は semantic key の SHA-256 先頭 16 byte から導出する。Session / Command は `node-w-<execution_id>-<hash>`、Fanout / Sequence は `branch-<hash>`。
- `WorkspaceTree::validate` は Workspace node id の一意性を不変条件として持つ。違反すると `WorkspaceTreeError::DuplicateNode` → `LocalEventQueryError::Corrupt` → `WorkflowError::CorruptStoredState` と伝播し、local API では error code `corrupt_stored_state` として表出する。
- Workspace tree の読み出しは、対象 Worktree に属する全実行木の node を 1 つの `WorkspaceTree` へ集めてから検査する。1 本の実行木が不変条件へ当たると、その Worktree の Workspace tree 読み出し全体が失敗する。
- Issue が原因として示すのは、fanout 子の semantic key に fanout インスタンスの同一性（親 NodeExecution の id）が含まれないことである。fact log 自体は健全であり、同じ fact log から正しく再構築できる、というのが Issue の判断である。

# Outcome

- 対象者: Releash で後方辺を持つ workflow を実行し、その進行を Workspace tree で観測・監督する開発者。
- 現在の問題: 後方辺で fanout を 2 回以上通過すると、その Worktree の Workspace tree 読み出しが `corrupt_stored_state` で拒否され、UI が全 Node で「Node unavailable」になる。WorkflowExecution は running のまま進行するため、待機中の Node へ介入できないまま観測も失う。
- 変更後に実現する状態: 同じ fanout を周回をまたいで通過しても Workspace tree が読み出せ、各周回の fanout 子が別の Node として観測でき、待機中の Node へ UI から介入できる。既に衝突を含む fact log も、データ移行なしに読み出せる。

# Current Behavior

## 再現手順

1. 後方辺で合成子を再入し、その配下に fanout を含む workflow を実行する。リポジトリ内の該当形状は `workflows/examples/full-cycle-development.yml` の `review` sequence（`review_scan` → `fix_round` → `next: review_scan`）であり、`review_scan` 配下の `full_review_fanout` を周回ごとに通過する。
2. 1 周目の fanout 子が完了し、後方辺で 2 周目に入って同じ fanout を再通過する。
3. その Worktree の Workspace tree を読み出す。

## 実際の出力

- ログに次が繰り返し出る。

  ```
  Workspace indexed record invariant failure [<correlation_id>]: duplicate Workspace node: node-w-<execution_id>-<hash>
  ```

- Workspace tree と Node 詳細の読み出しが `corrupt_stored_state: store corrupt (correlation_id=...)` で拒否される。
- UI は全 Node で「Node unavailable」になる。WorkflowExecution は running のまま継続する。

## 報告された実例

Issue 記載の実例は execution `97c31282-c12a-4163-b6f8-6735b78c73cf`（worktree `feat-issues-1696`、workflow `dev-cycle`）で、衝突した id は `node-w-97c31282c12a4163b6f86735b78c73cf-f429f364d6d4198efba29a00888580ee` である。この hash は `sha256("fanout-child\0review_fanout\0None\0" + "0" + "\0review_acceptance_opus")` の先頭 16 byte と一致することを本作成時に再計算して確認した。この文字列は `fanout_child_occurrence_key` が `parent_occurrence == 0 && occurrence == 0` のときに生成する短縮形と一致する。すなわち 1 周目と 2 周目の同じ fanout 子が同一 key を生成している。

## 実装上の現在の挙動

- fanout 子の semantic key は fanout の「名前」しか持たず、fanout インスタンスの NodeExecution id を含まない。`fanout_child_occurrence_key` は `fanout-child\0<fanout名>\0<item_index:?>\0<child_index>\0<子名>`（`parent_occurrence` / `occurrence` が 0 でない場合は各 suffix 付き）を返し、`fanout_dynamic_child_occurrence_key` は `item_index` を `None` として同じ関数へ委譲する。
- key に載る `parent_occurrence` は `workflow_occurrence` が、`occurrence` は同名の既存 Node 数が与える。いずれも「その周回に新しく作られた親の tree node の配下」で数えるため、周回ごとに 0 へ戻る。
- 対照として sequence 子の key は `scope:<parent_node_execution_id>:node:<名>:occurrence:<n>` で親の NodeExecution id を含むため、周回をまたいでも衝突しない。fanout 子の key 系列だけが親の同一性を欠いている。
- fanout そのものの分岐 node は、合成子の配下にある場合は sequence 子として key が決まるため、周回ごとに別 node になる。子だけが衝突する。
- fanout 子が Session / Command の場合の Workspace node id は `node-w-<execution_id>-<hash>` で execution_id を含むが、fanout 子が Sequence / Fanout の場合は `branch-<hash>` で execution_id を含まない。後者の semantic key も fanout の名前と位置カウンタしか持たないため、同じ Worktree で同じ workflow を複数回実行した場合にも同一の id を生成する。
- fact log は健全であり、衝突は projection の識別子導出でのみ発生する。

# Scope / Non-goals

## 変更するもの

- fanout 子 Node の Workspace node id 導出。item が定義済みの fanout と実行時に決まる fanout の双方を対象とする。この導出規則の変更により、同じ fact log から導出される fanout 子 Node id は修正前後で一致しない。Workspace node id は永続化されず読み出しごとに導出されるため、データ移行は伴わない。
- その結果として観測される、当該条件下での Workspace tree 読み出し、Node 詳細、待機中 Node への介入経路。

## 変更しないもの

- fact log の schema、記録内容、およびデータ移行。既存の fact log をそのまま読み直す。
- sequence 子、fanout 分岐 node、Sequence 分岐 node、workflow root、単独 Session の id 導出規則。これらは衝突していない。
- `corrupt_stored_state` の分類と表出の仕組み。
- WorkflowExecution の実行挙動、後方辺の解決、ループ回数の制御。
- 読み出し失敗時の UI 表示（「Node unavailable」）そのもの。

# Requirements

- R-001: 1 回の Workspace tree 読み出しが対象とする Worktree 配下の全実行木にわたって、fanout 子 Node の Workspace node id が他のいずれの Workspace node id とも衝突しない。同じ fanout 定義が後方辺の周回をまたいで複数回実行された場合と、同じ Worktree に複数の WorkflowExecution が存在する場合を含む。item が定義済みの fanout と実行時に決まる fanout の双方に成立する。
- R-002: 後方辺で fanout を 2 回以上通過した WorkflowExecution を含む Worktree で、Workspace tree の読み出しと Node 詳細の読み出しが、Workspace node id の衝突を原因とする `corrupt_stored_state` で拒否されない。UI は当該 Worktree の Node を「Node unavailable」にしない。
- R-003: 各周回で開始された fanout 子 NodeExecution が、その周回の fanout インスタンスの配下に、それぞれ独立した Node として現れる。
- R-004: 既に衝突を含む既存の fact log（報告実例: execution `97c31282-c12a-4163-b6f8-6735b78c73cf`）が、データ移行を行わずに読み出せる。同じ fact log に対する読み出しを繰り返しても、同じ Workspace tree が導出される。
- R-005: R-002 の条件下で、待機中の Node へ UI から介入できる。承認待ち Node への応答と、Session Node への回答の経路が失われない。
- R-006: Workspace node id の衝突以外の要因で Workspace tree の不変条件を満たさない保存状態は、引き続き `corrupt_stored_state` として読み出しが拒否される。

# Assumptions / Open Questions

- Assumptions: なし。
- Open Questions: なし。
