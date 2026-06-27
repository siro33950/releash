# Requirements

## Type

性能・メモリ効率改善（保存・配信モデルの変更を伴うリファクタリング + 新規 port 追加）。

対象 Issue: #1249

正本ドキュメント: `docs/releash-performance-architecture-audit.md`（M2 / セクション 4「Agent Session Storage / Streaming」）
マイルストーン: 性能・メモリ効率改善（Workbench State / Read Model）

## 背景と目的

### 背景

Issue #1213 は session summary index / message paging を、#1214 は累積 snapshot から `seq` delta streaming への移行を扱う。しかし **巨大な tool output 本文（test / lint / shell / terminal 由来の出力）をどこに保存し、message part に何を持たせるか** はまだ定義されていない。

実コードで確認した現状（要約。詳細は `design.md` で扱う）:

- message part の `MessagePart::ToolResult` は `content: String` に tool 出力本文を **そのまま全量保持** する（`src-tauri/src/usecase/agent_session/session/mod.rs`）。
- streaming 中は `persist_streaming_parts` / `emit_streaming_parts`（`src-tauri/src/infrastructure/agent_session/runtime/bridge_common/session_persistence.rs`・`stream_emit.rs`）が parts を保存・配信し、`extract_tool_result_content`（`bridge_common/sdk_message.rs`）が SDK の tool result raw content を文字列化して `content` に載せる。
- session 本文は paging 対応済み（`AgentSessionReader::get_session_page` / `persist_message_parts`、`src-tauri/src/domain/agent_session/storage.rs`）だが、page が返す message part には tool output 全文がそのまま含まれる。
- attachment は既に `get_session_attachment` で id 参照型の別取得経路を持つ（content-ref 化の先行事例）。一方 tool output には同等の参照保存経路がない。
- WS 配信は `WsBroadcaster`（`src-tauri/src/ws_bridge.rs`）が parts を運ぶため、巨大 tool output は配信 payload にも比例して載る。

参考実装の OpenCode は、tool output を max lines / max bytes の閾値で truncate し、full output を別保存して参照させる（`packages/opencode/src/tool/truncate.ts`）。古い tool output の pruning / compaction も持つ（`packages/opencode/src/session/compaction.ts`）。

この結果、以下の failure mode が発生する:

- test / shell output で session JSON が肥大化する。
- streaming frame payload が長い tool output に比例して増える（#1214 の delta 化だけでは、1 つの大きな tool result delta 自体が大きいため解消しきれない）。
- frontend memory に、画面に表示されない巨大ログが常駐する。
- remote / WS broadcaster が巨大 tool output を再送し続ける。

### 目的

tool（および tool 由来の test / lint / shell / terminal）output の **保存境界を定義** し、message part が巨大本文ではなく **`content_ref` / summary / truncated preview** を持てるようにする。full output は message body とは別の **ToolOutputStore** に保存し、message page API は preview と ref を返して、必要時だけ full output を読む構成にする。これにより session JSON・streaming payload・frontend memory・WS 再送のいずれも、tool output 全長に比例して増えない状態にする。あわせて #1209 telemetry で truncated count / full output bytes を観測できるようにし、ユーザーデータを通常ログへ出さない。

## スコープ

- **保存境界の定義**: tool output の size / line threshold（truncate 判定基準）を定義する。閾値未満の output は従来どおり part に inline し、閾値超過分のみ full output を別 store に退避して part を ref + preview 化する。
- **message part の content-ref 化**: `MessagePart::ToolResult` が、巨大本文の代わりに `content_ref`（full output の参照）・`summary`（行数 / バイト数等の privacy-safe metadata）・truncated `preview`（先頭一定量）を保持できるようにする。閾値未満の小さい output は従来どおり inline 本文のままでよい。
- **ToolOutputStore port の追加**: full output を message body とは別に保存・取得する port（`ToolOutputStore`）を、session paging / streaming delta と整合する形で domain / usecase 層に追加する（`.claude/rules/rust-first-logic.md`）。attachment store（`get_session_attachment`）と同様、id/ref ベースで full output を遅延取得できる経路を持つ。
- **保存先・retention・privacy-safe metadata の決定**: full output の保存先（content-addressed file blob 等）、retention / pruning ポリシー（OpenCode の compaction 相当の方針決定）、part に残す metadata（full 本文を含めない、サイズ・行数・error フラグ等のみ）を決める。
- **message page API の preview + ref 化**: message page API（`get_session_page`）が、巨大 tool output について preview と ref を返し、full output を読むのは frontend からの明示要求時だけにする。full output 取得用の Tauri command / port 経路を用意する。
- **streaming / WS 配信との整合**: streaming delta（#1214）と WS 配信（`WsBroadcaster`）が、巨大 tool output を全量 payload に載せず ref + preview で運べるようにする。full output は delta / snapshot 経路ではなく ToolOutputStore 経由で取得させる。
- **telemetry 観測点の追加**: #1209 の OTel 計装で、truncated count（truncate された tool output 件数）と full output bytes（store に退避した総バイト数）を観測できるようにする。
- **privacy 保証**: tool output 本文（= ユーザーデータ）を通常ログ（log / span attribute 等）へ出さない。telemetry は件数・バイト数等の集計値のみとする。
- **検証手段の整備**: 閾値超過 output で session JSON / page payload / streaming frame payload が full output 全長に比例しないこと、full output が必要時のみ読まれること、ログにユーザーデータが出ないことを確認する検証手段を用意する。

## 非スコープ

- **streaming の delta 化そのもの**（累積 snapshot → seq delta）は #1214 が担当。本 Issue は delta payload に巨大 tool output が載らない状態を保証するに留め、delta protocol 自体は再設計しない。
- **session summary index / message paging（`get_session_page` 等）の追加**は #1213 が担当。本 Issue はその paging 経路に preview + ref を載せる前提で利用する。
- **frontend の閉じた session / 非表示 worktree の body 退避・仮想化・LRU**（#1195）。本 Issue は part を ref 化して body を軽くするところまでで、frontend 側の visible window / LRU 退避ロジックは扱わない。
- **terminal / PTY の live output buffer cap・lifecycle**（#1215, CLOSED）。`ws_bridge.rs` / runtime の PTY output ring buffer（各 64KB 上限）の cap・解放・LRU・idle timeout は #1215 が実装済み。本 Issue が扱う「terminal / shell / test / lint 由来 output」は、agent message part 経由で保存される tool result 本文としての扱いに限定し、live PTY buffer 領域には踏み込まない（A1 で確定）。
- **`bridge_common.rs` の module 分割**（#1217）。
- **telemetry 基盤（OTel / New Relic）そのものの構築**は #1209 が担当。本 Issue は計測点（truncated count / full output bytes）の追加に留める。
- **legacy `content` / `thinking` / `activities` 二重保持の全面廃止**。tool output の ref 化に必要な範囲を超えた legacy 表現の撤去は扱わない。
- tool output 表示の UI 仕様・見た目の変更（preview の折りたたみ UI 等の新規デザイン）。「ref を必要時に展開取得する」最小限の経路整備に留める。

## 要求事項

- R1: tool output の truncate 判定基準（max lines / max bytes 等の閾値）が定義され、閾値未満は part に inline、閾値超過は full output を別 store へ退避して part を ref + preview 化すること。
- R2: `MessagePart::ToolResult`（およびこれに準ずる tool 由来 part）が、full output の代わりに `content_ref`・truncated `preview`・privacy-safe `summary`（行数 / バイト数 / error フラグ等）を保持できること。preview / summary に full output 全長が含まれないこと。
- R3: full output を message body とは別に保存・取得する `ToolOutputStore` port が追加され、session paging / streaming delta と整合する経路（id/ref ベースの遅延取得）を提供すること。port は usecase / domain 層に置き、frontend は invoke 経由で full output を取得すること（Rust-first）。
- R4: full output の保存先・retention / pruning ポリシー・part に残す metadata が定義され、part 側に full 本文が残らないこと。retention ポリシーにより古い full output を整理できる（または整理しないことが明示的に決定されている）こと。
- R5: message page API が、巨大 tool output について preview と ref を返し、full output を読むのは frontend からの明示要求時のみであること。閾値未満の小さい output は従来どおり inline 本文で返ってよい。
- R6: streaming delta（#1214）と WS 配信が巨大 tool output を全量 payload に載せず、ref + preview で運べること。reconnect / 通常配信のいずれでも full output 全長が payload に比例して載らないこと。
- R7: #1209 telemetry で truncated count と full output bytes を観測でき、かつ tool output 本文（ユーザーデータ）が通常ログ・span attribute・metric ラベル等へ出ないこと。
- R8: 上記を満たすことを確認する検証手段（閾値超過時に session JSON / page payload / streaming payload が full 全長に比例しないこと、full output が必要時のみ読まれること、ログにユーザーデータが出ないことの確認）が用意されること。

## 受け入れ基準の概要

- tool output の size / line threshold が定義されている（R1）。
- full output が message body とは別の store に保存できる（R3 / R4）。
- message page API が preview と ref を返し、必要時だけ full output を読む（R5）。
- streaming / WS 配信が巨大 tool output を ref + preview で運ぶ（R6）。
- telemetry（#1209）で truncated count / full output bytes を観測でき、ユーザーデータが通常ログへ出ない（R7）。
- 上記の非退行・privacy を確認する検証手段がある（R8）。

詳細な受け入れシナリオ（Gherkin）は `behavior.md` で定義する。

## 仮定

以下は確認の結果、確定済みの前提（A1〜A4 は人間レビューで合意済み）。

- A1【確定】: 本 Issue の対象は、**agent message part（`MessagePart::ToolResult` 等）として保存・配信される tool result 本文**（test / lint / shell / tool 由来）に限定する。terminal の live PTY output buffer 自体（`ws_bridge.rs` / runtime の各 64KB ring buffer）は #1215（CLOSED, 実装済み）が担当し、本 Issue では扱わない。
- A2【確定】: full output の保存先は、attachment store（`get_session_attachment`）と同様の **content-addressed file blob 参照方式**（per-session ディレクトリ配下のファイル + ref id で遅延取得）とする。新規 DB（sqlite 等）や単一ファイル追記方式は採らない。
- A3【確定】: requirements では「truncate 閾値（max lines / max bytes）が定義されていること」を要求とし、**具体的な確定値は #1209 の performance budget（通常 frame payload < 64KB 等）確定後に design.md / 実装で固定・調整する**。既定の検討起点は OpenCode 相当（例: max ~1000 lines / ~30KB）。
- A4【確定】: retention は **session ライフサイクル連動**（session 削除時に当該 session の full output blob を一緒に削除する）とする。OpenCode 相当の「古い tool output の積極的 pruning / compaction」は本 Issue では非スコープとし、必要なら別 Issue 化する。
- A5: 閾値未満の小さい tool output は、ref 化せず従来どおり part に inline 本文で保持する（過剰な store 参照を避ける）。
- A6: spec ディレクトリ名は `docs/specs/issues-1249` とする（直近 Issue の命名規約に合わせる）。

## Open Questions

なし（Q1〜Q4 は人間レビューで解消し、A1〜A4 として確定済み）。
