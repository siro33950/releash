# ユースケース層 規約

## 原則

- **アプリケーション固有の業務手順**を表現する
- ドメイン層の trait のみに依存（具体実装は知らない）
- 外部依存禁止（`tauri`, `git2` 等を直接 `use` しない）
- CQRS に従い、Command（Usecase）と Query（QueryService）を別ファイルで分離する
- **QueryService は Usecase ではない。** Usecase はアプリケーション固有の業務手順（オーケストレーション）を表現する唯一の単位であり、QueryService は読み取りクエリのサービスにすぎない。「ユースケース」と呼んでよいのは Usecase のみ。QueryService を「Query 側ユースケース」等と呼んで usecase 扱いしない

> **CQRS は「Command/Query のサービス分離」であって、「Repository を read 用 / write 用の trait に分割すること」ではない。** Repository は読み書きを問わず Entity を生成・取得する単位であり、read メソッドを持つこと自体は CQRS 違反ではない。Query 専用のテストダブルが未使用の write メソッドを実装させられる程度のことは、trait 分割の理由にならない。

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

## Usecase

書き込み・状態変更を伴う操作。Repository / Gateway を組み合わせて業務手順を実行する。アプリケーション層で唯一「ユースケース」と呼べる単位であり、読み取りと書き込みを跨ぐオーケストレーション（例: 一覧取得後にそのタイミングで GC を実行する等）もここに集約する。QueryService 等の読み取り部品は Usecase から呼ぶ協力者であって、Usecase ではない。

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

**複数の集約・Repository をまたぐオーケストレーションは usecase の業務手順である。** 操作の順序制御も usecase が持つ。例: 「ブランチ削除前に、紐づく worktree を先に削除する」——これは git の機構的制約（checkout 中ブランチは削除不可）に由来する順序だが、複数集約をまたぐ手順なので usecase の責務とする。gateway は単一集約に対する純粋な I/O プリミティブ（`branch.delete` / `worktree.remove` 等）に分解し、業務手順を gateway に潰し込まない。usecase が肥大化した場合は domain サービスの導入を検討する（[DOMAIN.md](./DOMAIN.md) ドメインサービス）。

## QueryService（Query 側）

読み込み専用のクエリサービス。**Usecase ではない**（「Query 側ユースケース」ではない）。表示向けに整形した DTO を返す。

**Query 側は、集約・JOIN・表示集計を伴う読み取りでは、Entity を経由せずデータソースから read model（DTO / `query_models`）を直接組み立てる。** Entity を生成する Repository を再利用して `Entity → DTO` に詰め替えるのは、単純な 1:1 写像の読み取りに限って許容される最適化であり、集約読み取りの既定手段ではない。表示・集計向けの読み取り専用モデルは `adaptor/gateway` の `query_models`（[GATEWAY.md](./GATEWAY.md)）として QueryService 実装（`query_service_impl`）が直接構築する。

```rust
// 単純な 1:1 写像に限り Entity 経由の map を許容する
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

集約・表示集計（例: ブランチ + worktree 配置 + ahead/behind + マージ状態をまとめた一覧）では、Entity を構築して詰め替えるのではなく、`query_service_impl` がデータソースから `query_models`（read model）を直接組み立てて返す。read model は domain の Entity ではない（[DOMAIN.md](./DOMAIN.md)「Entity か DTO か」）。

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
