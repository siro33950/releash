# ゲートウェイ層 規約

## 原則

- **gateway は変換する層である。** 外部世界の都合を内側の言語へ、内側の言語を外部世界の都合へ、相互に変換する。変換していない処理は gateway ではなく infrastructure に属する（[INFRASTRUCTURE.md](./INFRASTRUCTURE.md)）
- **変換先は port の所在で決まる。** domain 層の port（Repository / Gateway trait）を実装するときはドメインの言語へ、usecase 層の port（QueryService）を実装するときはフロントの言語（DTO）へ変換する
- **ドメインの trait の具体実装**を提供する
- 外部ライブラリ（`git2`, `reqwest` 等）は gateway が直接呼んでも、infrastructure が提供する能力を使ってもよい。**どちらで呼ぶかは gateway と infrastructure を分ける基準ではない**（基準は変換しているかどうか）。ただし外部ライブラリの型・エラー・形式を port の外側（domain / usecase / controller）へ漏らさない
- CQRS に従い、Command（書き込み）と Query（読み込み）を別ファイルに分離
- **gateway は単一集約に対する純粋な I/O プリミティブを提供する**: 複数集約をまたぐオーケストレーションや操作の順序制御（業務手順）は usecase の責務であり、gateway に潰し込まない（[USECASE.md](./USECASE.md)）
- **gateway は状態機械を持たない**: 状態・ライフサイクルの表現主体は domain の集約である（[DOMAIN.md](./DOMAIN.md) モデルが実行を担う）。gateway が domain の状態を別の型で表現し直したり、domain 集約を経由せず自前の可変状態を進めたりしてはならない。gateway が状態を扱う場合は、domain の集約を保持して判断を委譲する（参照: `domain/pty_session/entities/pty_session_registry.rs` と `adaptor/gateway/pty_session/backend_impl.rs`）
- **業務判断を gateway に沈めない**: 「マージ済みか」「削除してよいか」のような判定規則は、外部ライブラリ（git2 等）を使う位置にあっても domain のサービス・集約に置き、gateway はその入力となる生データの取得に徹する

## ディレクトリ構造

```
src-tauri/src/adaptor/gateway/
├── mod.rs
├── shared/                            # 共通コンポーネント
│   ├── mod.rs
│   ├── git_client.rs                  # git2 共通ラッパー
│   ├── http_client.rs                 # reqwest 共通ラッパー
│   └── error_handling.rs              # 外部エラー → ドメインエラー変換
└── <domain>/
    ├── mod.rs
    ├── repository_impl.rs             # Repository trait の実装（Command）
    ├── query_service_impl.rs          # QueryService trait の実装（Query）
    ├── service_impl.rs                # Gateway trait の実装（外部システム通信）
    ├── command_models.rs              # 永続化用モデル + 変換
    ├── query_models.rs                # Query 専用モデル
    └── service_models.rs              # 外部システム用モデル
```

シンプルなドメインでは `repository_impl.rs` 単体でもよい。CQRS が複雑さに見合わなければ統合する。

## Repository 実装

```rust
// src/adaptor/gateway/repository/repository_impl.rs
use async_trait::async_trait;
use crate::domain::repository::{BranchRepository, Branch};
use crate::adaptor::gateway::shared::git_client::GitClient;

pub struct BranchRepositoryImpl {
    git: GitClient,
}

impl BranchRepositoryImpl {
    pub fn new(git: GitClient) -> Self {
        Self { git }
    }
}

#[async_trait]
impl BranchRepository for BranchRepositoryImpl {
    async fn list(&self, repo_path: &Path) -> Result<Vec<Branch>, DomainError> {
        let raw = self.git.list_branches(repo_path).await
            .map_err(convert_git_error)?;
        Ok(raw.into_iter().map(Branch::from).collect())
    }
    // ...
}
```

## Service 実装（イベント駆動 / Stream 返却）

ドメイン側で `gateway.rs` に `Stream` を返す trait を定義した場合、その実装は `service_impl.rs` で行う：

```rust
// src/adaptor/gateway/code/service_impl.rs
use futures::stream::{Stream, StreamExt};
use crate::domain::code::gateway::FileChangeGateway;

pub struct FileChangeGatewayImpl {
    watcher: Arc<FileWatcher>,
}

impl FileChangeGateway for FileChangeGatewayImpl {
    fn watch(&self, path: &Path) -> Pin<Box<dyn Stream<Item = FileChangeEvent> + Send>> {
        let raw = self.watcher.subscribe(path);
        Box::pin(raw.map(FileChangeEvent::from))
    }
}
```

## WebSocket の外向き送信

**サーバ → クライアントのブロードキャストもこのレイヤーで扱う**：

- 送信実装（コネクション管理、シリアライズ、送信）は `infrastructure/` に置く
- ドメイン側は `domain/<context>/gateway.rs` で送信用 trait を定義（例: `NotifyGateway::publish(...)`）
- Gateway 実装は `adaptor/gateway/<context>/service_impl.rs` で trait を実装し、infrastructure の送信実装を呼ぶ

```rust
// domain/notification/gateway.rs
pub trait NotifyGateway: Send + Sync {
    async fn publish(&self, event: NotifyEvent) -> Result<(), DomainError>;
}

// adaptor/gateway/notification/service_impl.rs
pub struct NotifyGatewayImpl {
    ws: Arc<WsBroadcaster>,    // infrastructure
    webhook: Arc<WebhookClient>, // infrastructure
}

impl NotifyGateway for NotifyGatewayImpl {
    async fn publish(&self, event: NotifyEvent) -> Result<(), DomainError> {
        // 必要なチャネルへ振り分け
    }
}
```

## モデル分離

| ファイル | 役割 |
|---|---|
| `command_models.rs` | DB / ファイル等の永続化用モデル + ドメイン型変換 |
| `query_models.rs` | Query 専用モデル（JOIN 結果、表示・集計向け） |
| `service_models.rs` | 外部 API のリクエスト / レスポンス型 + ドメイン型変換 |

ドメイン型に外部システムの詳細を漏らさない。変換はすべてこのレイヤーで行う。

**`query_models` は read model であり、domain の Entity ではない。** Query 側（`query_service_impl`）は読み取り要求に応えて、Entity を経由せずデータソースから `query_models` を直接組み立てて返す。Entity を生成する Repository を再利用して `Entity → DTO` に詰め替えてはならない——向きが逆である（read model は要求起点であって Entity 起点ではない）。1:1 写像に見える場合も例外ではない（[USECASE.md](./USECASE.md) QueryService）。

read model か Entity かの判定は「**誰の都合でその形が決まっているか**」で行う。表示・転送（フロントの都合）のためにその形が必要なら read model（`query_models` / DTO）であり、domain に置かない（[DOMAIN.md](./DOMAIN.md)「Entity か DTO か」）。

## エラー変換

外部システムのエラーはドメインエラーに変換する。共通変換ロジックは `shared/error_handling.rs` に集約。

```rust
// src/adaptor/gateway/shared/error_handling.rs
pub fn convert_git_error(err: git2::Error) -> DomainError {
    match err.code() {
        git2::ErrorCode::NotFound => DomainError::NotFound,
        _ => DomainError::External(err.to_string()),
    }
}
```

## ファイルサイズの目安

- `repository_impl.rs`: 200〜800行
- `query_service_impl.rs`: 200〜700行
- `service_impl.rs`: 100〜500行
- `command_models.rs` / `query_models.rs`: 100〜600行
- `shared/`: 各ファイル200〜600行
