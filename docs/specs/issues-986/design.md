# Design

対象 Issue: #986「`notion` integration のロジックをクリーンアーキテクチャ構成へ移行する」

本書は `requirements.md` / `behavior.md` を入力に、`docs/architecture/` の各層規約に従って `notion` 責務を移行するための実装仕様を定める。本 Issue は外部観測可能な振る舞いを変えない純粋移行であり、本書の設計判断はすべて「振る舞い等価（behavior A1〜A4）を保ったまま、レイヤー規約準拠の配置と依存方向（内向きのみ）を満たす」ことを制約として行う。

---

## 1. 概要

現状 `src-tauri/src/notion/`（`mod.rs` / `client.rs` / `types.rs`、計約 2,083 行）に、Tauri command・HTTP client・JSON parse・filter 構築・wire 型・app-config domain との相互変換がレイヤー分離なく同居している。さらに `adaptor/gateway/app_config/`（`config_models.rs` / `repository_impl.rs`）が `crate::notion::types` を直接 import しており、app-config persistence → notion モジュールへのレイヤー逆依存が残っている。

本移行で、これらの責務を以下へ再配置する。

- `domain/notion/` — task query / task / label option / validation result / config status / Notion error 等の値オブジェクトと、Notion API への抽象（gateway trait）。外部依存（reqwest・Tauri・filesystem・app-config storage 具象型）を持たない。
- `usecase/notion/` — query tasks / fetch label options / save・get・delete config / validate config の業務手順。ドメインの抽象（trait）と app-config repository port のみに依存する。
- `adaptor/protocol/notion.rs` — command I/O 用 serde model。フロントから見える転送表現と後方互換を所有する。
- `adaptor/gateway/notion/` — Notion HTTP client の具体実装（reqwest blocking・API version/header・retry・JSON parse・filter 構築）。`NotionApiGateway` trait の実装。
- `adaptor/controller/command/notion/` — 6 つの Tauri command を、ユースケースを呼ぶ薄い入口として再配置。
- `adaptor/gateway/app_config/` — notion 永続化モデルを app_config 側が自前で所有し、`crate::notion::types` への依存を解消する。

移行完了後、`src-tauri/src/notion/` を削除し、`lib.rs` の `mod notion` を除去する。

---

## 2. 変更対象

### 新規追加

| パス | 役割 |
|---|---|
| `src-tauri/src/domain/notion/mod.rs` | サブモジュール公開・re-export |
| `src-tauri/src/domain/notion/value_objects.rs` | `NotionTaskQuery` / `NotionTask` / `NotionTaskPage` / `NotionLabelOption` / `NotionPropertyInfo` / `NotionValidationResult` / `NotionConfigStatus`（pure、serde なし） |
| `src-tauri/src/domain/notion/error.rs` | `NotionError`（`Display` 文字列を現状維持） |
| `src-tauri/src/domain/notion/gateway.rs` | `NotionApiGateway` trait |
| `src-tauri/src/usecase/notion/mod.rs` | サブモジュール公開 |
| `src-tauri/src/usecase/notion/usecase.rs` | `NotionUsecase` と query / fetch label / save / get / delete / validate の業務手順 |
| `src-tauri/src/usecase/notion/error.rs`（任意） | usecase エラー（`String` 表現を現状維持する場合は不要、後述 §6） |
| `src-tauri/src/adaptor/protocol/notion.rs` | command I/O 用 serde model + domain 変換 |
| `src-tauri/src/adaptor/gateway/notion/mod.rs` | サブモジュール公開 |
| `src-tauri/src/adaptor/gateway/notion/service_impl.rs` | `NotionApiGatewayImpl`（reqwest blocking・header/version・retry） |
| `src-tauri/src/adaptor/gateway/notion/service_models.rs` | JSON parse / filter 構築 / property 抽出ヘルパ |
| `src-tauri/src/adaptor/controller/command/notion/mod.rs` | `register()` / `COMMAND_NAMES` / `invoke_handler()` |
| `src-tauri/src/adaptor/controller/command/notion/commands.rs` | 6 command の薄い入口 |

### 変更

| パス | 変更内容 |
|---|---|
| `src-tauri/src/adaptor/gateway/app_config/config_models.rs` | `use crate::notion::types::NotionRepoConfig` を除去し、app_config 自前の永続化モデル（後述 §4.4）へ差し替え |
| `src-tauri/src/adaptor/gateway/app_config/repository_impl.rs` | `notion_to_domain` / `notion_to_model` 等を app_config 自前モデル基準に変更。テストの `use crate::notion::types::*` を差し替え |
| `src-tauri/src/adaptor/controller/command/mod.rs` | `pub(crate) mod notion;` 追加、fallback `app_handler` から `crate::notion::*` 6 行を除去、`notion::register(&mut router);` 追加 |
| `src-tauri/src/lib.rs` | `mod notion;`（14 行目）を除去。`NotionConfigRepository` / `NotionApiGatewayImpl` を `NotionUsecase` へ注入して `AppState` に配線 |
| `src-tauri/src/domain/notion/...`（必要なら `domain/mod.rs` / `usecase/mod.rs` / `adaptor/gateway/mod.rs`） | 各 `mod.rs` に `pub(crate) mod notion;` を追加 |

### 削除

| パス | 備考 |
|---|---|
| `src-tauri/src/notion/mod.rs` | command + 変換 → controller / usecase / gateway へ移設後に削除 |
| `src-tauri/src/notion/client.rs` | HTTP / parse / filter → gateway へ移設後に削除 |
| `src-tauri/src/notion/types.rs` | wire 型 → adaptor/protocol model / app_config 永続化モデル / domain VO へ分配後に削除 |

---

## 3. アーキテクチャと責務分割

依存方向（内向きのみ）:

```
controller/command/notion ─→ adaptor/protocol/notion（外部入口 model）
controller/command/notion ─┐
                           ├─→ usecase/notion ─→ domain/notion（VO・gateway trait・error）
gateway/notion (NotionApiGatewayImpl) ─→ domain/notion (trait 実装)
usecase/notion ─→ domain/app_config::NotionConfigRepository（既存 port）
gateway/app_config（notion 永続化モデルを自前所有。crate::notion へ依存しない）
```

### 3.1 domain/notion

- **値オブジェクト**（pure）: `NotionTaskQuery`, `NotionTask`, `NotionTaskPage`, `NotionLabelOption`, `NotionPropertyInfo`, `NotionValidationResult`, `NotionConfigStatus`。
  - 既存コードベースの慣習（`domain/app_config/value_objects` は serde を持たず、serde 表現は gateway/usecase 側が所有）に合わせ、domain VO は `Debug` / `Clone` / `PartialEq` のみ。serde は持たない。
- **gateway trait** `NotionApiGateway`（`domain/notion/gateway.rs`）:
  ```rust
  pub trait NotionApiGateway: Send + Sync {
      fn query_tasks(
          &self,
          config: &crate::domain::app_config::value_objects::NotionRepoConfig,
          query: &NotionTaskQuery,
      ) -> Result<NotionTaskPage, NotionError>;

      fn fetch_label_options(
          &self,
          config: &crate::domain::app_config::value_objects::NotionRepoConfig,
      ) -> Result<Vec<NotionLabelOption>, NotionError>;

      fn validate(
          &self,
          config: &crate::domain::app_config::value_objects::NotionRepoConfig,
      ) -> NotionValidationResult;
  }
  ```
  - **保存済み config の表現は app_config domain の `NotionRepoConfig`（値オブジェクト）を再利用する**（後述 §10 決定 D1）。これは domain↔domain（同一クレート・同一レイヤー）の参照であり、infra 依存ではないため規約に違反しない。
  - trait は **同期（blocking）**。reqwest blocking と現状の `spawn_blocking` 境界を保つため（後述 §10 決定 D2）。
- **error** `NotionError`（`domain/notion/error.rs`）: 現状の到達可能な variant（`RequestFailed` / `ApiError` / `ParseError`）と `Display` 文字列をそのまま維持。`std::error::Error` 実装も維持。

### 3.2 usecase/notion

- **業務手順**（`usecase/notion/usecase.rs`、いずれも同期関数）:
  - `NotionUsecase` が `Arc<dyn NotionConfigRepository>` と `Arc<dyn NotionApiGateway>` を保持し、`lib.rs` / `AppState` で 1 回だけ組み立てる。
  - `query_tasks(repo_path, query)`:
    `repo.get` → `None` なら `"Notion設定が見つかりません"` エラー、`Some` なら `api.query_tasks`。
  - `fetch_label_options(repo_path)`: 同様に config 解決後 `api.fetch_label_options`。
  - `save_config(repo_path, config)`: `repo.upsert`。
  - `get_config(repo_path)`: `repo.get`。
  - `delete_config(repo_path)`: `repo.remove`。
  - `validate_config(api_token, database_id)`: **空 token または空 database_id なら API を呼ばず `NotConfigured`（properties 空）を返す**。それ以外は `NotionRepoConfig`（default mapping）を組み立てて `api.validate`。
  - config 未保存時に「Notion API への問い合わせを行わない」こと（behavior Rule: unconfigured）と、空入力時に「問い合わせを行わない」こと（behavior Scenario Outline）は、usecase でガードすることで担保する。

### 3.3 adaptor/protocol/notion

- command I/O の serde 表現を外部入口 model として所有する。
  - `NotionTaskQueryInput`（command 引数）, `NotionTaskView`, `NotionTaskPageView`, `NotionLabelOptionView`, `NotionPropertyInfoView`, `NotionValidationResultView`, `NotionConfigStatusView`, `NotionRepoConfigView`, `PropertyMappingView`, `LabelPropertyView`。
  - command I/O の serde 属性を維持する: `NotionConfigStatusView` は `#[serde(rename_all = "snake_case")]`、`NotionLabelOptionView.option_ids` は `#[serde(default)]`、`PropertyMappingView.title` は `#[serde(default = "default_title")]`（既定 `"Name"`）、`labels` は `#[serde(default)]` の構造化 `LabelPropertyView[]`、`branch_prefix` は `#[serde(default)]`。
  - `NotionRepoConfigView` の `Debug` は `api_token` を `[REDACTED]` でマスク（behavior A4）。
  - 各 model に domain ↔ protocol model の `From`/変換関数を実装。

### 3.4 adaptor/gateway/notion

- `NotionApiGatewayImpl`（`service_impl.rs`）: `NotionApiGateway` を実装。ステートレス（`new()` は引数なし、API token から都度 client を構築）。`notification/webhook_sender_impl.rs` が `adaptor/gateway` で reqwest を直接使う先例に倣い、reqwest blocking client・`Notion-Version: 2022-06-28`・`NOTION_BASE_URL`・timeout・retry をここに閉じ込める（後述 §10 決定 D3）。
- `service_models.rs`: `serde_json::Value` ↔ domain 変換ヘルパ。現行 `client.rs` の純粋関数群を移設:
  - `build_notion_filter`（filter 構築）, `parse_query_response`, `extract_property_value`, `extract_multi_values`, `extract_properties_from_json`, `extract_property_options`, `extract_first_data_source_id`, `fetch_data_source_properties` / `fetch_database_properties` / `fetch_workspace_users`（HTTP を伴うものは `service_impl.rs`、純粋 parse は `service_models.rs` に分離）。
  - requirements の「gateway test が Notion API response parse と filter construction をカバーする」を満たすため、これらは公開（crate 内）かつ単体テスト可能な純粋関数として保つ。

### 3.5 adaptor/controller/command/notion

- `commands.rs`: 6 command の薄い入口。`tauri::State<'_, AppState>` から `Arc<NotionUsecase>` を取得し、protocol model ↔ domain の型変換後に usecase を呼ぶ。`NotionApiGatewayImpl` や repository port の具象配線は command 内では行わない。
  - blocking を伴う `query_notion_tasks` / `fetch_notion_label_options` / `save_notion_config` / `delete_notion_config` / `validate_notion_config` は現状どおり `tokio::task::spawn_blocking` で包む。
  - `get_notion_config` は現状どおり同期 command（`spawn_blocking` を使わない）。
  - protocol model ↔ domain の変換は command 境界で行い、戻り値の serde 表現を現状と一致させる。
- `mod.rs`: `COMMAND_NAMES` に 6 command 名を列挙し、`register()` で `router.register_domain(...)` を呼ぶ（`app_config` / `external_editor` の register パターンに一致）。

---

## 4. データモデル / 型

### 4.1 domain VO（pure）

`NotionTaskQuery { title_filter, label_filters: HashMap<String, Vec<String>>, cursor: Option<String>, page_size: Option<u32> }`、`NotionTask { id, title, url, labels, branch_name, created_at, last_edited_at }`、`NotionTaskPage { tasks, has_more, next_cursor }`、`NotionLabelOption { property_name, property_type, options, option_ids }`、`NotionPropertyInfo { name, property_type, options }`、`NotionValidationResult { status, properties }`、`NotionConfigStatus { NotConfigured | Configured | InvalidToken | InvalidDatabase | NetworkError }`。

フィールド構成は現行 `types.rs` と同一。serde を付けない点のみ異なる。

### 4.2 保存済み config の domain 表現

app_config domain の既存 `value_objects::{NotionRepoConfig, NotionPropertyMapping, NotionLabelProperty}` を引き続き使用する。notion domain では新規の config VO を定義しない（D1）。

### 4.3 protocol model（serde、command I/O）

§3.3 のとおり、現行 `types.rs` の serde 表現を 1:1 で踏襲。command の引数・戻り値の JSON/serialize 表現を不変に保つ唯一の所有者。

### 4.4 app_config 永続化モデル（serde、TOML 永続化）

`adaptor/gateway/app_config/config_models.rs`（または新規 `config_models/notion.rs`）に、永続化専用モデルを app_config 自前で定義する:

```rust
#[derive(Clone, Serialize, Deserialize)]
pub struct NotionRepoConfigModel { api_token, database_id, #[serde(default)] property_mapping: PropertyMappingModel }
// Debug は api_token を [REDACTED] でマスク
pub struct PropertyMappingModel { #[serde(default="default_title")] title, #[serde(default, deserialize_with="deserialize_notion_labels")] labels: Vec<LabelPropertyModel>, #[serde(default)] branch_name, #[serde(default)] branch_prefix }
pub struct LabelPropertyModel { name, property_type }
```

- `ReleashConfig.notion` は `HashMap<String, NotionRepoConfigModel>` になり、`crate::notion::types` への依存が消える。
- `deserialize_notion_labels`（保存済み config の文字列配列 → `property_type = "select"`）、`default_title`（`"Name"`）、`branch_prefix` 既定空文字、`Debug` マスクを移設する（behavior A4 / A5）。
- `repository_impl.rs` の `notion_to_domain` / `notion_to_model` はこのモデル ↔ app_config domain VO の変換に書き換える。

> 注: 同じ意味の config 型が「永続化モデル（app_config gateway）」「command I/O model（adaptor/protocol）」「domain VO（app_config domain）」の 3 表現を持つことになるが、これは各層が自層の都合（TOML 永続化 / フロント転送 / ビジネス意味）で形を所有する規約（GATEWAY.md「モデル分離」/ DOMAIN.md「Entity か DTO か」）に沿った意図的な分離である。重複は「同一責務の重複実装」（success criteria）ではなく層境界の変換である。

---

## 5. 処理フロー

### query_notion_tasks（configured / unconfigured）

```
command(spawn_blocking)
  → AppState.notion_usecase.query_tasks(repo_path, query.into())
      → repo.get(repo_path)?           // app_config port
          None  → Err("Notion設定が見つかりません")   // API 未呼び出し
          Some(config) → api.query_tasks(&config, &query)   // gateway: reqwest + parse
  → Result<NotionTaskPage> を NotionTaskPageView へ変換し返却
```

### fetch_notion_label_options

`query_notion_tasks` と同型。config 解決後 `api.fetch_label_options`。`people` 型がある場合のみ workspace users を取得する現行ロジックは gateway 内に保持。

### validate_notion_config

```
command(spawn_blocking)
  → AppState.notion_usecase.validate_config(api_token, database_id)
      api_token.is_empty() || database_id.is_empty()
          → NotionValidationResult { NotConfigured, properties: [] }   // API 未呼び出し
      else → api.validate(&config)   // HTTP status → Configured/InvalidToken/InvalidDatabase/NetworkError
  → NotionValidationResultView へ変換し返却
```

HTTP status からの status 判定（UNAUTHORIZED→InvalidToken、NOT_FOUND/BAD_REQUEST/その他非成功/parse 失敗→InvalidDatabase、送信失敗→NetworkError、成功→Configured）は gateway `validate` 実装に保持し、現行 `client::validate_config` と等価にする（behavior A3）。

### save / get / delete config

`NotionUsecase::save_config|get_config|delete_config` が `NotionConfigRepository`（`upsert` / `get` / `remove`）を呼ぶのみ。`save` は protocol model → app_config domain VO へ変換して `upsert`。`get` は domain VO → protocol model へ変換して `Option` を返す。

---

## 6. エラー処理

- **gateway → 上位**: `NotionApiGateway` のメソッドは `Result<_, NotionError>`（`validate` のみ `NotionValidationResult` 直返し、現状踏襲）。`NotionError::Display` 文字列（`リクエスト失敗:` / `API エラー:` / `パースエラー:`）を不変に保つ。
- **usecase → controller**: 現状 command は `Result<_, String>`。usecase 関数も `Result<_, String>` を返し、`NotionError` は呼び出し側（usecase または command）で `e.to_string()` 化する。`"Notion設定が見つかりません"` は usecase が文字列で返す。これにより command の戻り値エラー文字列が現状と完全一致する（behavior A1/A2、Rule: API 失敗時のエラー等価）。
  - 専用エラー型（`other/` 等）を新設するとエラー文字列表現が変わる懸念があるため、本移行では `String` 表現を維持する（純粋移行のため）。`usecase/notion/error.rs` は新設しない。
- **spawn_blocking join error**: 現状の `format!("task join error: {e}")` を command 側でそのまま維持。

---

## 7. テスト方針

TEST.md と requirements の明示要件に従い、層ごとに配置する（`#[cfg(test)] mod tests`）。

### domain/notion
- VO の不変条件（例: `NotionConfigStatus` の意味、`NotionValidationResult` 構築）に最小限のテスト。serde を持たないため snake_case serialize テストは protocol 側へ移す。

### usecase/notion（requirements 70-71 を満たす）
- mock `NotionConfigRepository` と mock `NotionApiGateway` を用意し、以下をカバー:
  - configured repo → `query_tasks` が task page を返す。
  - unconfigured repo → `query_tasks` / `fetch_label_options` が `"Notion設定が見つかりません"` を返し、**gateway が呼ばれない**（mock の呼び出し回数で検証）。
  - `save_config` / `get_config`（Some / None）/ `delete_config` が repository を介して反映される。
  - `validate_config`: 空 token / 空 db → `NotConfigured` かつ gateway 未呼び出し。空でない場合は gateway 委譲。
  - label fetch 成功。
  - API error（gateway が `Err`）→ usecase がエラーを伝播し、成功値を返さない。
### adaptor/protocol/notion
- protocol model serde テスト（`notion.rs`）: `NotionConfigStatusView` snake_case、`option_ids` default、`NotionTaskQueryInput` roundtrip、`NotionRepoConfigView` の `[REDACTED]` Debug。

### adaptor/gateway/notion（requirements: parse + filter）
- `service_models.rs` の純粋関数テストを現行 `client.rs` tests から移設:
  - `build_notion_filter`（title only / label only / multi_select / status / people / 複数値 OR・AND / 空値スキップ / 空 query）。
  - `parse_query_response`（basic / labels+branch / people / results 欠落）。
  - `extract_property_value`（title / rich_text / select / status / multi_select / number / checkbox / formula / unique_id / url / people / unknown / null）。
  - `extract_multi_values`、`extract_properties_from_json`（options 抽出含む）。

### adaptor/gateway/app_config
- 永続化モデルの後方互換テストを現行 `types.rs` tests から移設:
  - 保存済み config の `labels` 文字列配列 → `property_type = "select"`、`title` 既定 `"Name"`、`branch_prefix` 既定空文字、TOML roundtrip。
  - 既存 `config_models_tests` / `repository_impl` tests の `crate::notion::types` 参照を app_config 自前モデルへ差し替え。

### 構造不変条件（behavior 構造 Rule / CI）
- `src-tauri/src/notion/` 不在、`lib.rs` に `mod notion` 不在、`adaptor/gateway/app_config` が `crate::notion` を import しない、command 登録が新配置からなされること。これらは `cargo build` / `cargo clippy -D warnings` と grep ベースの確認、および CI（`cargo fmt --check` / `cargo clippy` / `cargo test`、フロント `pnpm lint`/`test`/`build`）で担保する。

---

## 8. リスクと代替案

- **エラー文字列の不一致リスク**: `NotionError::Display` / `"Notion設定が見つかりません"` / `task join error` を変えると behavior（API 失敗時の等価エラー）に違反する。→ 文字列を移設時に逐語コピーし、§6 のとおり `String` 表現を維持。回帰はテストで固定。
- **config serde 表現の不一致リスク**: 永続化モデルを app_config へ移す際に `deserialize_notion_labels` / 既定値 / `Debug` マスクを取りこぼすと A4/A5 違反・既存設定ファイル読み取り破壊。→ 属性を逐語移設し、後方互換テストを app_config gateway に移設して固定。
- **config 型 3 表現の重複**: 保守コスト増。→ 各層所有の規約上の分離であり許容。将来 app_config 移行（#985 系）で整理余地があるが本 Issue スコープ外。
- **blocking/async 境界**: gateway trait を async + reqwest async 化する代替もあるが、`spawn_blocking` 撤去はスレッドモデル変更を伴い純粋移行の範囲を超える。→ 同期 trait + controller `spawn_blocking` 維持（D2）。
- **gateway 配置（adaptor vs infrastructure）**: reqwest 実装を `infrastructure/notion/` に置く代替もあるが、`notification` webhook が `adaptor/gateway` で reqwest を直接使う先例があり、HTTP client が単一集約 I/O プリミティブに収まるため `adaptor/gateway/notion/` に統一（D3）。`shared/http_client.rs` は現状未整備のため本 Issue では新設せず、必要最小の client 構築を `service_impl.rs` に閉じる。
- **domain↔domain 参照（notion → app_config VO）**: notion domain が自前 config VO を持つ代替もあるが、変換層がさらに増える。→ app_config domain VO 再利用（D1）。同一レイヤー参照のため依存方向規約に違反しない。

---

## 9. 仮定

- requirements / behavior の確定仮定（A1〜A5、Assumptions）をすべて継承する。
- `docs/architecture/` の各層規約が移行先配置基準として有効（requirements Assumptions）。
- `get_notion_config` が同期 command である現状を維持する（他 5 command は `spawn_blocking`）。
- フロントエンド / リモートクライアントは backend I/F 不変のため変更不要。
- `crate::git_host`（GitHub 連携）は #985 の対象であり本移行では触れない。

## 10. 主要設計判断（確定）

- **D1**: 保存済み config の domain 表現は app_config domain の `NotionRepoConfig` VO を再利用し、notion domain には新規 config VO を作らない。
- **D2**: `NotionApiGateway` trait は同期（blocking）。reqwest blocking と現状の `spawn_blocking` 境界を controller に維持する。
- **D3**: Notion HTTP client 実装・JSON parse・filter 構築は `adaptor/gateway/notion/` に配置する（`infrastructure/notion/` は新設しない）。
- **D4**: command I/O の serde 表現は `adaptor/protocol/notion.rs` が、TOML 永続化の serde 表現は `adaptor/gateway/app_config/` が、それぞれ自層で所有する。domain VO は serde を持たない。
- **D5**: usecase / command のエラーは現状の `String` 表現を維持し、専用エラー型を新設しない。

## 11. Open Questions

なし。
