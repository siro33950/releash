# design — メモリ削減: フロント agent チャットの常駐 message body を退避/再供給で有界化する (#1195)

対象 ISSUE: #1195
関連 Spec: `requirements.md`, `behavior.md`（同ディレクトリ）
前提（裏取り済み）: #1213（M2: session storage を summary index + message paging に再設計）がマージ済み。`get_session_page(session_id, cursor, limit)` と `getSession` の `initialPage`（最新 50 件）が利用可能。

## 概要

webview（React）が常駐保持する agent チャットの message body 量を、会話の総 message 数および同時に開いている session 数に対して線形に増え続けない構造へ是正する。

#1213 により「初回 hydrate の有界化（最新 50 件）」は完了済みだが、以下 2 経路で常駐量が無制限に増え続ける:

1. **アクティブ session のスクロールバック累積**: `loadOlderMessages` が `PREPEND_MESSAGES` した古い page が drop されず `session.messages` が O(N) に積み上がる。
2. **非アクティブ session の常駐**: 表示していない session の `messages` body が `CLEANUP_SESSION`（close/delete 時のみ）まで `sessionsById` に残り続ける。

本設計は、この 2 経路に対して **Rust usecase が計画する webview キャッシュ退避（eviction）+ 既存 paging による再供給（再 hydrate）** を導入する。新規の永続データ型・WebSocket プロトコル型・read API は追加せず、#1213 が確定した paging 契約（`getSession.initialPage` / `get_session_page` / cursor）の上に構築する。退避 policy（保持上限、退避対象、cursor rewind、非アクティブ session の優先順位）は Rust 側 `plan_agent_chat_eviction` Tauri command に閉じ、フロントは scroll/window/viewable の観測値を渡して plan を反映する。

### 設計の前提となる cursor 意味論（実コードで確認済み）

再供給経路の設計を確定させるため、`get_session_page` の cursor 意味論を実装で確認した。

- `PageCursor` は `u64`（= message の `seq`）を文字列化した不透明トークン（`session.rs:515`）。フロントは `string | null` として扱う。
- `read_page_from_index`（`message_store.rs:406`）は **`entry.seq < cursor` を満たす message のうち末尾 `limit` 件**（= cursor の直前に位置する、より古い `limit` 件）を返す。
- 返り値 `next_cursor` は「返した page の先頭（最も古い）message の seq」。`has_more=false` のとき `null`。
- すなわち cursor が表現できるのは **「ある境界より古い方向（backward）への取得」のみ**。「より新しい方向（forward）への cursor 取得」は API に存在しない。
- 同一 index に対し **同じ `requestCursor` で再呼び出しすれば同じ page が決定的に返る**（再供給の冪等性が成立）。

この制約から、再供給は次の 2 経路に限定される:

- **backward（より古い page）**: `get_session_page(cursor)` で取得（既存 `loadOlderMessages` がこれ）。
- **newest（最新 page）**: `getSession` の `initialPage`（最新 50 件）で取得（既存 `selectSession` / `loadSession` がこれ）。

「中間レンジを forward に再取得する」操作は存在しないため、**退避は常に「最も新しい live tail を残し、より古い側（または body 全体）を drop し、backward / newest 経路で再供給できる範囲に限定する」** という形に統一する。

## 変更対象

- `src-tauri/src/usecase/agent_session/session/message_window.rs`
  - 退避 policy の定数（`RETAINED_MESSAGE_CAP` / `MAX_HYDRATED_SESSIONS`）、request/plan 型、active window の page 単位 drop、非アクティブ session body 退避の候補選定を実装する。
  - 非アクティブ session は request の `evictionRank`（フロントが観測した参照順）を Rust 側で sort し、JS object の列挙順に依存しない。
- `src-tauri/src/adaptor/controller/command/agent_session/session.rs`
  - `plan_agent_chat_eviction` Tauri command として Rust usecase の plan を公開する。
- `src/hooks/agentChatReducer.ts`
  - 退避用 action（後述 `EVICT_SESSION_BODY` / `EVICT_OLDER_MESSAGES`）と reducer case を追加。
  - 既存 `PREPEND_MESSAGES` / `ADD_MESSAGE` / `SET_STREAMING_MESSAGE` / `MARK_AGENT_TURN_COMPLETED` / `CLEANUP_SESSION` の挙動は不変。
- `src/hooks/useAgentChat.ts`
  - `pageStateRef` を拡張し、再供給のための loaded page cursor 履歴を保持する。
  - scroll/window/viewable/loading/turnPhase/messageCount/`evictionRank` を観測して `plan_agent_chat_eviction` に渡し、返った plan に従って reducer action と `pageStateRef` rewind を適用する。
  - viewable registry の遷移（非 viewable 化）を退避計画のトリガとして利用する。
- `src/components/panels/AgentChatPanel/ChatSessionView.tsx`
  - 既存 `handleScroll`（`scrollTop < 80` / 下端 100px 判定）に「上端から十分離れた」という観測点を追加する。表示層は観測値を渡すだけで、退避可否は Rust plan に従う。
- `src/components/panels/AgentChatPanel/BoundSessionChat.tsx`
  - 既存の viewable 登録/解除（`useEffect`）と `loadSession`（再 hydrate）はそのまま再供給経路として機能。退避トリガの呼び出し接続のみ追加。

## アーキテクチャと責務分割

### 責務境界（rust-first-logic との整合）

- **message の正本・順序・dedup・streaming overlay・token usage metadata の構築**: すべて Rust（`get_session_page` / `getSession`）。本 ISSUE で一切複製しない。再供給は既存 API を呼ぶだけ。
- **webview キャッシュ退避 policy（閾値、退避対象、cursor rewind、非アクティブ session の優先順位）**: Rust usecase（`message_window.rs`）。フロントは観測値を組み立てて Tauri command を呼び、返却 plan を適用するだけに留める。
- **webview キャッシュ窓の状態保持と UI 観測**: フロント。`pageStateRef` / scroll 位置 / viewable registry / turnPhase / loading 状態を request に含める。退避の判定結果は Rust plan を正とする。
- **退避判定に使う閾値（保持上限件数・同時保持 session 数の上限）**: Rust usecase の定数として集中管理する。フロント定数や JS object 列挙順には依存しない。

### 経路 1: アクティブ session のスクロールバック累積の有界化

`session.messages` は「最新の live tail を必ず含み、上方向（より古い）へ連続して伸びる contiguous な窓」である。退避はこの窓の **古い側（上端）を page 単位で drop** し、backward 再供給で復元する。

- **退避単位**: page 単位の drop（軽量プレースホルダ化ではなく実体 drop）。DOM は既に `@tanstack/react-virtual`（`ChatSessionView.tsx`, `overscan: 8`）で仮想化済みのため、常駐量を支配するのは `session.messages` JS 配列そのもの。よって配列要素を実体 drop することが有界化の本質。プレースホルダは「ある中間レンジが欠けている」状態を生み、本設計の「contiguous 窓」前提と forward 再供給不在の制約に反するため採らない。
- **退避トリガ**: アクティブ session について、
  1. 保持 message 数が上限 `RETAINED_MESSAGE_CAP` を超え、かつ
  2. ユーザーが上端から十分下へスクロール済み（= 直前にスクロールバックした古い prefix が可視範囲から離れた）
  のとき、最古側の page を 1 つ以上 drop する。条件 2 により「いま読み込んだばかりの古い prefix を即座に drop → 即再取得」という thrash を防ぐ（anti-thrash）。
  退避は live tail（最新 page・ストリーミング中/ターン進行中レンジ）を**決して含めない**（要求 4 / behavior「ストリーミング中は退避対象外」）。
- **再供給**: drop した最古 prefix は backward 経路で復元する。drop と同時に backward-paging cursor を **drop した最古 page を再取得できる位置へ rewind** し、`hasMore=true` に戻す。以後、ユーザーが上端へ再スクロールすれば既存 `loadOlderMessages` がそのまま再取得・`PREPEND_MESSAGES` する。再供給専用コードは増やさない。

### 経路 2: 非アクティブ session の常駐の有界化

- **退避単位**: body 全体 drop（`messages` を空配列化）。summary（`sessions` / `sessionsById` のシェル: id・state・backendId 等の軽量フィールド）は残し、session は **close/delete されず開いた状態を保つ**（behavior「退避は close/delete を引き起こさない」）。
- **退避トリガ**: ある session が「アクティブでもなく、現在 viewable でもない」状態へ遷移したとき（= `BoundSessionChat` unmount で viewable registry から外れた、または active 切替で背面に回った）。同時に body を保持する session 数は「active + 現在 viewable」に限定され、Rust plan は上限 `MAX_HYDRATED_SESSIONS` を超えた分を、request の `evictionRank` が小さい（最終参照が古い）非アクティブ session から drop する。同 rank の tie-break は session id で安定化する。
- **再供給**: 既存 `selectSession` / `loadSession`（→ `getSession` の `initialPage` = 最新 50 件）と `rememberInitialPage`（pageState 再初期化）がそのまま再 hydrate する。`BoundSessionChat` は sessionId 表示時に `loadSession` を呼ぶ（既存実装）ため、背面→前面の切替で自動的に再供給される。再供給専用コードは増やさない。

### 退避が「外部観測上不可視」である根拠

- 経路 1: live tail（最新・ストリーミング・ターン進行中）は常に保持されるため、進行中表示・新規追加・ターン完了確定（`SET_STREAMING_MESSAGE` / `ADD_MESSAGE` / `MARK_AGENT_TURN_COMPLETED`）は退避の影響を受けない。古いレンジは backward 再供給で退避前と同一 message・同一順序に復元される（cursor 冪等性）。
- 経路 2: 再表示時に正本から最新 page を再 hydrate するため、切替前と同一の表示・スクロール可能履歴が復元される。
- 既読/履歴の状態は Rust 正本（および `getSession` が返す metadata）から再構築されるため、退避前後でユーザーから区別できない。

## データモデルまたは型

新規の永続型・WebSocket プロトコル型・read API は追加しない。退避判定用に Tauri command の request/plan 型を Rust usecase に追加し、フロントは同じ JSON shape を TypeScript 型として mirror する。

### Rust request / plan 型（`message_window.rs`）

```rust
pub struct ActiveMessageWindowObservation {
    session_id: String,
    message_count: usize,
    oldest_visible_index: usize,
    loaded_pages: Vec<LoadedMessagePage>,
    turn_phase: TurnPhase,
}

pub struct HydratedSessionObservation {
    session_id: String,
    message_count: usize,
    eviction_rank: u64,
    protected: bool,
    loading: bool,
}

pub struct AgentChatEvictionPlanRequest {
    active: Option<ActiveMessageWindowObservation>,
    sessions: Vec<HydratedSessionObservation>,
}
```

- `evictionRank` はフロントが session 表示・load・送信などの参照時に単調増加で更新する観測値であり、Rust 側が非アクティブ session 候補を「最終参照が古い順」に sort/select するためだけに使う。未記録 session は `0` として扱い、同 rank は Rust 側で session id により安定化する。
- `AgentChatEvictionPlan` は active window の `count` / `nextCursor` / `loadedPages` rewind と、body 全体を退避する `evictSessionIds` を返す。
- `RETAINED_MESSAGE_CAP = 200`、`MAX_HYDRATED_SESSIONS = 3` は Rust usecase の定数として持つ。

### `pageStateRef` の拡張（`useAgentChat.ts`）

現状:

```ts
Record<string, { nextCursor: string | null; hasMore: boolean; loading: boolean }>
```

拡張案（経路 1 の rewind に必要な loaded page cursor 履歴を追加）:

```ts
Record<string, {
  nextCursor: string | null;   // 次に backward 取得する境界（既存）
  hasMore: boolean;            // 既存
  loading: boolean;            // 既存
  // 新規: 読み込み済み page を新しい→古い順に記録。各 page を再取得する
  // requestCursor（その page を取得した際に渡した cursor）と message 件数。
  // 退避時はここから drop する page を pop し、nextCursor を rewind する。
  loadedPages: Array<{ requestCursor: string | null; count: number }>;
}>
```

- 初回 hydrate（`rememberInitialPage`）で `loadedPages = [{ requestCursor: null, count: initialPage件数 }]`。
- `loadOlderMessages` 成功時に `loadedPages.push({ requestCursor: 使用した cursor, count: page.messages件数 })`。
- 経路 1 退避時: 最古 page から `k` 件 pop し、その合計件数を `session.messages` 先頭から drop。`nextCursor = pop した中で最も新しい page の requestCursor`、`hasMore = true` に rewind（再取得の冪等性は cursor 意味論で担保）。

### reducer action（`agentChatReducer.ts`）

```ts
// 非アクティブ session の body 全体退避（summary シェルは残す）
| { type: "EVICT_SESSION_BODY"; sessionId: string }
// アクティブ session の最古側 message を先頭から count 件退避
| { type: "EVICT_OLDER_MESSAGES"; sessionId: string; count: number }
```

- `EVICT_SESSION_BODY`: `sessionsById[id]` を `{ ...session, messages: [] }` に置換（id 等のシェルは保持、`CLEANUP_SESSION` とは異なり turnPhase 等の per-session map は消さない）。
- `EVICT_OLDER_MESSAGES`: `messages = messages.slice(count)`（先頭=最古を drop）。`count` は呼び出し側が page 境界に揃えて算出する。

### 退避ポリシー定数（Rust usecase）

```rust
RETAINED_MESSAGE_CAP   // アクティブ session が常駐保持する message 上限件数
MAX_HYDRATED_SESSIONS  // body を常駐保持する session 数の上限（active + viewable を優先）
```

**確定方針**: 退避 policy は `message_window.rs` の Rust 定数として集中管理する。値は `INITIAL_SESSION_PAGE_LIMIT` の数倍程度（`RETAINED_MESSAGE_CAP = 200`, `MAX_HYDRATED_SESSIONS = 3`）を初期値とし、有界化・anti-thrash を満たす範囲で調整する。アプリ設定や新規永続 config は追加しない。

## 処理フロー

### 経路 1（アクティブ session スクロールバック）

1. ユーザーが上端付近へスクロール → `ChatSessionView.handleScroll` が `loadOlderMessages` を起動（既存）。
2. `loadOlderMessages` が `get_session_page(nextCursor)` を呼び、`PREPEND_MESSAGES` + `loadedPages.push`（既存 + 拡張）。
3. ユーザーが下方向へスクロールし、上端の古い prefix が可視範囲から離れる（`handleScroll` が観測）。
4. `evictActiveOlderMessages` が active observation（session id、message_count、oldest_visible_index、loadedPages、turnPhase）を `plan_agent_chat_eviction` に渡す。`nextCursor` / `hasMore` は退避 plan の出力であり、入力には含めない。
5. Rust plan が保持件数 > `RETAINED_MESSAGE_CAP`、上端離脱、turnPhase idle、loaded page 境界を満たす場合だけ active eviction plan を返す:
   - drop する最古 page 群を `loadedPages` から決定（live tail は残す）。
   - rewind 後の `nextCursor` / `hasMore` / `loadedPages` を返す。
6. フロントは plan に従って `EVICT_OLDER_MESSAGES` を dispatch（先頭から該当件数 drop）し、`pageStateRef` を Rust plan の値に更新する。
7. 後でユーザーが再び上端へスクロール → 手順 1〜2 がそのまま再供給（退避前と同一 page を冪等取得）。

### 経路 2（非アクティブ session）

1. session が active から外れる、または `BoundSessionChat` unmount で viewable registry から外れる。
2. `evictInactiveSessions` を起動:
   - フロントは「active + 現在 viewable」を `protected=true`、loading 中を `loading=true`、参照順を `evictionRank` として観測値を渡す。
   - Rust は body を保持している session 数が `MAX_HYDRATED_SESSIONS` を超えた場合、保護集合外かつ非 loading の候補を `evictionRank` 昇順で sort し、超過分の `evictSessionIds` を返す。
   - フロントは返却された session について、適用直前にも active/viewable/loading でないことを再確認し、`EVICT_SESSION_BODY` を dispatch する。
3. 当該 session を再表示（`selectSession` / `BoundSessionChat` 再 mount → `loadSession`）すると `getSession.initialPage` で最新 page を再 hydrate（既存経路）。

### 退避から除外する不変条件

- アクティブ session の live tail（最新 page）、ストリーミング中・ターン進行中の message レンジは経路 1 退避の対象外。
- active session 自体は経路 2 退避の対象外。
- 退避は `messages` 配列の操作のみで、`turnPhases` / `pendingQueues` / `latestTokenUsage` 等の per-session メタや Rust 正本には触れない（`CLEANUP_SESSION` との明確な差異）。

## エラー処理

- 再供給（`loadOlderMessages` / `loadSession`）が失敗した場合は既存のエラーパス（`SET_ERROR` + `pageState.loading` 復帰）を踏襲する。退避によって失われるのは webview キャッシュのみで、正本は不変のため、再試行で復元可能。
- `get_session_page` が `null`（session 消失等）を返す場合は既存どおり `hasMore=false` に確定し、再供給を止める。
- 退避と再供給が競合しないよう、`loading` 中の session は経路 1 退避の対象外とする（dispatch 順序で整合を担保）。
- 重複・欠落・順序崩れ防止: 再供給は既存 `prependMessages` / `appendMessage` の id ベース dedup（`agentChatReducer.ts:162,173`）に依存。退避は contiguous 窓の端のみ操作し、cursor は冪等取得を保証するため、再供給後も id 一意・seq 順序が保たれる。

## テスト方針

配置・粒度は CLAUDE.md の方針（reducer/hook は単体、外部プロセスは実行しない、Tauri API は `vi.mock`）に従う。

### reducer 単体（`agentChatReducer.test.ts` に追加）

- `EVICT_OLDER_MESSAGES`: 先頭 `count` 件のみ drop、tail と件数、参照不変条件を確認。`count=0` / `count>=length` の境界。
- `EVICT_SESSION_BODY`: `messages` 空化、id/state 等シェル保持、他 session 不変、per-session メタ非破壊を確認。
- 既存 `PREPEND_MESSAGES` / `ADD_MESSAGE` / `SET_STREAMING_MESSAGE` / `MARK_AGENT_TURN_COMPLETED` が退避 action 追加後も不変であること（回帰）。
- 退避 → 再供給（`PREPEND_MESSAGES` / `UPSERT_SESSION`）往復で、退避前と同一 message・同一順序・重複/欠落なしを固定（behavior の「同一内容復元」シナリオ）。

### hook 単体（`useAgentChat.test.ts` に追加）

- `pageStateRef` の `loadedPages` 追跡と、経路 1 退避時の `nextCursor` rewind が再取得を冪等化することを、`get_session_page` モックで確認。
- 経路 1: 上限超過 + 上端離脱時に最古 page が drop され、上端再スクロールで同一 page が再供給される（モック呼び出し列で検証）。
- 経路 2: 非 viewable + active 外への遷移で `evictionRank` を含む request が Rust plan command に渡され、`EVICT_SESSION_BODY` が起動し、再表示で `getSession` 再 hydrate される。
- 有界性: 上端離脱後（`oldestVisibleIndex > 0`）に drop 可能な古い page が退避され、live tail が `RETAINED_MESSAGE_CAP` 以下なら保持件数が cap へ収束することを固定する。live tail が cap を超える場合は live tail を保持したまま古い page の部分 drop plan が返ること、連続純上方向（`oldestVisibleIndex` が 0 近傍）では plan が `None` になり一時増大を許容することを固定する。多数 session を開いても hydrate 数が `MAX_HYDRATED_SESSIONS` を超えないことも assertion で固定する。
- ストリーミング不変: 経路 1 退避タイミングで live tail / 進行中レンジが drop されないこと。

### Rust usecase 単体（`message_window.rs`）

- active window: turnPhase が idle で、保持件数が `RETAINED_MESSAGE_CAP` を超え、可視範囲外の page 境界で drop 可能な場合だけ `ActiveMessageEvictionPlan` を返す。
- active window: streaming / waiting_permission など進行中 turn では plan を返さない。
- inactive sessions: body 保持数が `MAX_HYDRATED_SESSIONS` を超えたとき、protected/loading を除外し、`evictionRank` 昇順（tie は session id）で退避対象を返す。入力 Vec の順序には依存しない。

### lint / 既存テスト

- `pnpm lint` / `pnpm test` / `cargo clippy -- -D warnings` / `cargo test` がすべて green。

## リスクと代替案

- **リスク: 退避↔再供給の thrash**。上限直上で頻繁に drop/再取得が起きるとスクロールが重くなる。→ 経路 1 はヒステリシス（上限 + 上端からの距離条件）と page 単位 drop で緩和。`RETAINED_MESSAGE_CAP` は `INITIAL_SESSION_PAGE_LIMIT` の数倍に設定。
- **リスク: 中間レンジの forward 再供給不能**。cursor が backward しか無いため、live tail を残さず中間だけ drop すると復元不能。→ 退避は常に「最古側 drop（backward 復元）」または「body 全体 drop（newest 復元）」に限定し、中間 drop を設計から排除。
- **リスク: 退避中にストリーミング/新規 message が到達**。→ live tail は退避対象外、`ADD_MESSAGE` / `SET_STREAMING_MESSAGE` は最後尾を操作するため経路 1 の最古側 drop と非干渉。`loading` 中 session は退避対象外。
- **リスク: 非アクティブ session の候補選定が JS object の列挙順に依存する**。→ フロントは `evictionRank` を明示的な観測値として request に含め、Rust usecase が rank 昇順で sort/select する。tie は session id で安定化する。
- **代替案（不採用）: 軽量プレースホルダ化**。中間欠落を生み forward 再供給不在の制約に反するため不採用。実体 drop + backward/newest 再供給に統一。
- **代替案（不採用）: 退避ポリシー閾値をアプリ設定から供給**。現時点ではユーザー設定化の要求がなく、2 つの整数定数のために新規永続 config を増やすのは非スコープに反する。Rust usecase の定数として保持する。

## 仮定

- session ストア（Rust）が message の正本であり、webview 保持は再取得可能なキャッシュ。退避 body の再供給に整合性問題は生じない（#1213 の `get_session_page` / `getSession` が overlay・metadata を含めて正本を返す）。
- 退避単位は **実体 drop**（プレースホルダ化しない）。経路 1 は page 単位の最古側 drop、経路 2 は body 全体 drop。
- 退避トリガは **経路 1 = Rust plan が保持上限超過 + 上端離脱を満たすと判断したとき**、**経路 2 = 非アクティブ化（active 外 かつ 非 viewable）+ Rust plan が hydrate session 数上限超過と判断したとき**。未参照時間ベースのタイマは採らない（イベント駆動で十分かつ単純）。
- 再供給は既存 `get_session_page`（backward）と `getSession.initialPage`（newest）のみで完結し、新規 read API・永続/プロトコル型を追加しない。中間レンジの forward 再供給 API は設計から排除する。
- 対象は agent チャット（`AgentChatPanel` / `BoundSessionChat` / `ChatSessionView`）。remote UI（`src/remote/`）は対象外。
- 「有界」の観測点はテストでの保持件数・hydrate session 数の assertion とする。アクティブ session は、上端離脱後かつ live tail が cap 以下なら `RETAINED_MESSAGE_CAP` へ収束すること、live tail が cap を超える場合は live tail 超過を例外として古い page のみ退避されること、連続純上方向では退避しないことを観測点にする。非アクティブ session は hydrate session 数が `MAX_HYDRATED_SESSIONS` を超えないことを観測点にする。

## Open Questions

なし。

- 退避ポリシー閾値（`RETAINED_MESSAGE_CAP` / `MAX_HYDRATED_SESSIONS`）の配置 → **解消**: Rust usecase の定数として集中管理する。新規永続 config は追加しない。
- 非アクティブ session の退避優先順位 → **解消**: フロントが `evictionRank` を観測値として渡し、Rust usecase が `evictionRank` 昇順（tie は session id）で sort/select する。
