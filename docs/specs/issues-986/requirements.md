# Requirements

## Type

リファクタリング（クリーンアーキテクチャ移行）

## Goal

`notion` integration（Notion task query、label option fetch、Notion 設定の save/get/delete/validate）のロジックを、`docs/architecture/` のクリーンアーキテクチャ規約に従ったレイヤー構成へ移行する。

完了時には、これらの責務がドメイン層・ユースケース層・アダプタ層（ゲートウェイ／コントローラ）の各規約に沿って配置され、reqwest（HTTP）・Tauri・filesystem・app-config storage の具象型への依存がゲートウェイ／インフラ層に閉じ込められた状態になる。あわせて旧構成の `src-tauri/src/notion/` モジュールが除去され、これらの責務がレイヤー規約に違反せず単一クレート内で再利用可能になっていることを成功とする。本移行を通じて、フロントエンド・リモートクライアントから見える機能の振る舞いは一切変わらない。

## Background

現状、Notion 関連のロジックは `src-tauri/src/notion/`（`mod.rs`、`client.rs`、`types.rs`）に、レイヤー分離のない構成で実装されている。

- `mod.rs` に Tauri command（`query_notion_tasks`、`fetch_notion_label_options`、`save_notion_config`、`get_notion_config`、`delete_notion_config`、`validate_notion_config`）と、app-config domain 値オブジェクトとの相互変換が混在している。
- `client.rs`（約 1,500 行）に Notion HTTP client、API JSON parse、フィルタ構築、blocking 処理が同居している。
- `types.rs` に wire 用の型定義（`NotionRepoConfig`、`PropertyMapping`、`LabelProperty` 等）が置かれ、これが `adaptor/gateway/app_config/`（`repository_impl.rs`、`config_models.rs`）から `crate::notion::types` として直接 import されており、app-config persistence が `notion` モジュールに逆依存している。

この状態には以下の課題がある。

- ドメインロジックが reqwest・Tauri・app-config storage に直接依存しており、CLI 等の別エントリポイントから再利用できない。
- ビジネスルール（property mapping、validation、filter 構築）とインフラ詳細（HTTP、JSON parse）が同一モジュールにあるため、ドメイン単位のテストが書きづらい。
- `adaptor/gateway/app_config/` が `crate::notion::types` に依存しており、`notion` モジュール削除を妨げるレイヤー逆依存が残っている。

リポジトリ全体で進行中のクリーンアーキテクチャ移行（ミルストーン「[12] クリーンアーキテクチャ移行」）の一環として、`notion` 責務を規約準拠の構成へ移行し、上記課題を解消する。本 ISSUE（#986）は独立して着手可能で、先行依存はなく、#878（final sweep）を blocks する。app-config 移行（#985 の GitHub integration、#1130/#1131 の WebSocket protocol/server 移行）とは並列可能。

## Users / Actors

- Releash デスクトップアプリを利用するエンドユーザー（Notion task 連携 UI の利用者）
- リモートクライアント（モバイル／タブレット）から WebSocket 経由で機能を利用する利用者
- `notion` 責務を将来再利用する別エントリポイント（CLI 等）
- 本コードベースを保守する開発者

## Scope

- ISSUE #986 が定める責務（Notion task query、task page/task summary、label option、property mapping、validation、Notion config の save/get/delete/validate）のロジックを、クリーンアーキテクチャ規約に従ったレイヤー構成へ移行する。
- これらの責務に対応するドメイン層（`domain/notion/`）を新構成で実装する。
  - task query、task page / task summary、label option、property mapping、validation result、Notion error / value object。
  - reqwest、Tauri、filesystem、app-config storage 具象型に依存しない。
- これらの責務のユースケース層（`usecase/notion/`）を新構成で実装する。
  - query tasks、fetch label options、save/get/delete config、validate config。
  - 保存済み config は app-config repository port 経由で扱う。
- Notion HTTP client 実装・API JSON parse・blocking/async 境界・HTTP header/API version を、ドメインが定義する抽象の実装としてゲートウェイ／インフラ層（`adaptor/gateway/notion/` または `infrastructure/notion/`）へ配置する。
- 対象 Tauri command を、ユースケースを呼び出す薄い入口として `adaptor/controller/command/notion/` へ再配置する。
- `adaptor/gateway/app_config/` の `crate::notion::types` への依存を解消する。shared config model は app_config 側に置くか、notion domain/usecase 型へ mapping する。
- ドメイン層・ユースケース層・ゲートウェイ層に対するテストを追加する。
- 移行完了後、旧構成 `src-tauri/src/notion/` を除去し、`lib.rs` の `mod notion` を削除する。
- 本責務の Tauri command 登録を、`adaptor/controller/command/mod.rs` の新配置（DI 配線）に整合させる。

## Non-goals

- 外部から観測可能な機能の振る舞い（Tauri command の引数・戻り値・エラー表現、WebSocket メッセージ形式、UI 上の操作結果）の変更。
- `app_config` ドメイン移行そのもの。本移行が扱うのは、`app_config` 側に残る Notion 参照（`crate::notion::types` への依存）の解消に必要な範囲に限る。
- GitHub issue/PR integration の移行（#985）。
- WebSocket protocol / server 移行（#1130 / #1131）。
- dead code sweep（#878）。
- Notion 連携機能そのものの仕様変更・機能追加（新しい Notion API 操作の追加等）。
- フロントエンド／リモートクライアント側のコード変更（バックエンド I/F が不変であるため不要）。

## Requirements

- `notion` 責務（task query、task page/summary、label option、property mapping、validation、config の save/get/delete/validate）が、`docs/architecture/` の各層規約に従って配置されること。
- ドメイン層（`domain/notion/`）が reqwest・Tauri・filesystem・app-config storage 具象型を直接参照しないこと。
- ユースケース層（`usecase/notion/`）がドメイン層の抽象（trait）のみに依存し、具体的な外部リソース実装に依存しないこと。保存済み config は app-config repository port 経由で扱うこと。
- Notion API への HTTP アクセス・JSON parse・filter 構築・blocking/async 境界・HTTP header/API version が、ドメインが定義する抽象の実装としてゲートウェイ／インフラ層に閉じ込められること。
- 対象 Tauri command が、ユースケースを呼び出す薄い入口に徹すること（ビジネスロジックを持たないこと）。
- `adaptor/gateway/app_config/`（`repository_impl.rs`、`config_models.rs` 等）が `crate::notion::types` を import しないこと。
- 移行対象責務について、ドメイン層・ユースケース層・ゲートウェイ層のテストが存在すること。
  - usecase test が configured/unconfigured repo、save/get/delete config、query validation、label fetch、invalid token/API error をカバーすること。
  - gateway test が Notion API response parse と filter construction をカバーすること。
- 移行後、旧構成 `src-tauri/src/notion/` が除去され、同一責務の重複実装が残らないこと。
- 移行前後で、各 Tauri command の引数・戻り値・エラー表現が等価であること。

## Constraints

- 外部から観測可能な振る舞い（Tauri command I/F、WebSocket メッセージ形式、Notion 連携の結果）を一切変えない純粋移行とすること。
- レイヤー間の依存方向が規約（`infrastructure → adaptor（controller / gateway / presenter）→ usecase → domain`、依存は内向きのみ）に従い、逆方向の依存を生まないこと。特に `app_config` ゲートウェイから `notion` モジュールへの逆依存を残さないこと。
- `app_config` ドメインへの影響は、Notion 参照解消に必要な範囲（shared config model の配置／mapping）に限定し、`app_config` ドメインの責務そのものを移行しないこと。
- API token を含む値オブジェクトの Debug 出力は、現状の `[REDACTED]` 相当のマスキングを移行後も維持すること。
- 既存の CI 品質チェック（フロントエンド lint／test／build、Rust fmt／clippy／test）が移行後も通過すること。

## Success Criteria

- `notion` 責務が新アーキテクチャの各層に規約準拠で配置され、依存方向の規約に違反していない。
- ドメイン層が外部リソース（reqwest・Tauri・filesystem・app-config storage）を直接参照していない。
- 移行対象責務のドメイン層・ユースケース層・ゲートウェイ層にテストが存在し、通過する。
- `src-tauri/src/notion/` が削除されている。
- `lib.rs` に `mod notion` が残っていない。
- `adaptor/controller/command/mod.rs` が `crate::notion::*` を直接登録していない。
- `adaptor/gateway/app_config` が `crate::notion::types` を import していない。
- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る。

## Assumptions

- 本 ISSUE はミルストーン「[12] クリーンアーキテクチャ移行」の Implementation ISSUE であり、`docs/architecture/` 配下の各層規約（DOMAIN.md / USECASE.md / CONTROLLER.md / GATEWAY.md / TEST.md）が移行先の配置基準として有効であると仮定する。
- ゲートウェイ層と Notion HTTP client 実装の最終配置（`adaptor/gateway/notion/` か `infrastructure/notion/` か）、および shared config model を `app_config` 側に置くか notion 型へ mapping するかは、要求レベルでは「依存方向の規約を満たすこと」のみを定め、具体的な配置・分割方針は design.md（実装仕様）で確定すると仮定する。
- 現状の `validate_notion_config` の挙動（空 token/database_id を `NotConfigured` として扱う等）は仕様であり、移行後も等価に保持すると仮定する。

## Open Questions

なし。
