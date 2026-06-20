# Design

本書は #1219「自前 MCP ドメイン一式の撤去」の実装設計を定義する。新機能追加・挙動変更はなく、デッドコード削除・機能廃止のみを対象とする。要求は `requirements.md`、外部から観測される振る舞いは `behavior.md` を参照する。

## 概要

CLI への全面移行で用済みとなった Releash 自前 MCP 機能（facet 1/2/3）を撤去する。

- facet 1: Releash 自前 MCP サーバ（agents → Releash の worktree / file tool 公開）
- facet 2: エージェントへの MCP 設定注入
- facet 3: MCP 設定 UI

撤去後、自前 MCP サーバは起動せず `mcp__releash__*` tool は提供されない。MCP 設定タブは消える。それ以外のデスクトップ全機能・facet 4（外部 MCP ツールの表示分類）・Notion 設定は従来どおり動作する（回帰なし）。

設計の核心は「撤去対象 MCP ドメイン（`domain/mcp` 〜 `infrastructure/external/mcp`）の物理削除」と「削除によってビルドが壊れる結合点の除去」を分離し、後者を漏れなく潰すことにある。コード調査の結果、結合点は requirements 記載分に加えて **app_config ドメイン内の MCP 設定**が存在することを確認した（後述「変更対象 C」「Open Questions」）。

## 変更対象

撤去は 3 群に分かれる。**A 群・B 群・C 群すべて確定**（C 群はユーザー合意により「徹底撤去」で確定）。

### A 群: 撤去対象 MCP ドメイン本体（物理削除・確定）

以下のディレクトリを丸ごと削除する。いずれも MCP 機能専用であり、外部からの依存は B 群・C 群の結合点に集約される。

| ディレクトリ | 役割 |
|---|---|
| `src-tauri/src/domain/mcp/` | MCP ドメインのエンティティ・値オブジェクト・サービス・gateway trait |
| `src-tauri/src/usecase/mcp/` | ライフサイクル / エージェント設定 / クエリの各ユースケース・DTO |
| `src-tauri/src/adaptor/gateway/mcp/` | MCP サーバ起動実装・エージェント設定生成実装・共有 state |
| `src-tauri/src/adaptor/controller/command/mcp/` | MCP 専用 Tauri コマンド（10 個）と `register` |
| `src-tauri/src/infrastructure/external/mcp/` | rmcp ベースの MCP サーバ実装・HMAC 認証ミドルウェア |

フロント（facet 3）:

| ファイル | 扱い |
|---|---|
| `src/components/panels/McpSettingsSection.tsx` | 削除 |
| `src/hooks/useMcpConfig.ts` | 削除 |

### B 群: A 群削除に伴う結合点の除去（確定）

A 群を削除すると、これらを参照している箇所がコンパイルエラー／lint エラーになるため除去する。

Rust:

- `src-tauri/src/lib.rs`
  - `.manage(adaptor::gateway::mcp::McpServerHandle::default())`（82 行目付近）の除去
  - 起動時の自動起動ブロック（394〜404 行目付近、`auto_start_mcp_server` を spawn する `{ ... }` ブロック全体）の除去
- `src-tauri/src/adaptor/controller/command/mod.rs`
  - `pub(crate) mod mcp;`（6 行目）の除去
  - `mcp::register(&mut router);`（215 行目）の除去
  - 144 行目の `// MCP Server` コメント（`generate_handler!` 内、現状コマンド本体は無い）の除去
  - ※ MCP コマンドは `generate_handler!` に直接列挙されておらず、`command/mcp/mod.rs` の `COMMAND_NAMES` + `register` 経由で登録されている。よって個別コマンドの `generate_handler!` からの削除は不要（A 群の物理削除で消える）。
- 各 `mod.rs` の `mod mcp;` 宣言除去:
  - `src-tauri/src/domain/mod.rs`（`pub(crate) mod mcp;`、6 行目付近）
  - `src-tauri/src/usecase/mod.rs`（9 行目付近）
  - `src-tauri/src/adaptor/gateway/mod.rs`（5 行目付近）
  - `src-tauri/src/infrastructure/external/mod.rs`（1 行目付近）
- `src-tauri/src/adaptor/controller/command/app_config/commands.rs`
  - `use crate::adaptor::gateway::mcp::McpServerGatewayImpl;`（10 行目）の除去
  - `use crate::usecase::mcp::McpLifecycleUsecase;`（13 行目）の除去
  - `restart_mcp_if_running` 関数定義（23〜26 行目）の除去
  - **理由**: この関数は撤去対象 `McpServerGatewayImpl` / `McpLifecycleUsecase` に依存しており、A 群削除でビルドエラーになる。除去は必須。
  - 呼び出し側（`update_mcp_config` 94 行目・`regenerate_mcp_token` 107 行目）は C 群で当該コマンドごと削除されるため、呼び出し行も消滅する。
- `src-tauri/Cargo.toml`
  - `rmcp` / `axum` / `tokio-util` の依存削除（69〜70 行目付近）。
  - **確認済み**: `grep -rln` の結果、3 クレートとも `src/adaptor/gateway/mcp/` と `src/infrastructure/external/mcp/` 以外では使用されていない（ws_server は axum 非依存）。A 群削除後は未使用となるため除去する。削除後 `cargo build` でビルド・`cargo tree` で未使用がないことを最終確認する。

フロント（facet 3）:

- `src/components/panels/SettingsModal.tsx`
  - `import { useMcpConfig } from "@/hooks/useMcpConfig";`（50 行目）除去
  - `import { McpSettingsSection } from "./McpSettingsSection";`（71 行目）除去
  - `SettingsSection` 型ユニオンから `| "mcp"`（314 行目）除去
  - タブ定義配列から `{ id: "mcp", label: "MCP", icon: Plug }`（331 行目）除去
  - `const mcp = useMcpConfig();`（1529 行目）除去
  - `mcp.reload();`（1544 行目、ダイアログ open 時）除去
  - `const { isDirty: mcpIsDirty, save: mcpSave } = mcp;`（1612 行目）除去
  - 保存処理内の `mcpIsDirty` チェック・`mcpSave()` 呼び出し（1636〜1637 行目）除去
  - `case "mcp": return <McpSettingsSection mcp={mcp} />;`（1742〜1743 行目）除去
  - 上記除去で未使用になる import（`Plug` アイコン等）があれば併せて除去（Biome が検出）。他タブ（Notion 等）の構成は変更しない。

### C 群: app_config ドメイン内の MCP 設定（徹底撤去・確定）

requirements / behavior には記載がないが、コード調査で app_config ドメイン内に MCP 設定（`mcp_port` / `mcp_token`）が独立して存在することを確認した。これらは撤去対象 A 群に**依存していない**（app_config 内で完結）。一方で A 群（`adaptor/gateway/mcp`）が ConfigRepository 経由でこれらを**読み取る**関係にあった。

**ユーザー合意により「徹底撤去（消す）」で確定**（Goal「自前 MCP 機能の撤去」と requirements「未使用となる参照・依存を残さない」に従い、死んだ設定値・到達不能コマンドを残さない）。以下をすべて削除する。

| ファイル | 削除する MCP 要素 |
|---|---|
| `domain/app_config/value_objects/mod.rs:16-25` | `ServerConfig.mcp_port` / `ServerConfig.mcp_token` フィールド |
| `usecase/app_config/usecase.rs:51-75` | `update_mcp_config` / `regenerate_mcp_token` メソッド |
| `usecase/app_config/query_service.rs:7, 23-29` | `get_mcp_config` メソッドと `McpConfigDto` import |
| `usecase/app_config/dto.rs:1-5` | `McpConfigDto` 型定義 |
| `adaptor/gateway/app_config/config_models.rs:48-52, 152-173, 293-308` | `McpConfig` model 型、`ServerSection.mcp_port/mcp_token` フィールド、`default_mcp_port()=19801`、`server_to_model` 内の `mcp_port`/`mcp_token` 変換行 |
| `adaptor/gateway/app_config/mod.rs:9` | re-export 一覧からの `McpConfig` 除去 |
| `adaptor/gateway/app_config/repository_impl.rs:107-130, 237-261` | `configured_secret_values` の `mcp_token` 参照、`load_or_create_config` の `mcp_token` 自動生成ブロック |
| `adaptor/controller/command/app_config/commands.rs:7, 72-109` | `get_mcp_config` / `update_mcp_config` / `regenerate_mcp_token` の 3 Tauri コマンド、import の `McpConfig` 除去 |
| `releash.toml`（ユーザー data_dir） | `[server]` セクションの `mcp_port` / `mcp_token`（コード上の生成・書き戻し停止により以後出力されない。`deny_unknown_fields` 不使用を確認済みのため既存ファイルの残存キーは無視され読み込みエラーにならない） |

加えて、この 3 コマンドの `command/app_config/mod.rs::COMMAND_NAMES`（`get_mcp_config` / `update_mcp_config` / `regenerate_mcp_token`、17〜19 行目付近）からの除去、フロント `useMcpConfig`（A 群で削除）からの呼び出し消滅により参照が完全に断たれる。B 群の `restart_mcp_if_running` 除去と相まって、`update_mcp_config` / `regenerate_mcp_token` 自体が消えるため、MCP サーバ再起動呼び出しも消滅する。

関連テスト: `usecase/app_config` / `gateway/app_config` の `#[cfg(test)]` 内に `mcp_port` / `mcp_token` / `update_mcp_config` 等を検証するケースがあれば併せて削除・修正する（実装時に grep で洗い出す）。

### 撤去対象に含めない（残す・確定）

requirements「撤去対象に含めない」に従い、以下は変更しない:

- `src-tauri/src/adaptor/controller/command/agent_session/tool_activity.rs` の `mcp__` 分類（facet 4）とそのテスト（`mcp__notion__get_page` 等）
- フロント `src/components/panels/AgentChatPanel/ActivityLog.tsx` 等の `mcp` 表示分類（facet 4 表示側）
- `src/components/panels/NotionSettingsSection.tsx`（Notion 設定）

## アーキテクチャと責務分割

本タスクは新規責務の追加を伴わない。クリーンアーキテクチャの各層から MCP ドメインを切除する作業であり、層構造そのものは変更しない（requirements Non-goals に準拠）。

撤去後の依存関係:

- A 群削除により `infrastructure → adaptor/gateway → domain ← usecase ← adaptor/controller` の MCP 縦串が消滅する。
- app_config ドメインは MCP ドメインに依存していないため、A 群削除後も独立して健全（C 群を残す場合）。
- 唯一の逆方向結合だった `adaptor/gateway/mcp → domain/app_config::ConfigRepository`（MCP サーバが mcp_port/mcp_token を読む経路）は、A 群削除で消滅する。

責務分割上の判断:

- **削除順序**: 結合点（B 群・C 群該当箇所）を先に除去 → モジュール宣言除去 → ディレクトリ物理削除 → 依存クレート削除、の順で進めると、各ステップで `cargo build` を回して切り分けやすい。ただし最終状態が同一であれば順序は本質ではない。
- **facet 4 との分離**: `mcp__` 文字列判定（tool naming convention）は MCP ドメインに一切依存しない純粋な文字列分類であり、A 群削除の影響を受けない。

## データモデルまたは型

新規型の追加・既存型のスキーマ変更はない。撤去により消滅／変化する型は以下。

- 消滅（A 群）: `McpServerHandle`、`McpConnectionInfo`、`McpSharedState`、`McpServerGatewayImpl`、`McpLifecycleUsecase` ほか MCP ドメイン内の全型。
- 消滅（C 群・確定）: `McpConfig`(model) / `McpConfigDto` / `ServerConfig.mcp_port` / `ServerConfig.mcp_token` / `ServerSection.mcp_port` / `ServerSection.mcp_token`。
  - **スキーマ影響**: `ServerSection` の `mcp_port` / `mcp_token` フィールドを削除すると `releash.toml` の `[server]` から両キーが消える。`deny_unknown_fields` は不使用であることを確認済み（`src/adaptor/gateway/app_config/` に該当属性なし）のため、既存ユーザーの toml に残存するキーは「未知フィールド」として無視され読み込みエラーにはならない。再保存時に物理的に消える。

## 処理フロー

撤去後のランタイム挙動（behavior.md の Rule に対応）:

1. **アプリ起動**: lib.rs から自動起動ブロックが消えているため、自前 MCP サーバの spawn は発生しない。`McpServerHandle` も `.manage` されない。→ behavior「自前 MCP サーバは起動しない」。
2. **エージェントセッション開始**: facet 2（MCP 設定注入）を担っていた A 群コマンドが存在しないため、`mcp__releash__*` の接続設定は注入されない。エージェントのツール一覧に `mcp__releash__*` は現れない。→ behavior「mcp__releash__* tool が見えない」。
3. **設定モーダル表示**: タブ配列から `mcp` が消え、`MCP` タブは描画されない。`Notion` 等の他タブは従来どおり。→ behavior「MCP タブが存在しない」。
4. **facet 4 のツール活動表示**: `classify_tool` の `mcp__` 判定は無改変のため、`mcp__notion__get_page` / `mcp__server__some_tool` は従来どおり `mcp` カテゴリで表示。→ behavior「facet 4 は従来どおり」。

実装手順フロー（開発者向け）:

```
1. B 群結合点の除去（lib.rs / command/mod.rs / app_config/commands.rs の restart_mcp_if_running / 各 mod.rs / SettingsModal.tsx / 各フロント参照）
2. C 群の徹底撤去（app_config の domain/usecase/gateway/controller から mcp_port/mcp_token・3 コマンド・McpConfig・McpConfigDto・トークン自動生成・シークレット参照・COMMAND_NAMES エントリを削除）
3. A 群ディレクトリ・フロント 2 ファイルの物理削除
4. Cargo.toml から rmcp / axum / tokio-util を削除
5. cargo fmt / cargo clippy -- -D warnings / cargo test
6. pnpm lint / pnpm build / pnpm test
7. cargo tree・grep で MCP 残滓（未使用 import / 文字列 "mcp__releash" / "mcp_port" / "mcp_token"）が無いことを確認
```

## エラー処理

- 撤去作業中の主たる失敗モードはコンパイルエラーであり、`cargo build` / `cargo clippy` / `pnpm build` を各ステップで実行して検出する。ランタイムのエラー処理ロジックは新設しない。
- C 群（徹底撤去）では、`McpConfigDto` / `McpConfig`(model) 削除に伴う変換関数（`server_to_model` 等）の整合を取り、`configured_secret_values` から `mcp_token` を除く。シークレットマスキング対象が 1 件減るのみで、他の秘匿値（`token` / `webhook_url`）の処理は不変。`update_mcp_config` / `regenerate_mcp_token` 自体が消えるため、設定保存後の MCP サーバ再起動経路も消滅する。

## テスト方針

新規テストは追加せず、既存テストの緑維持と回帰検証を行う（behavior.md「撤去後も成果物は健全である」）。

- **Rust**（`src-tauri/` で実行）:
  - `cargo clippy -- -D warnings`: 未使用 import / dead_code 警告ゼロ。特に B 群除去漏れ・Cargo.toml 依存削除後の未使用検出に有効。
  - `cargo test`: A 群削除に伴い MCP ドメイン内の `#[cfg(test)]` テストは消滅する（想定どおり）。app_config / agent_session（facet 4）の既存テストが緑であること。`command/mod.rs` の `tests`（`workflow_register_routes_...`）は MCP 非依存で不変。
- **フロント**（プロジェクトルートで実行）:
  - `pnpm lint`（Biome）: SettingsModal の未使用 import 除去確認。
  - `pnpm build`（TSC + Vite、メイン + リモート）: 型エラーなし。
  - `pnpm test`（Vitest）: SettingsModal 関連テストが存在する場合、MCP タブ参照が無い状態で緑。useMcpConfig / McpSettingsSection 専用テストがあれば削除する。
- **手動回帰確認の観点**（自動化対象外）:
  - 設定モーダルに MCP タブが無い／Notion タブが従来どおり。
  - アプリ起動時に MCP サーバが起動しない（ログに自動起動メッセージが出ない）。
  - facet 4 のツール活動表示が従来どおり。

テスト追加に関する補足: requirements / behavior は「振る舞い変更なし・回帰なし」を求めており、撤去機能に対する新規テストは不要。削除されるテストの有無は実装時に grep（`mcp` を含む `*.test.ts(x)` / `#[cfg(test)]`）で洗い出す。

## リスクと代替案

- **リスク 1: B 群除去漏れによるビルド破綻**。特に `app_config/commands.rs` の `restart_mcp_if_running`（requirements 未記載の結合点）。→ 本設計で明示済み。`cargo clippy -- -D warnings` で必ず検出される。
- **リスク 2: Cargo.toml 依存削除のやり過ぎ**。axum 等が他所で使われていれば削除でビルド破綻。→ `grep -rln` で MCP ドメイン専用と確認済み。削除後 `cargo build` で再確認。
- **リスク 3: C 群の撤去範囲の漏れ**。app_config は別ドメインのため、`server_to_model` 変換・`configured_secret_values`・`COMMAND_NAMES`・テスト等、フィールド削除に連動する箇所を取りこぼすとビルドエラー。→ `cargo clippy -- -D warnings` と grep（`mcp_port` / `mcp_token`）で検出。Non-goals「MCP 以外ドメインのリファクタ」との緊張はユーザー合意（徹底撤去）で解消済み。本変更は MCP 設定の除去に限定し、app_config の他構造は変えない。
- **リスク 4: releash.toml 後方互換**。既存ユーザー設定ファイルの `mcp_port` / `mcp_token` キー残存が読み込みエラーを起こさないか。→ `deny_unknown_fields` 不使用を確認済みのため無視され、エラーにはならない。
- **代替案（C 群）**: 「最小撤去（残す）」も検討したが、ユーザー合意により「徹底撤去（消す）」を採用。
- **代替案（削除手段）**: ディレクトリを `git rm` で一括削除 vs ファイル個別削除。前者を採用（履歴・差分が明快）。

## 仮定

- Spec ディレクトリは `docs/specs/issues-1219`（既存命名慣習）。
- MCP コマンドは `generate_handler!` への個別列挙ではなく `command/mcp/mod.rs::COMMAND_NAMES` + `register` 経由で登録されている。よって A 群物理削除＋`mcp::register` 行除去で過不足なく消える（`generate_handler!` 側の個別削除は不要）。
- `rmcp` / `axum` / `tokio-util` は MCP ドメイン専用（grep 確認済み）であり、Cargo.toml から削除してよい。共有依存（tokio 本体等）は残す。
- facet 4（`mcp__` 文字列分類）と Notion 設定は MCP ドメイン非依存であり、A 群削除の影響を受けない。
- `releash.toml` の MCP キーは `#[serde(default)]` 構成かつ `deny_unknown_fields` 不使用（確認済み）のため、徹底撤去後も既存ファイル読み込みは後方互換。
- C 群（app_config 内 MCP 設定）の扱いはユーザー合意により「徹底撤去（消す）」で確定。

## Open Questions

なし。
