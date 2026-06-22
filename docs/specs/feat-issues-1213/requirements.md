# Requirements

## Type

性能・メモリ効率改善（機能追加を伴うリファクタリング）

対象 Issue: #1213

正本ドキュメント: `docs/releash-performance-architecture-audit.md`（M2 / 項目 3・4）
マイルストーン: 性能・メモリ効率改善（Workbench State / Read Model）（#80）

## Goal

Agent session の list / get / fork / save が「session 本文の総量」に比例して memory / IO を消費する現状を解消し、session storage を **summary index** と **message paging** に分離する。

完了時には、session 一覧の取得が会話本文の総量に依存せず、長い会話を開いても初期描画で全 message body を読み込まず、page 単位（cursor + limit）で message body・attachment 参照・token / run metadata を取得できる状態にする。あわせて、#1190 の復帰時 context restoration と矛盾しない「保存正典（どのデータが正典で、どこに保存されるか）」を定義する。

## Background

`docs/releash-performance-architecture-audit.md` の M2 が指摘するとおり、現状の Agent session storage は本文量・session 数に比例して memory / IO を圧迫する。確認できている現状は次のとおり:

- `SessionStore`（`src-tauri/src/usecase/agent_session/session/store.rs`）は、`~/.releash/sessions/{session_id}.json` に `ChatSession` 全体を `to_writer_pretty` で **full JSON pretty** 保存する（atomic write: tmp → rename）。
- `list_sessions_filtered`（store.rs 134-157）は、初回に全 session JSON を cache に読み込んでから summary 化する。session 数と本文量に比例して startup / list が重くなる。
- `get_session`（store.rs 294-305）は `ChatSession` 全体を clone して返す。message 本文と attachment が毎回 clone される。
- `save_session` / `persist_and_update_cache`（store.rs 321-363）は full session を pretty JSON で書き直す。streaming 中は巨大 JSON を繰り返し全量保存する。
- `fork_session`（store.rs 249-292）は `messages.clone()` 相当で session を複製する。長い会話を即時 full copy する。
- `ChatMessage`（session/mod.rs 212-228）は新形式の `parts: Option<Vec<MessagePart>>` を主ペイロードとしつつ、legacy 互換の `content` / `thinking` / `activities` を二重保持している。`MessagePart::Image` 等の binary payload は JSON に inline される懸念がある。
- frontend は `get_session`（`src/hooks/useSessionStore.ts`）で全 message を読み込み、`useAgentSdkListeners.ts` の `ViewableSessionRegistry` で表示対象 session の listener を絞ってはいるが、**page API は未実装**で、初期描画時に全 message body を hydrate する。

これらにより、session 数・会話長が増えるほど startup / list / get / save / fork の memory・IO が線形に悪化する。本 Issue は、この読み書きモデルを summary index + paging へ作り直す。

## Users / Actors

- 多数の session・長い会話を抱えた状態で session 一覧を開き、過去 session を開いて続行するユーザー（デスクトップ UI）
- session の永続化・一覧化・取得・fork・保存を担う Releash（`SessionStore` および周辺 usecase / adaptor）
- session を読み書きする Agent runtime（streaming 永続化経路を含む）

## Scope

- **session list を summary index だけで返す**: 一覧取得が message 本文の総量に依存しないよう、summary（id / worktree / title / state / 件数や代表メッセージなどの軽量メタ）と body を分離する。
- **`get_session_page(session_id, cursor, limit)` の追加**: message body・attachment 参照・token / run metadata を page 単位（cursor ベース）で取得する Rust 側 read API を追加し、Tauri command として公開する。
- **保存正典の定義**: 何を正典データとして、どの粒度・どの場所に保存するかを定義し、#1190 の復帰時 context restoration（`agent_session_id` / `context_carry` / `messages` の復元）と矛盾しないようにする。
- **fork の非 full-copy 化**: `fork_session` を full copy から copy-on-write / parent 参照 / selected range fork のいずれか（design で確定）へ寄せ、長い会話の即時全量複製をなくす。
- **既存 session の互換維持**: 旧フォーマット（現行の full JSON）で保存済みの session を、データを破壊せず読めるようにする（必要に応じ migration / 遅延変換）。
- **frontend が paging API を消費する**: 初期描画で全 body を hydrate せず、可視 page のみを取得・表示するところまでを本 Issue に含める。frontend を新 paging API へ接続し、受け入れ基準「初期描画で全 message body を hydrate しない」を本 Issue 内で満たす。閉じた session / 非表示 worktree の body 退避や本格的な仮想化・LRU は #1195 が主担当とする。

## Non-goals

- ターン完了時の `streaming_parts` 解放（#1194 が担当）。本 Issue の paging はその成果を前提にできるが、解放処理の実装自体は対象外。
- 閉じた session / 非表示 worktree の body 退避、本格的な仮想化・LRU 等の frontend メモリ最適化（#1195 が担当）。本 Issue は paging API の提供と、その消費による「初期描画で全 body を hydrate しない」までに留める。
- streaming を cumulative snapshot から seq delta protocol へ移行する変更（#1214 が担当）。
- `bridge_common.rs` の module 分割（#1217 が担当）。
- legacy `content` / `thinking` / `activities` の全面廃止。paging が legacy 二重保持に依存しない形にはするが、互換出力としての legacy 生成（`parts_to_legacy`）の完全撤去は別途扱う。
- session 検索 UI / 一覧 UI の仕様変更そのもの（性能のための内部変更に限定）。
- Agent backend（Claude / Codex）の resume 仕様の変更。

## Requirements

- session 一覧の取得が、session 本文（message body）の総量に比例しないこと。summary index だけを読んで一覧を構成できること。
- `get_session_page(session_id, cursor, limit)` により、message body・attachment 参照・token / run metadata を page 単位で取得できること。cursor で続きを辿れ、limit で 1 回の取得量を制御できること。
- 長い会話を開いても、初期描画時に全 message body を hydrate しないこと（可視 window 相当のみを取得する）。
- `get_session`（単一 session 取得）が `ChatSession` 全体を毎回 clone しないか、または summary + page 取得に置換され、本文量に比例した clone が発生しないこと。
- session の保存が、streaming 中も含めて本文全量の繰り返し全量書き込みに依存しないこと（差分 / chunk / append のいずれか。具体方式は design で確定）。
- `fork_session` が message 本文を即時 full copy せず、copy-on-write / parent 参照 / selected range fork のいずれかで成立すること。
- 保存正典が明文で定義され、#1190 の復帰時 context restoration（`agent_session_id`・`context_carry`・復元対象の `messages`）と矛盾しないこと。
- 旧フォーマットで保存済みの既存 session を、本変更後に開いてもデータ（messages / メタ）を破壊しないこと。
- ロジックは Rust（Tauri バックエンド）側に置く方針（`.claude/rules/rust-first-logic.md`）に従い、paging / summary 化 / fork のロジックを frontend に持ち込まないこと。

## 受け入れ基準の概要

- **session list が本文総量に比例しない**: 多数・長文の session がある状態で一覧取得しても、取得コスト（時間 / 読み込みバイト）が message body 総量に対して支配的に増えない（summary index のみを参照する）ことを確認できる。
- **初期描画で全 body を hydrate しない**: 長い会話を開いたとき、初期表示で全 message body を読み込まず、page 単位で取得していることを確認できる。
- **保存正典の定義**: #1190 の context restoration と矛盾しない保存正典が文書（behavior / design）に定義され、復帰時に必要な識別子・messages が欠落しないことを確認できる。
- **fork が full copy でない**: 長い会話を fork しても即時の本文全量複製が発生しないことを確認できる。
- **既存 session の非破壊**: 旧フォーマットの session を開いて表示・続行しても、データが壊れないことを確認できる。

## Constraints

- 表示用の messages 復元と Agent 側 context 復帰の整合（#1190 の前提）を壊さないこと。paging 導入により「見えているのに復帰できない」状態を新たに作らない。
- 既存の永続化済み session（旧フォーマット）を意図せず破壊しないこと。
- 新規 session 開始・通常のターン継続・streaming 永続化の挙動を壊さないこと。
- summary index と body の整合（index が body の実態と食い違わない）を保証すること。
- ロジックは Rust 側に置き、frontend はインターフェースに徹する（`.claude/rules/rust-first-logic.md`）。

## Success Criteria

- 多数 / 長文の session を用意した状態で session 一覧を開き、取得コストが本文総量にスケールしないことを計測で確認できる。
- 長い会話 session を開いたとき、初期取得が page 単位に限定され、全 body を読み込まないことを確認できる。
- session を fork したとき、本文の即時 full copy が発生しないことを確認できる。
- 再起動 / 履歴復帰した session で、#1190 の context restoration（復帰後の会話継続）が引き続き成立することを確認できる。
- 旧フォーマットの既存 session を開いてもデータが破壊されないことを確認できる。

## 仮定

- 本 Issue のスコープは M2 の項目 3（summary index）と 4（message paging）であり、`streaming_parts` 解放（#1194）・frontend 仮想化の本格実装（#1195）・seq delta streaming（#1214）は別 Issue として切り離す。本 Issue はそれらと整合する read model を提供する。
- 保存先は引き続き `~/.releash/sessions/` 配下とし、保存単位・フォーマット（単一 JSON + summary index sidecar / per-message chunk / append-only log のいずれか）は design.md で確定する。本 requirements では「本文全量の繰り返し保存に依存しない」ことのみを要求とする。
- fork の具体方式（copy-on-write / parent ref / selected range）は design.md で確定する。本 requirements では「即時 full copy をしない」ことのみを要求とする。
- cursor の意味（時系列の前方 / 後方ページング、安定 id ベース）は behavior / design で確定する。本 requirements では「cursor + limit で page を辿れる」ことのみを要求とする。
- 旧フォーマット session は遅延 migration（開いたとき / 保存時に新形式へ変換）または読み取り時互換のいずれかで扱う。具体方式は design.md で確定する。
- 性能の定量基準（具体的な時間 / バイト閾値）は #1209 の telemetry を前提とし、本 Issue では「本文総量にスケールしない」という相対基準を採用する。

## Open Questions

なし

（解消済み: 本 Issue における frontend 変更の範囲 → paging API の消費まで本 Issue に含め、初期描画で全 message body を hydrate しないことを #1213 内で満たす。閉じた session / 非表示 worktree の body 退避・本格的な仮想化・LRU は #1195 に委ねる。）
