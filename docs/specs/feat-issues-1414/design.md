# Design

FE-5「error banner の session スコープ化」の実装設計。requirements.md / behavior.md を正本とし、error banner の状態と自己回復 policy を Rust に置き、frontend を backend-owned state の mirror に限定する。

## 概要

従来の `AgentChatState.error` は全 session で共有されており、他 session の `UPSERT_SESSION` で消える、複数 pane に同じエラーが出る、という問題があった。

本設計では次の境界に変更する。

- Rust の `AgentSessionNoticeUsecase` が session_id 別の transient notice と、失敗操作・成功操作の対応判定を所有する。
- Tauri command は session_id と操作結果を usecase に渡し、更新後の session notice snapshot を返す。
- frontend reducer は snapshot の `message | null` を session_id 別に mirror するだけとし、last-write-wins や同種成功判定を持たない。
- `BoundSessionChat` は表示対象 session の mirror だけを `ChatSessionView` に渡す。
- list / init / create / `sendMessage(null)` は明示的な対象 session を持たないため、active session の banner へ変換しない。

## 変更対象

- `src-tauri/src/usecase/agent_session/notice.rs`
  - session notice の mutation usecase と更新規則を実装する。
  - `SessionStore` の state-change listener と結合し、Closed / Archived への全遷移で notice を破棄する。
  - notice 変更 listener を公開し、backend lifecycle 経路の snapshot delta を client surface へ配信可能にする。
  - session 間の独立、同種成功だけの自己回復、last-write-wins、dismiss / removal を単体テストする。
- `src-tauri/src/usecase/agent_session/notice_state.rs`
  - mutation usecase と QueryService が共有する操作 enum、in-memory state、単調 revision、read model を保持する。
  - Command / Query のどちらにも依存しない中立な共有状態として構成する。
- `src-tauri/src/usecase/agent_session/notice_query_service.rs`
  - 状態を変更せず現在の session notice snapshot を返す read 専用 QueryService を実装する。
- `src-tauri/src/adaptor/protocol/agent_session_notice.rs`
  - command input / response と event payload が共有する唯一の wire message 型を定義する。
- `src-tauri/src/adaptor/controller/command/agent_session/notice.rs`
  - `update_agent_session_notice` command と `get_agent_session_notice` query command を定義する。
- `src-tauri/src/lib.rs` / agent-session command registration
  - usecase を process state として組み立て、command と lifecycle cleanup / snapshot event listener を登録する。
- `src/hooks/useSessionStore.ts`
  - notice command の typed invoke wrapper を公開する。
- `src/hooks/agentChatReducer.ts`
  - `sessionErrors: Record<string, string>` を Rust snapshot の mirror として保持する。
  - `SYNC_SESSION_ERROR` は指定 session の値を反映するだけとする。
  - `UPSERT_SESSION` / `SET_ACTIVE_SESSION_ID` は notice mirror を変更しない。
  - `CLEANUP_SESSION` で当該 session の mirror を破棄する。
- `src/hooks/useAgentChat.ts`
  - 明示的な session 操作の結果を notice command へ渡し、返された snapshot を dispatch する。
  - session 非依存・生成前処理を session banner 経路から外す。
  - close / archive 成功後の後処理エラーを削除済み session の操作失敗として扱わない。
- `BoundSessionChat.tsx` / `ChatSessionView.tsx`
  - session 別 selector と dismiss callback を使い、対象 pane の banner だけを描画する。

## アーキテクチャと責務分割

### Rust usecase

`AgentSessionNoticeUsecase` は process 内で次の最小状態だけを保持する。

```rust
AgentSessionNoticeState {
    revision: u64,
    notices: HashMap<session_id, StoredAgentSessionNotice { operation, message }>,
}
```

更新規則は usecase が所有する。

1. `Failure`: 同一 session の notice を新しい `{ operation, message }` で置換する（last-write-wins）。
2. `Success`: 現在の notice の operation が成功 operation と一致する場合だけ削除する。
3. `Dismiss`: 当該 session の notice を削除する。
4. `RemoveSession`: 当該 session の notice を削除する。

状態が変化するたび process 内の単調 revision を増加させ、mutation response と event に同じ snapshot を載せる。読み取りは専用 `AgentSessionNoticeQueryService` が現在の revision と notice を返し、mutation enum には含めない。global revision を snapshot の順序識別子として使うことで、session 除去後に revision tombstone map を保持せず full-retention を避ける。

異なる session のエントリには触れない。session 除去時に削除するため full-retention にはしない。この usecase は Tauri 固有型に依存せず、将来の WebSocket / daemon surface からも同じ policy を利用できる。

Closed / Archived への遷移は `SessionStore` の全経路共通 state-change listener から `RemoveSession` へ接続する。frontend command が session 除去を申告することには依存しない。復元・アーカイブ等の stored lifecycle usecase は自身の成否を notice usecase へ渡し、`WorkspaceList` や workspace-node gateway のように React hook を通らない caller にも同じ policy を適用する。

### Controller / protocol

frontend は次の update message を `update_agent_session_notice` に送る。

```ts
type AgentSessionNoticeUpdate =
	| { action: "failure"; operation: AgentSessionNoticeOperation; message: string }
	| { action: "success"; operation: AgentSessionNoticeOperation }
	| { action: "dismiss" }
	| { action: "remove_session" };
```

command は usecase 更新後の snapshot を返す。

```ts
interface AgentSessionNoticeSnapshot {
	sessionId: string;
	revision: number;
	notice: { message: string } | null;
}
```

operation / update / snapshot の message 型は `adaptor/protocol/agent_session_notice.rs` に集約し、command response と event payload の serialize shape を一致させる。operation の妥当性と recovery 対応は Rust enum / usecase が正本であり、frontend に source 一致判定を置かない。

backend lifecycle から notice が変化した場合は `agent-session-notice-changed` event で同じ snapshot を配信する。frontend は command response と event のどちらも `SYNC_SESSION_ERROR` へ投影するだけで、独自の recovery 判定を行わない。

### Frontend mirror

```ts
interface AgentChatState {
	sessionErrors: Record<string, string>;
	sessionErrorRevisions: Record<string, number>;
}

type AgentChatAction =
	| { type: "SYNC_SESSION_ERROR"; sessionId: string; revision: number; message: string | null };
```

`SYNC_SESSION_ERROR` は既知 revision 以下の snapshot を無視する。新しい snapshot の非 null message を当該キーへ格納し、null なら当該キーだけを除去する。他 action による暗黙 clear は行わない。

`getSessionError(sessionId)` は `state.sessionErrors[sessionId] ?? null` を返す。`dismissSessionError(sessionId)` は backend へ `dismiss` を送り、返された null snapshot を mirror する。

pane が session を表示対象として register した時は `get_agent_session_notice` で backend の現在値を取得する。これにより frontend context が remount しても backend-owned notice から mirror を再構築でき、遅延 query response が新しい event を上書きすることもない。

## 対象操作

session notice に含めるのは、失敗時点で明示的な対象 session_id を持つ操作だけである。

| 操作 | Rust operation | 対象 session |
|---|---|---|
| 既存 session への送信 | `send` | 引数 `sessionId` |
| session 選択 / 読み込み | `load_session` | 引数 `sessionId` |
| 過去メッセージ読み込み | `load_older` | 引数 `sessionId` |
| キューのキャンセル | `cancel_queue` | 引数 `sessionId` |
| session クローズ | `close_session` | 引数 `sessionId` |
| session 復元 | `restore_session` | 引数 `sessionId` |
| session アーカイブ | `archive_session` | 引数 `sessionId` |
| session フォーク | `fork_session` | 元 sessionId |
| session タイトル変更 | `set_title` | 引数 `sessionId` |
| パーミッション応答 | `respond_permission` | 引数 `sessionId` |
| Agent 変更 | `set_backend` | 引数 `sessionId` |

次は session notice の対象外とする。

| 処理 | 理由 / error surface |
|---|---|
| `refreshSessions` / `refreshClosedSessions` | worktree/list scope。background 実行時の active session と無関係 |
| `initSessions` | worktree 初期化 scope。active session をまだ確定できない |
| `createNewSession` | 生成前 scope。既存 API の null 戻り値を維持 |
| `createNewWorkspaceSession` | 生成前 scope。呼び出し元の creation status へ reject を返す |
| `sendMessage(null)` | 送信先 session をまだ持たない。以前の active session には表示しない |

## 処理フロー

### 失敗と自己回復

明示 session 操作が失敗した場合、hook は `failure` update を送り、Rust が返した snapshot を mirror する。同じ session・同じ operation が成功した場合は `success` update を送り、Rust が一致を判定する。

これにより次を保証する。

- session A の失敗は session B の成功や turn event で変化しない。
- session A で別種操作が成功しても A の既存 notice は消えない。
- session A で同種操作が成功した場合だけ A の notice が消える。
- 同一 session の別の失敗は最新 message へ置換される。

### close / archive-open 成功後の処理

close / archive-open は session 操作 API の try/catch を、成功後の adjacent session 読み込み・一覧 refresh から分離する。

1. close / archive-open API が失敗した場合だけ、削除対象 ID に対応 operation の failure を記録する。
2. API が成功したら backend notice を `remove_session` で削除し、frontend の `CLEANUP_SESSION` を行う。
3. adjacent session の `getSession` が失敗した場合は、削除済み ID ではなく adjacent session ID の `load_session` failure として記録する。
4. list refresh は session banner を生成しない。

したがって API 成功後に後処理が失敗しても、表示面のない削除済み ID の notice は復活しない。

### 表示と dismiss

`BoundSessionChat` は自身の `session.id` で `getSessionError` を呼び、`ChatSessionView` に per-session message を渡す。複数 pane が同時 mount されても別 session の message は参照しない。

ユーザーが × を押すと `dismissSessionError(session.id)` が backend へ dismiss を要求し、返された snapshot を mirror する。他 session の notice には影響しない。

## エラー処理

- notice command の同期自体が失敗した場合は console error として明示し、frontend 独自 policy で state を推測更新しない。
- session 非依存処理の失敗は session banner に変換しない。既存の戻り値 / reject 契約を維持し、background 処理は console error を残す。
- error message 文面は既存の session 操作文面を維持する。
- `CLEANUP_SESSION` は local mirror も除去し、閉じた session の表示不能 notice を残さない。

## テスト方針

### Rust usecase

- session A / B が独立して notice を保持する。
- 同一 session の同種成功だけが notice を clear する。
- 別種成功は既存 notice を保持する。
- 最新失敗が同一 session の旧 notice を置換する。
- dismiss / session removal が当該 notice を破棄する。

### Frontend reducer / component

- `UPSERT_SESSION` / `SET_ACTIVE_SESSION_ID` が mirror を変更しない。
- `SYNC_SESSION_ERROR` が対象 session だけを反映する。
- `CLEANUP_SESSION` が対象 session の mirror を破棄する。
- session B の error が A pane に表示されず、B の turn 開始後も A banner が残る。
- dismiss が表示対象 session ID で呼ばれる。

### Hook regression

- 送信失敗後、同一 session の送信成功で clear し、別 session の成功では保持する。
- background list failure、workspace session 作成失敗、`sendMessage(null)` 失敗が active session に混線しない。
- close / archive-open API 成功後に adjacent `getSession` が失敗しても、削除済み ID の `getSessionError` は null のままである。

## リスク

- frontend と Rust の operation 名に不整合があると command の deserialize で検出される。TypeScript union と Rust enum を同じ protocol 名で固定し、両層のテストと build で確認する。
- backend notice state と frontend mirror の同期失敗時に独自 fallback を行うと source of truth が分裂するため、frontend は推測更新せずエラーを記録する。
- command response / event / query が逆順で届いても、frontend は revision が既知値以下の snapshot を無視する。
- process 内 map は session 除去時に必ず削除し、閉じた ID を retention しない。

## Open Questions

なし。
