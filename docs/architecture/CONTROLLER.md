# コントローラ層 規約

## 原則

- **外部入力の受け口**として薄く保つ
- 引数のシリアライズ／デシリアライズと型変換のみ
- 業務ロジックを書かない（Usecase を呼ぶだけ。QueryService や Repository を controller から直接呼ばない）
- 2系統の入口を分離：
  - `controller/command/` — Tauri コマンド（`#[tauri::command]`）
  - `controller/handler/` — WebSocket ハンドラ

## ディレクトリ構造

```
src-tauri/src/adaptor/controller/
├── mod.rs
├── state.rs                           # AppState 構造体（DI 受け皿）
├── command/                           # Tauriコマンド
│   ├── mod.rs                         # 全ドメインの register をまとめる
│   └── <domain>/
│       ├── mod.rs                     # register(builder) 関数
│       └── <usecase>.rs               # #[tauri::command] 関数群
└── handler/                           # WebSocketハンドラ
    ├── mod.rs                         # ルーティング
    └── <domain>/
        └── <usecase>.rs
```

## AppState（DI 受け皿）

```rust
// src/adaptor/controller/state.rs
use std::sync::Arc;

pub struct AppState {
    pub repository_usecase: Arc<RepositoryUsecase>,
    pub code_usecase: Arc<CodeUsecase>,
    // ...
}
```

- `Arc<T>` または `Arc<dyn Trait>` で各 Usecase を保持する
- **QueryService は AppState に直接持たせない。** 読み取りクエリサービスは各 Usecase が内部に保持する協力者であり、`lib.rs`（composition root）で Usecase に注入する。controller は QueryService を保持・直呼びしない
- `lib.rs` で組み立てて `builder.manage(AppState { ... })` 一発

## Tauri コマンド

```rust
// src/adaptor/controller/command/repository/branch.rs
use tauri::State;
use crate::adaptor::controller::state::AppState;
use crate::other::error::AppError;

#[tauri::command]
pub async fn list_branches(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<Vec<BranchListItemDto>, AppError> {
    // controller は Usecase のみを呼ぶ。読み取りも Usecase 経由で行い、
    // Usecase が内部の QueryService に委譲する。
    state.repository_usecase.list_branches(Path::new(&repo_path)).await
        .map_err(AppError::from)
}
```

### コマンド登録（lib.rs 集約の解消）

各ドメイン配下に `register(builder) -> Builder` を用意し、`lib.rs` ではドメインごとの登録を呼ぶだけにする。

```rust
// src/adaptor/controller/command/repository/mod.rs
pub mod branch;
pub mod commit;
pub mod worktree;

pub fn register<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        branch::list_branches,
        branch::create_branch,
        commit::get_log,
        worktree::list_worktrees,
        // ...
    ])
}
```

```rust
// src/adaptor/controller/command/mod.rs
pub mod code;
pub mod repository;
pub mod workflow;

pub fn register_all<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    let builder = repository::register(builder);
    let builder = code::register(builder);
    let builder = workflow::register(builder);
    // ...
    builder
}
```

```rust
// src/lib.rs
let builder = tauri::Builder::default();
let builder = adaptor::controller::command::register_all(builder);
```

> **注意**: Tauri の `invoke_handler` は1度しか呼べない場合がある。実装時に検証が必要。複数呼び出しが許容されない場合は、各ドメインから関数リストを集めて1回でまとめる形に調整する。

## WebSocket ハンドラ

```rust
// src/adaptor/controller/handler/repository/branch.rs
use crate::adaptor::controller::state::AppState;
use crate::adaptor::protocol::branch::{ListBranchesRequest, ListBranchesResponse};

pub async fn handle_list_branches(
    state: Arc<AppState>,
    req: ListBranchesRequest,
) -> Result<ListBranchesResponse, AppError> {
    let branches = state.repository_usecase
        .list_branches(Path::new(&req.repo_path)).await?;
    Ok(ListBranchesResponse { branches })
}
```

WebSocket ルーティングは `handler/mod.rs` で `protocol/` のメッセージ型をディスパッチする。

## protocol/（DTO）

WebSocket メッセージや Tauri コマンドの引数として使う構造体は `adaptor/protocol/` に配置する。これは DTO であり、ドメイン型ではない。

```rust
// src/adaptor/protocol/branch.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct ListBranchesRequest {
    pub repo_path: String,
}

#[derive(Debug, Serialize)]
pub struct ListBranchesResponse {
    pub branches: Vec<BranchDto>,
}
```

## エラーハンドリング

- コントローラの戻り値は `Result<_, AppError>` で統一
- `AppError` は `serde::Serialize` を実装し、フロントへ構造化エラーとして返却される
- 詳細は [USECASE.md](./USECASE.md) と `other/error.rs` を参照
