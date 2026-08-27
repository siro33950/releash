# Design

## The actual design

### Architecture

#### fanout 子 Node の identity 導出は Workspace tree 集約が所有し続ける

Workspace node id の semantic key 導出は `src-tauri/src/domain/workspace_tree/entities/mod.rs` の `WorkspaceTree::apply_node_started` が所有する。この変更で所有者は動かさない。gateway（`adaptor/gateway/workspace_tree/`）、usecase（`usecase/workflow/workspace_tree.rs`）、controller、frontend は変更しない。Workspace tree を fact log から毎回 fold して導出する読み出しの形もそのままで、識別子の導出規則だけを差し替える。

R-005 の介入経路（承認応答、Session への回答）は `WorkspaceTreeRepository::load_node` / `node_id_for_session` が Workspace node id を node_execution_id へ解決してから実行する。この解決は Workspace tree と同じ projection を通るため、識別子衝突が消えれば経路自体に変更は要らない。

#### 主要な変更対象

- `src-tauri/src/domain/workspace_tree/entities/mod.rs` — fanout 子の semantic key 導出。`fanout_child_occurrence_key` / `fanout_dynamic_child_occurrence_key` の入力から fanout の名前と位置カウンタを外し、fanout インスタンスの NodeExecution id を anchor にする。位置カウンタを供給していた `workflow_occurrence` と、その入力だった親 tree node の node_name 参照は不要になるので取り除く。`apply_node_started` の dynamic fanout 判定は、root / nested の分岐後に sentinel を 1 回だけ参照する形へ集約する。

sequence 子、fanout / Sequence の分岐 node、workflow 直下の Node、単独 Session の key 導出と、`WorkspaceTree::validate` の不変条件は変更しない。

#### 検証手段が自明でない受入条件

- B-001 / B-002 / B-005 — 後方辺の周回を表す fact 列（同じ合成子の 2 つの NodeExecution インスタンスが、それぞれ同名 fanout のインスタンスと同名の子を持つ）を projector へ流す domain test で確認する。item が定義済み（`FanoutSlot.item_index` が `Some`）の場合と、実行時に決まる（fanout 定義の `items` が `ItemsSource::ArtifactField`）場合の双方を作る。
- B-003 / B-004 / B-008 / B-009 — 同じ形の fact log を store へ書き、`SqliteWorkspaceTreeRepository` 経由の Workspace tree 読み出しと Node 読み出しが成功することを gateway test で確認する。介入経路は node id から node_execution_id への解決が成立することで足りる。
- B-011 — 同じ workflow の 2 つの WorkflowExecution を同じ Worktree に持つ fact 列で、Worktree 単位の Workspace tree 読み出しが成功することを gateway test で確認する。fanout 子が Sequence の場合を含める。
- B-006 — データ移行を行わないため、変更前に記録された fact 列と同じ並びを再生する fixture で読み出し成功を確認する。読み出しが fact log へ書き戻さないことは、読み出し経路が fold と projection だけで構成されることで示す。
- B-007 — 同じ fact 列を 2 回 fold し、導出された tree が等価であることを確認する。`occurrence` が fold の event 順に依存するため、順序の同一性が再現性の根拠になる。
- B-010 — fanout 子 id の衝突以外の不変条件違反（親不在、Session の二重束縛など）が引き続き `LocalEventQueryError::Corrupt` から `WorkflowError::CorruptStoredState` へ伝播することを確認する。

### Interface

外部から観測できる契約は変えない。Workspace node id は不透明文字列のままで、Tauri command、local API、DTO（`WorkspaceTreeSnapshotDto` / `WorkspaceNodeDetailDto`）の形と error code は変更しない。

内部境界の変更は 1 点で、fanout 子の semantic key を導出する関数が「fanout の名前 + 親 tree scope 内での位置」ではなく「fanout インスタンスの NodeExecution id」を受け取る。既存規約から読み取れないのは key の形そのものなので、それだけを示す。

```rust
// fanout 子 1 件の semantic key
format!("fanout-child\0{parent_node_execution_id}\0{item_index:?}\0{child_index}\0{child_name}\0{occurrence}")
```

`parent_node_execution_id` は `ExecutionParentRef::parent_id`（fanout インスタンスの NodeExecution id）。`item_index` / `child_index` は `FanoutSlot` の値、`occurrence` は同じ fanout インスタンス配下に既に存在する同名 Node の数。区切りに `\0` を使う点は現行のままにする。node 名は識別子として検証されないため、`:` などを含みうる値を区切り文字に選ばない。

### Data Model

追加・変更する record は無い。fact log の schema、記録内容、既存 fact の解釈は変えない。Workspace node id は読み出しごとに導出する値で永続化しないため、versioning は要らない。`WorkspaceTreeNode` の field も変えない。

### Database

該当なし。access path は変わらない。

### UI/UX

該当なし。

### Algorithm

#### 位置カウンタでは fanout インスタンスを識別できない

現行の fanout 子 key は、fanout の名前と `workflow_occurrence` が返す位置カウンタで fanout インスタンスを表す。`workflow_occurrence` は、対象 fanout の tree node と同じ親・同じ node_name を持つ先行 sibling を数える。後方辺で合成子を再入すると、周回ごとに親（合成子インスタンスの tree node）自体が別 node になるため、どの周回でも先行 sibling は 0 件になり、カウンタは 0 に戻る。結果として、周回の違いが key に現れない。

同じ形の親子でも sequence 子は衝突しない。sequence 子の key は親の node_execution_id を anchor にしており、周回ごとに別の NodeExecution が親になるからである。fanout 子だけがこの anchor を持っていない。

#### fanout NodeExecution id を anchor にする

fanout 子の key を、fanout インスタンスの NodeExecution id を anchor とする形へ置き換える。fanout インスタンスの同一性を、位置ではなく NodeExecution id が表す。sequence 子と同じ規則になり、fanout 子だけが親の同一性を欠く状態が解消される。

単射性は次で成り立つ。

- 同じ fanout インスタンス配下では、`occurrence` が同名 Node ごとに 0 から単調に増えるため、(child_name, occurrence) が重複しない。
- 異なる fanout インスタンス配下では anchor の NodeExecution id が異なる。NodeExecution id の一意性は `validate` が `DuplicateNodeExecution` として保証している。
- 他種の Node の key とは先頭 token（`fanout-child` / `scope:` / `node` / `workflow`）で分かれる。

anchor が NodeExecution id であるため、一意性は 1 本の WorkflowExecution 内に限らず、Worktree 配下の全実行木の node を 1 つの `WorkspaceTree` へ集める読み出しでも成り立つ。R-001 が求めるのはこの範囲である。fanout 子が Sequence / Fanout の場合の id は `opaque_branch_id` が semantic key だけから作り execution_id を含まないため、execution をまたぐ一意性は anchor だけが与える。

`occurrence` は fold の event 順に依存する。fold 結果の node_executions は event 順の列であり、読み出しごとに同じ順序で再生されるため、同じ fact log からは毎回同じ key が導出される。

#### 短縮形は残さない

現行の key は `parent_occurrence == 0 && occurrence == 0` のときだけ短い形を返す分岐を持つ。anchor を変えると `parent_occurrence` が無くなり、この分岐は同じ意味に 2 通りの表記を残すだけになるため、単一の形にする。同じ fact log から導出される fanout 子 Node id が変更前後で変わることは、Requirements が変更対象の帰結として明記している。

item が実行時に決まる fanout で `item_index` を key に載せない現行の規則は変えない。この場合の子は `occurrence` で別 key になるため、anchor の変更後も単射性を損なわない。

### Infra

該当なし。

## Alternatives Considered

- **`validate` の一意性検査を緩める、または重複した id を読み出し側で採番し直す** — 採らない。真に壊れた保存状態まで受理してしまい B-010 を満たせない。重複を 1 つの Node へ統合する形にすると、周回ごとの fanout 子が別の Node として現れる R-003 / B-005 も満たせない。
- **fanout の tree node id（`branch-<hash>`）を anchor にする** — 一意性は満たすが、子の識別子が親の hash へ入れ子で依存する形になり、親の node_execution_id を anchor にする sequence 子の規則と非対称になる。得られるものが無い。
- **`workflow_occurrence` の数える範囲を実行木全体へ広げる** — カウンタが位置依存のままで、fanout インスタンスの同一性を表さない。範囲を広げても、どの周回の子かを key が持たない点は変わらない。

## Cross-cutting concerns

- full-retention / full-recompute を増やさない。Workspace node id を fact log や projection record へ保持する案は採らず、導出は現行と同じ 1 回の fold の中に閉じる。`workflow_occurrence` が行っていた node 走査が無くなるため、fanout 子 1 件あたりの走査はむしろ減る。
- 壊れた保存状態の観測は変えない。不変条件違反時の log 文言、correlation_id、`corrupt_stored_state` への写像は現行のままにする。

## Risks

- 報告実例（execution `97c31282-c12a-4163-b6f8-6735b78c73cf`）の fact log が、fanout 子 id の衝突以外の不変条件違反も含んでいる場合、この変更だけでは B-006 を満たせない。実装時に当該 fact 列を再生し、衝突以外の違反が無いことを確認する必要がある。
