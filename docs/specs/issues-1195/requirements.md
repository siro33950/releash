# requirements — メモリ削減: フロント agent チャットの全メッセージ保持を退避/仮想化する (C6 / #1191 から分離)

対象 ISSUE: #1195
関連: #1191（広域調査・候補 C6 の正本）, #1194（C1 / Rust 側 streaming_parts 解放）, #1213（M2 / session summary index + message paging。**マージ済み**, commit 8cd3efe8）
正本ドキュメント: `docs/releash-performance-architecture-audit.md`
Spec ディレクトリ: `docs/specs/issues-1195/`

## 背景と目的

### 背景

`#1191` のメモリ枯渇広域調査で特定した無制限増大経路のうち、フロント（webview）側の常駐型増幅が候補 **C6** である。`#1191` 本体は振る舞い不変・低リスクな Rust 側純粋削減（A 群）に絞ったため、表示挙動・履歴の見え方に関わる設計判断を要する C6 を本 ISSUE に分離した。

#### #1213 の完了により前提が更新された（裏取り済み）

本 ISSUE 起票時点では Rust 側に部分取得 API が存在せず「初回に全 message を含む `ChatSession` を丸ごと hydrate する」ことが C6 の中核問題だったが、その後 **#1213（M2: session storage を summary index + message paging に再設計）がマージ済み**（commit 8cd3efe8）となり、初回 hydrate の有界化はすでに完了している。現状を実コードで確認した結果は以下のとおり:

- Rust 側に部分取得 API `get_session_page(session_id, cursor, limit)` が**実装済み**である（`src-tauri/src/adaptor/controller/command/agent_session/session.rs:515`, `.../gateway/agent_session/session_storage/message_store.rs:68`, `.../infrastructure/agent_session/runtime/bridge_common.rs:4692`）。runtime streaming overlay・token usage metadata の適用、legacy flat JSON の移行、index 欠落の自己修復まで含む。
- フロントは初回ロード時、`getSession` が返す `initialPage`（**最新 `INITIAL_SESSION_PAGE_LIMIT = 50` 件のみ**, `src/hooks/useSessionStore.ts:39`）で hydrate し、`pageStateRef` で `nextCursor` / `hasMore` を管理する（`src/hooks/useAgentChat.ts:276,288`）。
- 過去方向のスクロールバックは `getSessionPage(sessionId, cursor)` を呼び、得た page を `PREPEND_MESSAGES` で `session.messages` の先頭に追記する（`src/hooks/useAgentChat.ts:442-468`, `agentChatReducer.ts:60,170-178`）。
- すなわち「session を開いた瞬間に全 message body が常駐する」という当初の中核問題は #1213 で解消済みである。

#### #1213 完了後に残る無制限増大経路（本 ISSUE の対象）

`sessionsById: Record<string, ChatSession>`（`agentChatReducer.ts:29`）から message body が解放されるのは依然として `CLEANUP_SESSION`（session の close/delete 時のみ。`useAgentChat.ts:641,746`）に限られ、**退避（eviction）の経路が存在しない**。このため、初回 50 件制限の後にも次の2経路で webview メモリが無制限に増え続ける:

1. **アクティブ session のスクロールバック累積**: `getSessionPage` で過去方向に遡って `PREPEND_MESSAGES` した古い message が drop されない。長い単一会話を遡るほど `session.messages` が O(N) に積み上がる。
2. **非アクティブ session の常駐**: 開いた session の body は close/delete されるまで `sessionsById` に残り続ける。同時に開く（切り替えて使う）session 数だけ常駐量が倍増する。

### 目的

開いている agent チャット session の webview メモリ常駐量を、会話長・同時 session 数に対して線形に増え続けない構造へ是正する。表示・履歴・既読の外部から観測可能な振る舞いを変えずに、webview が保持する message body 量を有界化する。#1213 が導入済みの paging API（`get_session_page` + cursor）を再供給経路として活用し、新規の永続データ型・プロトコル型を増やさずに実現する。

## スコープ

本 ISSUE は、上記2経路の常駐量を有界化する **webview 側の退避（eviction）+ 必要時の再供給（再 hydrate）** を導入する。粒度・トリガの細部は `design.md` で確定する。

1. **アクティブ session 内の可視範囲外 message の退避（仮想化）**: スクロールバックで遡って `sessionsById` に積み上がった、可視範囲から外れた古い message body を退避（drop / 軽量プレースホルダ化）し、保持件数を有界に保つ。
2. **非アクティブ（バックグラウンド）session の body 退避**: close/delete されていないが現在表示していない session の `messages` body を退避し、同時保持量を有界に保つ。
3. **退避した message の再供給経路**: 退避した範囲が再表示対象になった時点で、#1213 の `get_session_page(session_id, cursor, limit)` を用いて Rust の session 正本から再取得し、表示を復元する。再供給は既存 paging API の利用に留め、新規 read API の追加は行わない。
4. **責務配置**: 退避判定・再供給に必要なロジックは `rust-first-logic` 方針に従って配置する。フロントは表示・入力・invoke・表示用フォーマットに徹し、新たなビジネスロジックを増やさない。

## 非スコープ

- Rust 側の session summary index 化、`get_session_page` 等の paging API 新設、fork の copy-on-write 化 → `#1213`（マージ済み）の責務。本 ISSUE は #1213 が提供する既存 API を利用するに留め、再設計・拡張は行わない。
- Rust 側 `streaming_parts` のターン完了時解放 → `#1194`（C1）の責務。
- Rust 側の `SessionStore` cache / clone / serialize 削減（C9）→ `#1191` A 群の責務。
- ストリーミング表示・イベント永続化・session の保存正本フォーマットの変更。表示内容・永続化イベント・履歴復元の外部仕様は不変に保つ。
- 初回ページロード件数（`INITIAL_SESSION_PAGE_LIMIT`）・cursor paging の取得方式そのものの変更（#1213 で確定済み。退避/再供給がこれを利用するだけで、ページング契約は変えない）。
- remote UI（`src/remote/`）への退避/仮想化の適用。本 ISSUE の主対象は agent チャット（`AgentChatPanel` 系）であり、remote は必要なら別途扱う。
- 新規の永続データ型・プロトコル型の追加（既存 `get_session_page` の利用で完結する想定）。

## 要求事項

1. 開いている session について、webview が常駐保持する message body 量が、会話の総 message 数および同時に開いている session 数に対して**線形に増え続けない**こと（有界化）。アクティブ session では、上端を離れた後（下方向スクロール）または present 復帰時に、drop 可能な古い page が退避され、live tail が `RETAINED_MESSAGE_CAP` 以下なら常駐量が `RETAINED_MESSAGE_CAP` へ収束する。ただし live tail（最新 page・進行中レンジ）自体が cap を超える長尺 session では、要求 4 により live tail 超過分は退避せず例外として保持し、drop 可能な古い page のみ退避する。連続した純上方向スクロールバック中（`oldest_visible_index` が 0 近傍）は、anti-thrash のため contiguous な窓が一時的に cap を超えて増大しうることを明示的な制約として許容する。非アクティブ session では body 退避により同時保持 session 数の増加に対して有界化する。
2. message body を退避しても、ユーザーから見た表示・スクロールバック・既読履歴の**外部観測可能な振る舞いが変わらない**こと（退避は実装内部の最適化であり、表示は退避前と同一に再現できる）。
3. 退避済み message が再表示対象になった場合、#1213 の `get_session_page` を用いて Rust 側の session 正本から再供給し、表示を復元できること。再供給に整合性の問題（重複・欠落・順序崩れ）を生じないこと。
4. ストリーミング中・ターン進行中のアクティブ session の表示・更新が、退避/仮想化によって破壊されないこと（`SET_STREAMING_MESSAGE` / `ADD_MESSAGE` / `MARK_AGENT_TURN_COMPLETED` の挙動が不変。退避対象はアクティブな進行中レンジを含めない）。
5. 退避判定・再供給に必要なロジックは `rust-first-logic` 方針に従って配置すること。フロントに新たなビジネスロジックを増やさない。
6. #1213 が確定した paging 契約（`initialPage` / `get_session_page` / cursor / `INITIAL_SESSION_PAGE_LIMIT`）と矛盾しないこと。退避/再供給は既存契約の上に構築し、ページング方式の再設計・二重実装を生まない。
7. 既存テスト・lint が green であること（`pnpm test` / `pnpm lint` / `cargo test` / `cargo clippy -- -D warnings`）。退避/再供給の振る舞い不変は新規テストで固定する。

## 受け入れ基準の概要

- 長い会話（多数 message）をスクロールバックで遡った後に上端を離れる、または present へ戻った状態、および複数 session を開いて切り替えた状態で、webview が保持する message body 量が有界である（live tail が cap 以下なら `RETAINED_MESSAGE_CAP` へ収束し、live tail が cap を超える場合は live tail を保持したまま古い page のみ退避される）ことを確認できる。
- 退避と再供給を経た後でも、退避前と同一の表示・スクロールバック・既読履歴が再現される（振る舞い不変を単体テストで固定）。
- 退避済みレンジへの再スクロール時、`get_session_page` 経由の再供給で重複・欠落・順序崩れなく表示が復元される。
- ストリーミング中・ターン進行中の表示更新が退避/仮想化により退行しない。
- `rust-first-logic` 方針に反するロジックがフロントに追加されていない。
- 既存テスト・lint がすべて green。
- 具体的な計測手順・閾値（保持上限件数・session 数上限等）・観測点は `behavior.md` で定義する。

## 仮定

- session ストア（Rust）が message の正本であり、webview の保持は再取得可能なキャッシュである。退避した body の再供給に整合性問題は生じない（#1213 の `get_session_page` が runtime streaming overlay・metadata を含めて正本を返す前提）。
- 退避/再供給の対象は agent チャット（`AgentChatPanel` 系）であり、remote UI（`src/remote/`）は本 ISSUE の主対象外。
- 退避のトリガ・閾値（非アクティブ化時に即時か、保持 message 数 / session 数の上限超過時か、未参照時間ベースか）と、退避単位（body 全体 drop か軽量プレースホルダ化か）は、要求（有界化・振る舞い不変・再供給整合）を満たす範囲で `design.md` の裁量により確定する。
- 再供給は既存 `get_session_page` と `getSession.initialPage` で完結する。中間レンジの forward 再供給 API や新規 read API は追加しない。

## Open Questions

なし（起票時の3点はいずれも解消済み）。

- #1213 との責務境界・再供給 API の先行実装要否 → **解消**: #1213 がマージ済みで `get_session_page` + cursor paging が実装済み。#1195 は既存 API を利用し、新規 API を追加しない。
- 退避の粒度（非アクティブ session 全体 / アクティブ session 内可視範囲外 message）→ **解消**: 双方を対象とする（ユーザー合意済み）。
- 退避トリガ（非アクティブ化時 / 上限超過時 / 未参照時間ベース） → **解消**: 要求を満たす範囲で `design.md` の裁量により確定する（仮定参照）。
