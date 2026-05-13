# ドメイン層 規約

## 原則

- **外部依存禁止**: `tauri`, `git2`, `tokio`, `reqwest`, `sqlx` 等を `domain/` 配下では `use` しない
- **ビジネスロジック専念**: ドメイン固有の概念・不変条件・状態遷移を表現する
- **テスト容易性**: 純粋関数として書ける範囲で書く

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
