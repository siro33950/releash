# ドメイン層 規約

## 原則

- **外部依存禁止**: `tauri`, `git2`, `tokio`, `reqwest`, `sqlx`, `thiserror`, `serde` 等を `domain/` 配下では `use` しない。domain 層は外部フレームワーク・永続化・転送形式から独立した、純粋なビジネス表現に留める
- **ビジネスロジック専念**: ドメイン固有の概念・不変条件・状態遷移を表現する
- **テスト容易性**: 純粋関数として書ける範囲で書く
- **配置は「どのドメインに凝集するか」で決める**: 外部リソース依存か否かは配置の基準ではない。外部リソース（git2・外部 API 等）に依存する関心でも、そのドメインに凝集すべきなら domain に置く。逆に、別ドメインの関心（例: `git_host` の PR 情報）を別ドメインの型（例: `repository` の branch）に常設してはならない

## ディレクトリ構造

```
src-tauri/src/domain/<bounded-context>/
├── mod.rs                     # 公開インターフェース
├── entities/
│   ├── mod.rs
│   └── <entity>.rs            # 1構造体1ファイル
├── value_objects/
│   ├── mod.rs
│   └── <vo>.rs                # 1値オブジェクト1ファイル
├── repository.rs              # 永続化 trait
├── gateway.rs                 # 外部リソース trait（Stream返却可）
└── services.rs                # ドメインサービス
```

## エンティティ

- 一意の識別子（多くは `Uuid` または文字列ID）を持つ
- 1構造体1ファイルを基本とする
- ビジネスロジック（メソッド）はエンティティのファイル内に置く

```rust
// src/domain/repository/entities/branch.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub upstream: Option<String>,
    pub is_head: bool,
}

impl Branch {
    pub fn is_tracking(&self) -> bool {
        self.upstream.is_some()
    }
}
```

### Entity か DTO（read model）か

型を domain に置くかは「**誰の都合でその形が決まっているか**」で判断する。

- **フロント（クライアント）の都合をドメインに漏らしてはならない。** 表示・転送のためにその形が必要な型は DTO（read model）であり、`usecase`（DTO）や `adaptor/gateway`（`query_models`）側に置く。
- フロントの都合でデータ型が必要になったら DTO を作る。domain の Entity を表示用に歪めない。
- 補足的なサイン: 振る舞い（`impl` のメソッド）・不変条件・識別子を持たず、表示・転送のためだけに生成される型は、Entity ではなく read model（DTO）の疑いが強い。ただし `impl` の有無は症状であって判定の本質ではない（本質は上記の「誰の都合か」）。

→ Query 側のモデル構築は [GATEWAY.md](./GATEWAY.md) の `query_models`、[USECASE.md](./USECASE.md) の QueryService を参照。

## モデルが実行を担う

ドメインモデルはシステムの説明ではなく、システムの振る舞いそのものである。実行がモデルを通らないなら、そこにモデルは無い。Rust で書かれた文書があるだけである。

配置規約（どこに何を置くか）はこの前提を保証しない。型が正しい場所にあっても、実行が別の経路を通れば不変条件は効かない。以下はこの前提の帰結である。

### 規則は domain が所有する。形は概念による

判断・計算・分類・検証・方針は domain にある。どう表現するかは概念の性質で決まる。

- **状態とライフサイクル（開始・遷移・終端・復旧）を持つ概念** — 集約（Entity）が状態、正当な遷移、遷移の受理条件、不変条件を所有する。
- **状態を持たない概念** — 値オブジェクトとドメインサービスが規則を所有する。

**状態が無いことは、ドメインモデルが要らないことを意味しない。** 閾値、命名規約、マージ済み判定、削除可否、通知可否 — いずれも状態を持たないが、ドメインの規則である。これらが usecase や gateway に置かれたなら、それは外部ライブラリの近くにあるからではなく、規則の置き場所を誤っている。

domain に無いまま他層が状態や規則を持ち始めたなら、その概念のドメイン境界が欠落している。`domain/<name>/` が無いことは、そのドメインが無いことを意味しない。

### 遷移は集約の操作としてのみ起きる

状態は集約の操作を通じてのみ変わる。他層が状態を直接書き換える経路を作らない。

「今この操作を受理してよいか」に答えるのは集約である。この判断を呼び出し側に置くと、判断は複製され、やがて食い違う。食い違ったときにどちらが正しいかを決める主体はいない。

### 表現できてはならない状態は、型として表現できないようにする

不変条件のうち「その中間状態は存在しない」と言えるものは、規約や注意ではなく型で閉じる。

例: 観測した事実を状態へ反映する遷移は、事実の受理と記録を一つの操作にする。「事実は受け取ったが記録していない」という中間状態が型として作れるなら、いつか作られる。

### 一つの概念に一つの表現

同じ概念を二つの場所で表現しない。名前が違っても、同じ概念なら同じである。状態については型が、状態を持たない規則については規則そのものが、一つであること。

不変条件も規則も片方にしか付かない。実行がもう片方を通れば、それは無いのと同じになる。domain に規則があり、実行は gateway 側の同じ規則を通る、という形は「二重化」ではなく「domain 側が死んでいる」ということである。

永続化・転送のための DTO は表現の複製ではない。DTO は変換だけを担い、判断を持たないからそう言える（[Entity か DTO か](#entity-か-dtoread-model-か)）。DTO に受理判定や遷移を書いた時点で、それは二つ目の表現になる。

### 他層は判断を持たない

- usecase — 何を、どの順で呼ぶか。手順であって判断ではない。
- gateway — 外界との接続。取得した生データに判定規則を当てない。domain の集約を保持して判断を委ねることはあるが、自前の状態機械を持たない。
- controller — 入口。受理判定を書かない。

これらの層に判断（受理可否・遷移・分類・計算・検証）が溜まったら、domain に置かれるべきものが置かれていないというサインである。外部ライブラリ（git2 等）を使う位置にあることは、その層に規則を置く理由にならない。生データの取得と、そこに当てる規則は別である。

### モデルは実行経路にある

domain に型や関数があることと、それが効いていることは別である。実行経路から切断された集約や規則は、モデルではなく、モデルのつもりで書かれた文書である。

あるのに効かないものは、無いものより悪い。読む者はそれをモデルだと信じ、実際の振る舞いは別の場所で決まっているからである。移行の途中で一時的にそうなる場合は、いつ・何によって接続されるかを明示する。明示できないなら、それは移行ではなく放棄である。

### 参照実装

状態を持つ概念:

- `domain/terminal_surface/entities/terminal_surface_registry.rs` — 集約が cap 判定・退避選択・予約の commit/rollback を所有し、gateway は集約を保持して判断を委ねる。
- `domain/comment/mod.rs` — 状態型・イベント・遷移適用（`ThreadAccumulator::apply`）・受理判定（`ensure_thread_open`）がすべて domain にあり、usecase は port 呼び出しの手順のみ。
- `domain/provider_lifecycle/entities/provider_lifecycle_slot.rs` — Slotごとのcurrent launch binding、bindingの失効、capability検証、signalの受理判定をdomainが持ち、usecaseはSlot単位の排他と永続化の手順のみを担う。

状態を持たない概念:

- `domain/code/services/`、`domain/code/value_objects/review.rs` — hunk 区切り・patch 生成・閾値判定という規則を domain が所有する。usecase が薄いのは、判断が domain にあるからである。

## 値オブジェクト

- 不変（immutable）、等価性は値そのものから判断
- enum で状態遷移を表現する場合は `impl` に判定メソッドを置く

```rust
// src/domain/workflow/value_objects/workflow_status.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowStatus {
    Pending,
    Running,
    Approved,
    Rejected,
    Completed,
}

impl WorkflowStatus {
    pub fn can_approve(&self) -> bool {
        matches!(self, Self::Pending | Self::Running)
    }
}
```

## port（Repository / Gateway trait）

port を domain 層に置くのは依存関係逆転のためである。domain は外側を知らないまま、必要な能力を trait として宣言し、adaptor/gateway がそれを実装する。

**domain 層に置く port は、ドメインの言語だけで書く。** 引数・戻り値・エラーに外部世界の語彙（`git2` の型、SQL の行、wire の形式、外部システム固有のコード）を出さない。

port のシグネチャに外部世界の語彙が現れたら、それは**変換が gateway で完了しておらず内側へ漏れている**というサインである。port の型付けは、変換が境界で閉じているかの判定になる。

読み取り要求の出力仕様（DTO）を返す port は、ドメインの言語をスキップするために存在するので usecase 層に置く（[USECASE.md](./USECASE.md) QueryService）。domain 層の port と混同しない。

## Repository trait

- データアクセスの抽象化
- 戻り値は `Result<_, DomainError>` 形式（ドメイン固有のエラー型）

```rust
// src/domain/repository/repository.rs
#[async_trait::async_trait]
pub trait BranchRepository: Send + Sync {
    async fn list(&self, repo_path: &Path) -> Result<Vec<Branch>, DomainError>;
    async fn current(&self, repo_path: &Path) -> Result<Option<Branch>, DomainError>;
    async fn create(&self, repo_path: &Path, name: &str) -> Result<(), DomainError>;
}
```

## Gateway trait（外部リソース）

- Repository とは別に、**外部システムとの非永続な対話**を扱う
- イベント駆動（監視、ストリーム）は **`Stream` を返す形式**で定義する

```rust
// src/domain/code/gateway.rs
use futures::stream::Stream;

pub trait FileChangeGateway: Send + Sync {
    fn watch(&self, path: &Path) -> Pin<Box<dyn Stream<Item = FileChangeEvent> + Send>>;
}
```

## ドメインサービス

- 複数エンティティにまたがるロジックを置く
- 単一エンティティで完結する場合は entity の `impl` に置く

## Aggregates パターン（任意）

エンティティのビジネスロジックが**1000行を超える**場合に検討する：

```
src/domain/<context>/aggregates/<aggregate>/
├── mod.rs                 # 構造体定義
├── constructors.rs        # 生成
├── update_status.rs       # 状態遷移
├── calculate.rs           # 計算
└── common.rs              # pub(super) のヘルパー
```

実装ファイルごとに `impl Aggregate { ... }` を分割する。

## モジュール公開インターフェース

`mod.rs` で外部向けの API を明示する：

```rust
// src/domain/repository/mod.rs
pub mod entities;
pub mod value_objects;
pub mod repository;
pub mod gateway;
pub mod services;

pub use entities::Branch;
pub use value_objects::BranchKind;
pub use repository::BranchRepository;
```

## ファイルサイズの目安

- 〜1000行: 同一ファイル
- 1000〜2000行: 分割を検討
- 2000行以上: 分割を強く推奨（Aggregates 適用候補）

行数だけでなく、責務の凝集度で判断する。
