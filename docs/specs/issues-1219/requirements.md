# Requirements

## Type

不要コードの撤去（デッドコード削除・機能廃止）。新機能追加や挙動変更を伴わない。

関連: #1219（本 Issue）/ #1198（A0 掃除・並行作業）

## Goal

CLI への全面移行によって用済みとなった、Releash 自前 MCP ドメイン一式を撤去する。

撤去対象は「Releash がエージェントへ公開する自前 MCP サーバ機能」(facet 1/2/3) であり、具体的には次の3機能を指す:

1. Releash 自前 MCP サーバ（agents → Releash の worktree / file tool 公開）
2. エージェントへの MCP 設定注入
3. MCP 設定 UI

完了時には、これらの実装（Rust ドメイン一式・フロント UI・起動時自動起動・コマンド登録）がコードベースから除去され、自前 MCP サーバが起動せず `mcp__releash__*` tool が提供されない状態になる。同時に、MCP 以外のデスクトップ全機能が従来どおり動作する（回帰なし）。

## Background

- MCP の実機能（Releash 自前 MCP サーバ＝agents → Releash の worktree / file tool 公開／エージェントへの MCP 設定注入／設定 UI）は **CLI への全面移行で用済み**になった。
- 現役で使われていないため、A0（掃除 #1198）と並行して撤去する。
- マイルストーン「A: ローカル server-client 化（基盤）」の一環として、基盤整備前に未使用ドメインを除去し、後続移行作業の対象面積を減らす意図がある。

### コード調査で確認済みの結合点（撤去対象）

静的調査で、Issue 記載の撤去対象がすべて実在することを確認済み:

- Rust ドメイン一式:
  - `src-tauri/src/domain/mcp/`（`entities` / `value_objects` / `error.rs` / `gateway.rs` / `mod.rs` / `services.rs`）
  - `src-tauri/src/usecase/mcp/`（`agent_config_usecase.rs` / `dto.rs` / `error.rs` / `lifecycle_usecase.rs` / `mod.rs` / `query_service.rs`）
  - `src-tauri/src/adaptor/gateway/mcp/`（`agent_config_impl.rs` / `mod.rs` / `server_impl.rs` / `state.rs`）
  - `src-tauri/src/adaptor/controller/command/mcp/`（`commands.rs` / `mod.rs`）
  - `src-tauri/src/infrastructure/external/mcp/`（`auth.rs` / `mod.rs` / `server.rs`）
- Rust 結合点:
  - `src-tauri/src/lib.rs` の `McpServerHandle` の `.manage(...)` 登録（82行目付近）と、起動時の `auto_start_mcp_server` 呼び出し（396〜399行目付近）
  - `src-tauri/src/adaptor/controller/command/mod.rs` の `pub(crate) mod mcp;`（6行目付近）と `mcp::register(&mut router);`（215行目付近）、および `generate_handler` への MCP コマンド登録
- フロント:
  - `src/components/panels/McpSettingsSection.tsx`
  - `src/hooks/useMcpConfig.ts`
  - `src/components/panels/SettingsModal.tsx` の MCP タブ（`useMcpConfig` / `McpSettingsSection` 参照箇所）

### 撤去対象に含めない（残す）もの

調査で「MCP という名前を含むが本撤去の対象外」であることを確認済み:

- `src-tauri/src/adaptor/controller/command/agent_session/tool_activity.rs` の `mcp__server__*` 分類（facet 4）。これは **エージェント自身が使う外部 MCP ツールの表示用**であり、撤去対象の MCP ドメインに依存しない。`classify_tool` の `mcp__` 判定とそのテスト（`mcp__notion__get_page` 等）はそのまま残す。
- フロント側 `src/components/panels/AgentChatPanel/ActivityLog.tsx` 等の `mcp` 表示分類（facet 4 の表示側）。残す。
- Notion 設定（`src/components/panels/NotionSettingsSection.tsx`）は別機能であり影響なし。残す。

## Users / Actors

- Releash デスクトップアプリのエンドユーザー（MCP 設定 UI が消える／自前 MCP サーバが起動しなくなる）
- 撤去・回帰検証を行う開発者
- A0 掃除（#1198）と並行作業する開発者

## Scope

- 上記「撤去対象（facet 1/2/3）」に列挙した Rust ドメイン一式・結合点・フロント UI の削除。
- 削除に伴って不要化する参照・インポート・モジュール宣言・コマンドハンドラ登録の除去。
- 自前 MCP サーバがアプリ起動時に起動しないことの確認。
- 削除後にビルド・テスト・lint がすべて緑であることの確認（回帰検証）。

## Non-goals

- facet 4（`tool_activity.rs` の `mcp__server__*` 分類とフロント表示）の変更・削除。
- Notion 設定機能（`NotionSettingsSection`）の変更。
- CLI 移行そのものの実装・拡張（本 Issue は撤去のみで、代替機能の新規実装は行わない）。
- MCP 以外のドメインのリファクタや、A0（#1198）が扱う掃除範囲の取り込み。
- クリーンアーキテクチャ構造そのものの変更（MCP ドメイン削除に必要な範囲を超えた改変は行わない）。

## Requirements

- 撤去対象の Rust ディレクトリ（`domain/mcp` / `usecase/mcp` / `adaptor/gateway/mcp` / `adaptor/controller/command/mcp` / `infrastructure/external/mcp`）が削除されていること。
- `lib.rs` から `McpServerHandle` の `.manage(...)` 登録と `auto_start_mcp_server` 呼び出しが除去され、アプリ起動時に自前 MCP サーバが起動しないこと。
- `command/mod.rs` から `mcp` モジュール宣言・`mcp::register` 呼び出し・`generate_handler` の MCP コマンド登録が除去されていること。
- フロントの `McpSettingsSection.tsx` / `useMcpConfig.ts` が削除され、`SettingsModal.tsx` から MCP タブとその参照が除去されていること。
- facet 4（`tool_activity.rs` の `mcp__` 分類とテスト、フロント側の `mcp` 表示分類）および Notion 設定が、本撤去によって変更・破壊されないこと。
- 削除に伴い、未使用となるインポート・参照・依存（撤去ドメインのみが使用していたクレート等があれば）が残らないこと。

## Acceptance Criteria（概要）

- `cargo clippy -- -D warnings` / `cargo test` がすべて緑（src-tauri/ で実行）。
- `pnpm lint` / `pnpm build` / `pnpm test` がすべて緑（プロジェクトルートで実行）。
- 自前 MCP サーバが起動せず、`mcp__releash__*` tool が提供されないこと。
- デスクトップの MCP 以外の全機能が従来どおり動作すること（回帰なし）。
- facet 4 の `mcp__server__*` 分類表示と Notion 設定が従来どおり機能すること。

## 仮定

- Spec ディレクトリ ID は、既存の `docs/specs/issues-NNN` 命名慣習に従い `docs/specs/issues-1219` とする。
- 「`generate_handler` の MCP コマンド登録」とは、`command/mod.rs`（および関連する Tauri ハンドラ登録箇所）に列挙された MCP 関連 `invoke` コマンドエントリを指すものとし、削除対象に含める。
- 撤去対象ドメインのみが利用していた外部クレート依存（`Cargo.toml`）があれば、未使用となるため併せて除去する。共有依存は残す。詳細な依存判定は `design.md` で確定する。
- フロントの `SettingsModal.tsx` における MCP タブ削除は、タブ自体の除去とそれに付随する import / state 参照の除去を指し、他タブ（Notion 等）の構成は変更しない。

## Open Questions

なし。
