# ユースケース層 規約

## 原則

- **アプリケーション固有の業務手順**を表現する
- ドメイン層の trait のみに依存（具体実装は知らない）
- 外部依存禁止（`tauri`, `git2` 等を直接 `use` しない）
- CQRS に従い、Command と Query を別ファイルで分離

## ディレクトリ構造

```
src-tauri/src/usecase/
├── mod.rs
├── <domain>_usecase.rs           # Command 側
├── <domain>_query_service.rs     # Query 側
└── <domain>_dto.rs               # 入出力 DTO
```

ドメインが大きい場合はサブディレクトリ化する：

```
src-tauri/src/usecase/<domain>/
├── mod.rs
├── usecase.rs
├── query_service.rs
└── dto.rs
```

## Usecase（Command 側）

書き込み・状態変更を伴う操作。Repository / Gateway を組み合わせて業務手順を実行する。

```rust
// src/usecase/repository_usecase.rs
use std::sync::Arc;
use crate::domain::repository::{BranchRepository, Branch};

pub struct RepositoryUsecase {
    branch_repo: Arc<dyn BranchRepository>,
}

impl RepositoryUsecase {
    pub fn new(branch_repo: Arc<dyn BranchRepository>) -> Self {
        Self { branch_repo }
    }

    pub async fn create_branch(
        &self,
        repo_path: &Path,
        name: &str,
    ) -> Result<(), UsecaseError> {
        // バリデーション・整合性チェック
        if name.is_empty() {
            return Err(UsecaseError::InvalidInput("branch name is empty".into()));
        }
        self.branch_repo.create(repo_path, name).await
            .map_err(UsecaseError::from)
    }
}
```

## QueryService（Query 側）

読み込み専用操作。表示向けに整形した DTO を返す。

```rust
// src/usecase/repository_query_service.rs
pub struct RepositoryQueryService {
    branch_repo: Arc<dyn BranchRepository>,
}

impl RepositoryQueryService {
    pub async fn list_branches(
        &self,
        repo_path: &Path,
    ) -> Result<Vec<BranchListItemDto>, UsecaseError> {
        let branches = self.branch_repo.list(repo_path).await?;
        Ok(branches.into_iter().map(BranchListItemDto::from).collect())
    }
}
```

## DTO

- ユースケースの入出力に使う構造体
- `serde::{Serialize, Deserialize}` を実装してフロントへの返却に流用できる
- ドメイン型 ↔ DTO の変換ロジックを `<domain>_dto.rs` に集約

```rust
// src/usecase/repository_dto.rs
use serde::Serialize;
use crate::domain::repository::Branch;

#[derive(Debug, Serialize)]
pub struct BranchListItemDto {
    pub name: String,
    pub is_current: bool,
    pub upstream: Option<String>,
}

impl From<Branch> for BranchListItemDto {
    fn from(b: Branch) -> Self {
        Self {
            name: b.name,
            is_current: b.is_head,
            upstream: b.upstream,
        }
    }
}
```

## DI への組み込み

ユースケースの構造体は `lib.rs` の配線時に `Arc<dyn Trait>` で受け取れるよう、trait を切るかは判断する。

- **trait を切る**: 複数の実装が想定される、テストでモック差し替えしたい
- **構造体のまま**: 単一実装、シンプルな手続きで十分

迷ったら **trait を切らず構造体を直接 `AppState` に持たせる**ことから始めて、必要が出たら trait 化する。

## エラー型

- ユースケース固有のエラーは `UsecaseError`（thiserror）として定義
- ドメインエラーから `#[from]` で変換
- adaptor 層で `AppError` に集約される
