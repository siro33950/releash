# Requirements

## Type

土台リファクタ（決定済みの削除掃除。機能追加なし・既存デスクトップ機能の振る舞い不変）

## Goal

マイルストーン A「ローカル server-client 化（基盤）」の A1（walking skeleton）が乗る土台を整えるため、決定済みの削除を掃除する。具体的には、ドリフトしたリモートクライアント（`src/remote/`）と remote ビルド経路、ドリフトした ws の req/resp ハンドラ群一式を削除し、ws を「**認証＋push broadcast の shell**」へ縮退させる。ドリフト遺産の上に A1 の新実装を建てない状態を達成する。

## Background

- リモートアクセス機能（`src/remote/` の React クライアント＋WebSocket 経由の Git/PTY/diff/コメント/エージェント操作）は、現行アーキテクチャ方針（全ロジックを Rust に集約し、クライアントは任意の薄いインターフェースに徹する server-client 化）の確定に伴い、ドリフトした旧実装として削除対象に決定済みである。
- 現状の ws は `src-tauri/src/ws_server/routing.rs` で約 25〜26 個の req/resp ハンドラ（PTY spawn/kill/output、worktree list/select、backend list、agent session/message/interrupt/permission/model 等、review list/get/create/append/resolve/history 等）を保持しており、これらは削除予定のリモートクライアントからの要求に応えるためのものである。
- デスクトップアプリ本体は Tauri の `invoke`（コマンド）経由で Rust ロジックを呼び出しており、ws の req/resp ハンドラには依存しない。したがってリモートクライアントと req/resp ハンドラを削除してもデスクトップの機能には影響しない、という前提で本作業を行う。
- push 系（broadcast）の経路と、認証・HTTP・レート制限の土台は A1 以降も再利用するため残す（Issue 本文の「案X」）。具体的には `src-tauri/src/ws_server/http.rs` / `auth.rs` / `rate_limit.rs` と `WsBroadcaster`（`src-tauri/src/ws_bridge.rs`）を残す。
- デスクトップは現状 Tauri の `emit`/`listen`（イベント）も利用しているため、`emit` 経路は当面残す。完全な ws 集約（A-flip）は後続マイルストーンで行う。

## Users / Actors

- Releash のデスクトップアプリ利用エンドユーザー（本作業後も従来どおり全機能が動作することが必要）
- A1（walking skeleton）以降の実装を行う開発者（掃除された土台の上に新実装を建てる）
- 削除に伴う回帰が無いことを検証する開発者

## Scope

- `src/remote/` 一式（`RemoteApp` とその配下の components/hooks/styles/main 等）の削除。
- remote ビルドターゲット・設定の削除（`vite.config.remote.ts`、および `package.json` の `build:remote` 等 remote 専用スクリプト、remote 専用依存）。`build` / `dev` スクリプトから remote ビルドへの参照を除去する。
- CI（`.github/workflows/ci.yml`）からの remote 参照除去（`pnpm build:remote` ステップ、`RemotePanel*` スクリーンショットフィルタ）。remote 削除後も CI がクリーンに通る構成にする。
- リモートアクセス起動トリガー一式の削除（追加合意「使わないものは全て削除」）。`src/remote/` クライアント削除により接続先が消えるため、リモートサーバを起動・制御する経路を残らず削除する。対象は以下。
  - フロント: `src/components/panels/RemotePanel.tsx`（+テスト）、`src/hooks/useRemoteServer.ts`（+テスト）、`src/hooks/useRemoteAutoStart.ts`（+テスト）、`App.tsx` 等からの RemotePanel 参照。
  - Tauri command: `start_server` / `stop_server`（`adaptor/controller/command/mod.rs` の登録と invoke ラッパー）。
  - tray（独立経路）: `Start Server` / `Stop Server` メニュー、`handle_start_server` / `handle_stop_server`、`update_tray_menu(server_running)`。
  - 自動起動（独立経路）: `lib.rs` の `config.remote.auto_start` を見て `start_server_core` を spawn する配線。
  - config: `remote.auto_start` / `app.last_bind_ip` / サーバーポート等の remote 起動専用設定。
  起動機構の core（`ws_server/commands.rs` の `start_server_core` / `stop_server_core`）と shell（後述の案X）は A1 で再利用するため残す。起動トリガー全削除により core/shell の呼び出し元・push 送信元はゼロになるが、`#[allow(dead_code)]` 等で温存し A1 で起動を再配線する。温存手段の具体（`#[allow]` / `cfg` 等）と config マイグレーション（remote セクション除去）の互換性確保は `design.md` で確定する。
- ドリフトした ws の req/resp ハンドラ群（約 26）と、それ用の partial メッセージ variant（`WsMessage` の各 `*Request` / `*Response` 等）の削除。
- ws を「認証＋push broadcast の shell」へ縮退（`http.rs` / `auth.rs` / `rate_limit.rs` / `WsBroadcaster` は残す＝案X）。push（`*Sync` / `PtyOutput` / `PtyExit` 等の broadcast）と認証ハンドシェイクのみを ws が担う状態にする。
- 削除に伴って不要となるコード（`routing.rs` の req/resp 分岐、`handlers.rs` の req/resp ハンドラ実装、`commands.rs` / `validation.rs` 等の req/resp 専用部分、対応する protocol 型・テスト）の整理。
- 上記による回帰が無いことの検証（フロント＋Rust＋CI のクリーンビルド、デスクトップ全機能の回帰確認、既存テスト緑）。

## Non-goals

- A1（walking skeleton）そのものの実装（本 Issue は土台の掃除のみ）。
- ws を通じた新しいクライアント（モバイル等）の再実装・新プロトコル設計。
- Tauri `emit`/`listen` 経路の撤去や ws への一本化（A-flip）。本 Issue では `emit` 経路を残す。
- デスクトップ機能の挙動変更・機能追加・UI 変更（ただし、本 Issue で合意済みの「リモートアクセス起動トリガー一式（RemotePanel 経路・tray の Start/Stop Server・自動起動）の削除」を除く。これはリモートアクセス機能の一部であり、通常のデスクトップ機能の変更ではない）。
- リモートクライアントが提供していた機能の代替手段の提供。リモートアクセス起動トリガーを削除した後の代替手段も提供しない。
- 残置する `http.rs` / `auth.rs` / `rate_limit.rs` / `WsBroadcaster` / `start_server_core` の再設計・リファクタ（縮退・温存に必要な最小限の調整＝`#[allow(dead_code)]` 付与等を超える変更は行わない）。

## Requirements

- `src/remote/` 一式と remote ビルド経路（`vite.config.remote.ts`、`package.json` の remote 専用スクリプト・依存）が削除され、ビルド設定から remote ターゲットへの参照が残らないこと。
- CI（`ci.yml`）から remote 参照（`build:remote` ステップ、`RemotePanel*` スクリーンショットフィルタ）が除去され、remote 削除後も CI がクリーンに通ること。
- リモートアクセス起動トリガー一式が削除されていること。すなわち、フロントの起動UI（`RemotePanel.tsx`+テスト、`useRemoteServer.ts`+テスト、`useRemoteAutoStart.ts`+テスト、`App.tsx` 等の参照）、`start_server` / `stop_server` の Tauri command、tray の `Start Server` / `Stop Server` メニューとハンドラ・`update_tray_menu`、`lib.rs` の auto-start 配線、`remote.auto_start` / `last_bind_ip` / サーバーポート等の config 項目が残らないこと。
- 起動機構の core（`start_server_core` / `stop_server_core`）と shell（案X）は残置すること。起動トリガー全削除で呼び出し元・push 送信元がゼロになっても、`#[allow(dead_code)]` 等で温存され `clippy -D warnings` を通る状態であること。
- ドリフトした ws の req/resp ハンドラ群（約 26）と、それ専用の `WsMessage` variant（`*Request` / `*Response` 等）が削除されていること。これらに紐づくルーティング分岐・ハンドラ実装・バリデーション・テストも併せて除去されること。
- ws が「認証＋push broadcast の shell」に縮退していること。すなわち、認証ハンドシェイクと push（broadcast）配信のみを担い、req/resp 要求は受理しない（または明示的にエラー応答する shell）状態であること。
- `http.rs` / `auth.rs` / `rate_limit.rs` / `WsBroadcaster` が残存し、引き続き機能すること（案X）。
- Tauri `emit` 経路が残存し、デスクトップの `listen` 利用が従来どおり機能すること。
- デスクトップ本体のロジック（`invoke` から呼ばれる usecase/adaptor/コマンド）は、ws req/resp ハンドラ削除の巻き添えで削除されないこと。削除対象は ws の req/resp 表層（routing/handler/protocol variant）に限定し、デスクトップが利用する下層ロジックは保持する。
- 既存テスト（`cargo test` / `pnpm test`）と lint（`cargo clippy -D warnings` / `pnpm lint` / `cargo fmt --check`）、および CI がクリーンに通ること。

## Constraints

- 削除は決定済みのドリフト遺産（remote クライアント・req/resp ハンドラ）および合意済みのリモートアクセス起動トリガー一式（RemotePanel 経路・tray・autostart・command・config）に限定し、要求外のスコープ拡大（無関係なリファクタ・残置土台の作り直し）を行わないこと。
- デスクトップから観測できる機能的振る舞いを変えないこと（remote 削除は invoke 経路に影響しないことを回帰確認する）。ただしリモートアクセス起動トリガー（RemotePanel 経路・tray の Start/Stop Server・自動起動）の除去は本 Issue の意図された削除であり、この例外を除き挙動を変えないこと。
- 案Xに従い `http.rs` / `auth.rs` / `rate_limit.rs` / `WsBroadcaster` / `start_server_core` を残すこと。これらを削除・大幅改変しないこと（温存のための `#[allow(dead_code)]` 付与は許容）。
- `emit` 経路を本 Issue では撤去しないこと（A-flip は後続）。
- CLAUDE.md の方針（全ロジックは Rust、フロントはインターフェース）に反する状態を新たに作らないこと。

## Success Criteria（受け入れ基準の概要）

- フロントエンド・Rust・CI がクリーンにビルドできる（`pnpm build` / `cargo build` / CI が remote 削除後も成功）。CI から `build:remote` ステップ・`RemotePanel*` フィルタが除去されている。
- リモートアクセス起動トリガー（RemotePanel 経路・tray・自動起動）を除くデスクトップの全機能が従来どおり動作する（desktop は invoke 経由のため remote 削除の影響を受けないことを回帰確認する）。起動トリガーの除去は意図された削除であり回帰ではない。
- 既存テストが緑であり、ws は req/resp ハンドラを持たない「認証＋push broadcast の shell」になっている。
- `src/remote/` と remote ビルド経路・専用依存、およびリモートアクセス起動トリガー一式（RemotePanel/tray/autostart/command/config）がリポジトリから除去され、ドリフトした req/resp ハンドラ群・partial variant が残存しない。起動機構の core（`start_server_core` / `stop_server_core`）と shell（案X）は `#[allow(dead_code)]` 等で温存されている。

## 仮定（本文中で明示）

- Spec ディレクトリ名は直近の慣例（`docs/specs/issues-1191` 等）に合わせ `docs/specs/issues-1198` とする。
- 「req/resp ハンドラ群（~26）」は `routing.rs` の `WsMessage::*Request` 分岐とそれが呼ぶ `handlers.rs` の実装、対応する `*Request` / `*Response`（および `ReviewThreadResponse` 等の応答 variant）を指すと解釈する。`*Sync`（`WorktreePrStatusSync` / `BranchListSync` / `AgentStateSync` / `WorkflowStateSync` / `AgentStreamSync`）、`PtyOutput` / `PtyExit` / `PtyReady`、認証系（`AuthChallenge` / `AuthResponse` / `AuthResult`）、`Error` は push/認証 shell として残す。
- インバウンドのアプリ制御メッセージ（`PtyInput` / `PtyResize` / `PtyOutputRequest` 等）は、リモートクライアント削除後に送信元が無くなるため req/resp ハンドラ群と同様に削除対象とみなす（ws の inbound 受理は認証ハンドシェイクに縮退）。これに該当する partial variant も削除する。削除/残置の最終境界は `behavior.md` / `design.md` で確定する。
- ws req/resp ハンドラが内部で呼んでいた usecase/adaptor 層は、デスクトップが `invoke` 経由で利用しているため保持する。削除はあくまで ws 表層に限定する。
- remote 専用依存の特定（`package.json` のどの依存が remote 専用か）と削除可否の精査は `design.md` で行う。本 Issue では「remote 専用依存を削除する」方針のみ確定する。
- リモートアクセス起動トリガー一式の削除は、当初の Scope には無かったがユーザー合意（「使わないものは全て削除」「再利用するものは削除しない」）により追加した。`src/remote/` クライアント削除で接続先が消え、起動トリガー（RemotePanel・tray の Start/Stop Server・auto-start 配線・`start_server`/`stop_server` Tauri command）が「繋ぐ相手のいないサーバを起動する」宙ぶらりん状態になるため、これらは全て削除する。
- 一方、起動機構の core（`start_server_core` / `stop_server_core`）と shell（案X: `http.rs`/`auth.rs`/`rate_limit.rs`/`WsBroadcaster`）は A1 で再利用するため残す。起動トリガー全削除で呼び出し元・push 送信元がゼロになるため、`#[allow(dead_code)]` 等で温存し A1 で起動を再配線する方針とする。温存手段の具体（`#[allow]` / `cfg` 等）、`WsBroadcaster` の push 送信経路の扱い、config マイグレーション（`remote` セクション除去の後方互換）の最終境界は `design.md` で確定する。

## Open Questions

なし（上記仮定で進め、削除/残置の詳細境界は behavior.md / design.md で確定する）。
