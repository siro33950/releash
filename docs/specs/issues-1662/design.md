# Design

## The actual design

### Architecture

#### ルート行の表示名を決める規則の所有者

`WorkspacePublicRoot`（`src-tauri/src/domain/workspace_tree/services.rs`）が「public root 行が外へ見せる名前は owner（Workflow node）の表示名である」という規則を所有する。gateway は写像だけを行い、どの名前を出すかを選ばない。

この struct は既に同型の規則を持つ。行が外へ見せる識別子は public root Node の id ではなく owner の execution_id である（`services.rs:41-47` の `public_id`）。表示名も同じく「行が代表する実行の属性を見せる」規則であり、識別子と表示名で所有者を分けない。`docs/architecture/DOMAIN.md`「規則は domain が所有する。形は概念による」（状態を持たない概念は値オブジェクトとドメインサービスが規則を所有する）および「一つの概念に一つの表現」、`docs/architecture/GATEWAY.md`「業務判断を gateway に沈めない」に従う。

owner の表示名を名前の出所にする。owner の title は `WorkflowSummaryProjected` の適用で `WorkflowExecutionMetadataRecord.workflow_name` から入り（`src-tauri/src/adaptor/gateway/workspace_tree/repository.rs:214-221`、`src-tauri/src/domain/workspace_tree/entities/mod.rs:747`）、アーカイブ済み履歴の名前は同じ record の同じ field から入る（`src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs:239`, `:504`）。R-004 は両者が同一の事実を読むことで成立する。workflow 名を read model 側で別途組み立て直す経路は作らない。

#### 変更対象は投影であって Node の表示名ではない

`WorkspaceTreeNode.title` は変更しない。差し替えるのは Workspace ツリー read model の投影（`query_service.rs` の `project_tree`）で組み立てるルート行の表示名だけである。

Node の表示名は Node 詳細の経路でも使われる。`load_node` はルート行の要求に対して public root Node を clone して id だけ execution_id へ差し替えたものを返し（`repository.rs:145-155`）、`node_detail` がそこから詳細 DTO を作る。Node の title を書き換えると詳細側の名前も同時に変わり、R-007 を満たせない。

#### ルート行の Node 種別に依らない単一の写像

`project_tree` 内の `RootProjection` に表示名を持たせ、Fanout / Sequence / それ以外（Session・Command）の 3 つの分岐すべてが `RootProjection` から表示名を取る。`root_projections` に載らない行（ルート行以外）は従来どおり Node の title を使う。

現状は 3 つの分岐がそれぞれ独立に `node.title` を読んでいる（`query_service.rs:385-412`）。ここに個別の差し替えを足すと種別ごとの漏れが起きる。`public_id` と `workflow_capabilities` は既に `RootProjection` 経由で 3 分岐へ配られているため、表示名を同じ経路に載せることで R-002 が構造で保証される。

#### 単独 AgentSession を launch 種別で分岐しない

表示名の差し替えは public root へ一律に適用し、`workflow_execution_ids`（launch 種別）による分岐を入れない。

規則は「行はそれが代表する実行の名前を見せる」の一つである。単独 AgentSession の合成定義は definition の `name` と唯一の Node 名がともに `session` であるため（`src-tauri/src/domain/workflow/value_objects/node_fact.rs:155-192`）、一律適用でも表示名は変わらず R-005 は満たされる。launch 種別で分岐すると、domain が表現していない区別に基づく表示規則が gateway 側に生まれ、規則の所有者が二つに割れる。

#### 検証の置き場所

B-001 / B-002 / B-003 / B-006 は `project_tree` の単体テスト（gateway 層、`query_service_test.rs` に in-memory の `WorkspaceTree` を組む既存ヘルパーがある）で判定できる。手段が自明でないのは次の 2 点である。

- B-004: 実行中の行の名前とアーカイブ済み履歴の名前が同一の事実から来ることを確かめるには、ツリー投影と履歴投影の両方を同じ fold から作って比較する必要がある。gateway 層のテストで両投影を同時に取る。
- B-005: 単独 AgentSession の表示名が変わらないことは、別 module の合成定義（`node_fact.rs`）の名前の一致に依存する。gateway 層の回帰テストで、単独 session の実行木のルート行の表示名を固定する。

### Interface

外部から観測できる契約は変更しない。`WorkspaceNodeDto` / `WorkspaceSequenceDto` / `WorkspaceFanoutDto` の field 構成、`id`、`status`、`workflowCapabilities` は現状のままで、ルート行の `title` の値だけが変わる（R-008）。Tauri command の追加・削除・signature 変更はない。frontend は受け取った `title` をそのまま描画しており（`src/components/workspace/WorkspaceList.tsx:408`）、型定義（`src/types/workspace-tree.ts`）と描画の変更は不要である。

内部境界として `WorkspacePublicRoot` に「行が外へ見せる表示名」を返す公開アクセサを一つ追加する。`public_id` と同じく借用を返し、他の domain port / trait は変更しない。

### Data Model

追加・変更する永続 record はない。`project_tree` のローカル構造体 `RootProjection` に表示名の field を一つ追加する。所有者は投影関数であり、identity は現状どおり public root Node の id をキーとする map で引く。versioning は不要（永続化されない投影中の値である）。

### Database

該当なし。新しい access path は追加せず、既存の fold 経路のみを使う。

### UI/UX

Workspace ツリーの workflow 実行ルート行のラベルが Node 名から workflow 名に変わる。行の並び、階層、状態表示、操作 UI、選択挙動は変更しない。frontend の変更はない。

### Algorithm

該当なし。

### Infra

該当なし。

## Alternatives Considered

- **`WorkspaceTreeNode.title` を owner 名で生成または上書きする**: 採らない。`load_node` 経由の Node 詳細の名前も同時に変わり、R-007 を満たせない。
- **gateway が `WorkspacePublicRoot::owner()` の title を直接読み、domain へアクセサを足さない**: 採らない。行の公開表現を決める規則が、識別子は domain、表示名は gateway と二か所に分かれる。`docs/architecture/DOMAIN.md`「一つの概念に一つの表現」に反する。
- **`workflow_execution_ids` で分岐し、workflow として起動された実行のルート行にだけ差し替えを適用する**: 採らない。R-005 は合成定義の名前の一致で既に満たされ、分岐は domain が表現していない区別を gateway に持ち込むだけになる。

## Cross-cutting concerns

Workspace ツリー read model は現在 2 つの Tauri command 経路（`list_workspace_worktree_nodes`、`get_workspace_tree_selection_reconciliation`）から出ており、いずれも `project_tree` を通る。表示名の差し替えを投影に置くことで両経路が同じ名前を返し、将来 local API や別 client surface が同じ query service を使う場合も追加の対応が要らない（`AGENTS.md`「同じ backend-owned state を Tauri、local API、将来の client surface で再利用できるか」）。

## Risks

R-005 の成立は、単独 AgentSession の合成定義における definition の `name` と Node 名がともに `session` であるという、`node_fact.rs` 側の一致に依存する。この一致が将来崩れると単独 AgentSession のルート行の表示名が変わる。B-005 の回帰テストでこの表示名を固定し、崩れた場合にテストで検知できるようにする。
