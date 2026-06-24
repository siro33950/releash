# Requirements

## Type

内部構造のリファクタリング（巨大 module の責務別分割）。新機能追加や observable behavior の変更を伴わない。

関連: #1217（本 Issue） / #1192（CLOSED） / #1194（CLOSED） / #878（OPEN） / `docs/releash-performance-architecture-audit.md` M4（正本ドキュメント, commit `b0c5e4c2`） / マイルストーン #80「性能・メモリ効率改善」順序8「構造整理」

## 背景と目的

`src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs` は単一ファイルで 21,225 行（production 約 9,000 行・`#[cfg(test)]` 約 12,000 行）に達しており、agent bridge の以下の責務を 1 module に同居させている:

- runtime / process registry（`AgentProcess`、`BridgeState`、`TurnPhase`、per-session process map）
- stream emit（`AgentStreamSync` の累積 `streaming_parts` payload 生成、約 30fps の集約 emit、flush 閾値）
- session persistence（`persist_streaming_parts`、`load_post_turn_base_parts_from_store`、session storage 連携、event log 記録）
- permission（`set_agent_permission_mode`、`respond_agent_permission`、permission resolution の記録）
- recovery（PID ファイル管理・`cleanup_orphan_processes`、process 死活検知・再 spawn、session_ready 時の resume / context 復元）

このため、性能修正や lifecycle 修正の影響範囲が広く、変更が局所化できない。直近の #1192（ターン完了後に bridge プロセスが死んでも Rust が検知・再 spawn しない問題、CLOSED）と #1194（ターン完了時の `streaming_parts` 解放、CLOSED）の修正で、runtime / stream / persist / recovery の責務境界が具体的に見えた。

本 Issue の目的は、これらの責務を `bridge_common.rs` から責務別 module へ分割して恒久化し、今後の process death detection / respawn（#1192 系）や streaming release（#1194 系）の修正を局所的に扱える構造にすることである。あわせて、read command に write side-effect を持たせない境界を明確化し、command surface / compat path の削除候補を整理して #878（デッドコード削除）へ接続する。

audit M4「Clean Architecture 移行を性能改善と結びつける」の項目2「`bridge_common.rs` を runtime/process/stream/persist/recovery に分割する」および項目4「read command に write side-effect を持たせない」を実装に落とす Issue である。

## スコープ

### 分割対象 module

`bridge_common.rs` の production コードを、次の責務別 module へ分割する。具体的な module 名・ファイル配置・公開境界（`pub` / `pub(crate)`）は design.md で確定する。

1. **runtime / process registry** — `AgentProcess`、`BridgeState`、`TurnPhase`、`PendingMessage` 等の runtime 状態型と per-session process map（`chat_session_id -> AgentProcess`）の管理。
2. **stream emit** — `AgentStreamSync` の累積 `streaming_parts` payload 生成と、Tauri event / WS の両チャネルへの集約 emit（emit interval、flush 閾値 `STREAMING_PENDING_PART_LIMIT` / byte size cap、stale turn 判定）。
3. **session persistence** — streaming parts / post-turn base parts の永続化と読み出し、turn event log 記録、session storage 連携。
4. **permission** — permission mode 設定、permission 応答、permission resolution の記録。
5. **recovery** — PID ファイル管理・orphan process cleanup、process 死活検知・再 spawn、session_ready 時の resume / context 復元の接続。

### テスト

- 分割後の各 module は、その責務に対応する境界テスト（`#[cfg(test)]`）を持つ。
- 既存の `#[cfg(test)]` テスト（約 12,000 行）は、対応する責務の module へ移動・再配置する。テストの期待値は変更しない。

### read / write 境界の明確化

- read command（例: `get_session`）と write side-effect を持つ command の境界を、module 分割を通じて明確化する。
- 「read command に write side-effect を持たせない」という方針を、分割後の module 構造で表現する（read 経路と write 経路が別 module / 別関数として識別可能になる）。

### 削除候補の整理（#878 接続）

- 分割の過程で判明した、frontend から使われない command surface / compat path（旧 Tauri command、後方互換専用経路等）を削除候補として一覧化する。
- 一覧は #878（デッドコード削除）が参照・実行できる形で整理する。

## 非スコープ

- **observable behavior の変更**: agent チャットの動作、streaming の見え方、permission フロー、session 復元結果など、外部から観測可能な挙動は一切変えない。本 Issue は内部構造のみを変更する。
- **read command からの write side-effect の実除去**: 既存 read command が持つ write side-effect の削除は、observable behavior を変える可能性があるため本 Issue では行わない（境界の明確化までに留める）。実際の除去が必要な場合は別 Issue で扱う。
- **デッドコードの実削除**: 削除候補の一覧化までを行い、実際の削除は #878 で実施する。
- **性能・メモリの最適化そのもの**: #1192 / #1194 で実施済みの修正を再実装・再設計しない。本 Issue は構造の恒久化のみ。
- **frontend の変更**: フロントエンドのコード・呼び出し経路は変更しない（command の signature / 名前は維持する）。
- **`runtime/` 配下の他 module の再設計**: `claude.rs` / `codex.rs` / `codex_app_server.rs` / `context_restore.rs` / `permission_flags.rs` / `runtime_coordinator.rs` の責務再編は本 Issue の対象外（`bridge_common.rs` からの分割に必要な参照調整は行う）。

## 要求事項

### R1. 責務別 module への分割

- `bridge_common.rs` の production コードを、runtime / process registry・stream emit・session persistence・permission・recovery の 5 責務に対応する module へ分割する。
- 分割後、各責務の変更が当該 module 内で局所的に行える構造になっている。

### R2. observable behavior の不変

- 分割前後で、agent bridge の外部から観測可能な挙動（command の入出力、emit される event の内容・タイミング、永続化結果、エラー挙動）が一致する。
- 既存テストがすべて pass する（期待値の変更なし）。

### R3. 各 module の境界テスト

- 分割後の各 module が、その責務に対応する境界テストを持つ。
- 既存テストは対応する module へ移動し、テスト網羅性を維持する。

### R4. read / write 境界の明確化

- read 経路（副作用を持たない取得系）と write 経路（永続化・状態変更を伴う系）が、module 構造上で識別できる。
- 「read command に write side-effect を持たせない」方針が構造で表現される。

### R5. 削除候補の整理と #878 接続

- frontend から使われない command surface / compat path の削除候補が一覧化され、#878 が参照できる。

### R6. ビルド・Lint・テストの通過

- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`（`src-tauri/`）がすべて通る。

## 受け入れ基準の概要

- `bridge_common.rs` が runtime / process registry・stream emit・session persistence・permission・recovery の責務別 module に分割されている（R1）。
- 既存の observable behavior が変わっていない（R2）。既存テストが期待値変更なしで pass する。
- 各 module に境界テストがある（R3）。
- read command と write side-effect の境界が module 構造で明確になっている（R4）。
- command surface / compat path の削除候補が整理され、#878 と接続できる形になっている（R5）。
- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る（R6）。

## 仮定

- **A1**: 分割の責務区分は、Issue 本文および audit M4 が挙げる「runtime / process registry・stream emit・session persistence・permission・recovery」の 5 区分を採用する。具体的な module 名・ファイル分割粒度（1 責務 1 ファイルか、さらに細分するか）は design.md で確定する。
- **A2**: #1192（process death detection / respawn）と #1194（streaming_parts 解放）はすでに CLOSED であり、本 Issue はそれらの修正を再実装せず、修正で見えた責務を恒久的に局所化する構造整理に限定する。
- **A3**: #878（デッドコード削除）の実削除は本 Issue では行わず、削除候補の一覧化までを行う。実削除は #878 が担う。
- **A4**: read command が現在持つ write side-effect の実除去は、observable behavior を変えうるため本 Issue では行わず、境界の明確化に留める。
- **A5**: 既存の `#[cfg(test)]` テストは期待値を変えずに対応 module へ移動する。テストの追加は「各 module の境界テストが存在する」状態を満たすための補完に限る。
- **A6**: 分割先は `runtime/` 配下（`src-tauri/src/infrastructure/agent_session/runtime/`）に置き、`mod.rs` の `pub mod bridge_common;` 公開境界を維持または整理する。public command の signature・名前・呼び出し経路は変更しない。

## Open Questions

なし。
