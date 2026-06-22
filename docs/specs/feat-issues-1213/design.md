# Design

対象 Issue: #1213
要求の正本: `requirements.md`（本ディレクトリ）
振る舞いの正本: `behavior.md`（本ディレクトリ）
性能監査の正本: `docs/releash-performance-architecture-audit.md`（M2 / 項目 3・4）

本書は、Agent session storage を **summary index** と **message paging** に分離するための実装設計を定義する。保存フォーマット・cursor 方式・fork 方式・migration 方式といった、`behavior.md` が `design.md` に委ねた内部経路をここで確定する。

---

## 概要

現状の `SessionStore` は `~/.releash/sessions/{session_id}.json` に `ChatSession` 全体を full pretty JSON で保存し、起動時に全 session を `cache: HashMap<String, ChatSession>` に丸ごとロードする。これにより list / get / save / fork / streaming 永続化のすべてが「session 本文の総量」に比例して memory / IO を消費する。

本設計では保存レイアウトを次の 3 系統に分割する。

1. **session メタ（保存正典・summary index）**: message body を含まない session 単位の軽量メタ。一覧取得・復帰識別子はこれだけで成立する。
2. **message body（page 取得対象）**: メッセージ単位に分割保存する本文。cursor + limit で page 単位に読む。
3. **attachment blob（参照取得対象）**: 画像等の binary を message body から外出しし、page には参照のみを載せる。

in-memory cache は full session ではなく **メタのみ**を保持し、body は要求された page だけを disk から読む。これにより list は本文総量に依存せず、get / save / fork / streaming 永続化が本文全量の複製・再書き込みに依存しなくなる。frontend は初期描画で最新 page のみを hydrate し、過去方向のスクロールで cursor 単位に追加取得する。

旧 flat JSON フォーマットは読み取り互換 + 遅延 migration（アクセス時に新レイアウトへ変換）で非破壊に扱う。

---

## 変更対象

### Rust（バックエンド）

- `src-tauri/src/domain/agent_session/`（新規 port）
  - session paging / append / meta update / fork / attachment 取得に必要な repository / gateway trait を定義する。
- `src-tauri/src/usecase/agent_session/session/`
  - `SessionStore` 相当の公開 API は業務手順（list / page / append / save / fork / state/meta update）と port 呼び出しに限定し、保存レイアウトやファイル I/O の詳細を持たない。
- `src-tauri/src/adaptor/gateway/agent_session/`（新規）
  - split layout の read/write、meta cache、diff 保存、hardlink fork、遅延 migration、attachment blob I/O を実装する。
- `src-tauri/src/usecase/agent_session/session/mod.rs`
  - 新規 API DTO / read model 型（`SessionMeta` / `MessageIndexEntry` / `AttachmentRef` / `SessionPage` / `PageCursor`）と summary 変換。保存レイアウトへの分解・再構成は gateway 実装側に閉じる。
- `src-tauri/src/adaptor/controller/command/agent_session/stored_session.rs`
  - `get_session_page` Tauri command を追加。`list_sessions` / `fork_session` は新 read model 経由に差し替え（外部 API シグネチャは原則維持）。
- `src-tauri/src/adaptor/controller/command/agent_session/session.rs`
  - `get_session`（full）経路を、frontend 初期描画向けには最新 page 返却へ寄せる。内部の full 取得は restoration 等の cold path に限定。
- `src-tauri/src/adaptor/controller/command/mod.rs`
  - `get_session_page`（および必要なら `get_session_attachment`）を command 登録。
- `src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs`
  - `persist_streaming_parts` を「対象 message chunk だけを更新する」経路へ変更。`get_session_internal_with_data_dir` の streaming overlay を page 応答にも適用。
- `src-tauri/src/infrastructure/agent_session/runtime/context_restore.rs`
  - 復帰時の messages 再構成を full session 読み出し（cold path）経由に明示。識別子は メタから取得。

### フロントエンド

- `src/hooks/useSessionStore.ts`
  - `getSessionPage(sessionId, cursor, limit)` を追加。初期 hydrate を full `get_session` から最新 page 取得へ置換。
- `src/hooks/useAgentChat.ts`
  - active session の初期取得を page 化。過去方向スクロールで追加 page を要求。
- `src/hooks/useAgentSdkListeners.ts`
  - streaming 反映対象は表示中 page の in-flight message に限定（`ViewableSessionRegistry` の gating は維持）。
- 一覧 UI（`listSessions` 消費側）は summary のみで構成（変更は最小、API 形は不変）。

> 仮定: frontend 変更は「page API の消費」と「初期描画で全 body を hydrate しない」までに限定する（requirements scope / `behavior.md` 末尾 2 シナリオ）。本格的な仮想化・LRU・閉じた session の body 退避は #1195 が担当する。

---

## アーキテクチャと責務分割

`.claude/rules/rust-first-logic.md` に従い、paging / summary 化 / fork / migration の判断はすべて Rust に置く。frontend は cursor を不透明トークンとして受け渡し、返ってきた page を表示するだけにする。

```
disk layout (per session)            責務
~/.releash/sessions/
  {session_id}/
    meta.json        … 保存正典: session 識別子・状態・復帰識別子・config・summary cache
    index.json       … 順序マップ: message id ↔ seq・role・timestamp・content_hash・attachment refs・token/run meta
    messages/
      {seq}.json     … message body（parts/content 等、binary を除く）
    attachments/
      {attachment_id} … content-addressed blob（画像等の binary）
  {session_id}.json  … 旧 flat フォーマット（migration 前のみ存在）
```

| レイヤー | 責務 |
| --- | --- |
| `domain/agent_session` | session storage の repository / gateway trait と、保存正典に関わる純粋な値の制約 |
| `usecase/agent_session` | list / page / append / save / fork / state/meta update の業務手順、repository / gateway trait 呼び出し、hot path と cold path の選択 |
| `adaptor/gateway/agent_session` | split layout の read/write、メタ cache、diff 保存、hardlink/copy fork、migration、attachment blob I/O |
| `adaptor/controller/command` | Tauri command 公開（`get_session_page` 等）、DTO 整形 |
| `infrastructure/bridge_common` | runtime 状態（streaming overlay / turn_phase / token usage）と保存正典のマージ |
| frontend hooks | cursor 受け渡しと page 表示のみ |

`docs/architecture/README.md` / `USECASE.md` の依存方向に従い、usecase は `std::fs`・hardlink・atomic rename・blob hydrate といった保存実装詳細を直接扱わない。usecase は repository / gateway trait に対して「meta-only 更新」「message append」「page query」「full cold load」などの意図を渡し、split layout のファイル I/O と migration は adaptor/gateway 実装で完結させる。これにより QueryService / Command の分離や別永続化実装への差し替え時に、アプリケーション手順層の公開 API が保存フォーマットへ固定されない。

### 保存正典（canonical storage）の定義

要求の「保存正典の明文化」と #1190 整合のため、正典を次のとおり定義する。

- **session メタ（`meta.json`）= 正典**: `id` / `worktree_path` / `state` / `created_at` / `updated_at` / `agent_session_id` / `context_carry` / `permission_mode` / `plan_mode` / `selected_model` / `permission_profile_id` / `backend_id` / `workflow_step_session`、および summary 用の `first_message_preview` / `message_count`。**#1190 の復帰に必要な `agent_session_id` と `context_carry` はメタに常駐**し、body を読まずに summary だけで取得できる（「見えているのに復帰できない」状態を構造的に作らない）。
- **message body（`messages/{seq}.json`）= 正典**: 各メッセージの `parts` / legacy `content` 等。復帰時 reinject の prompt prefix 再構成はここを cold path で読む。
- **index（`index.json`）= 順序の正典 / body から再生成可能**: 破損時は message chunk から再構築できる派生情報。cursor の安定性（A2）はここの `seq` が担保する。
- **attachment blob = 正典（binary 実体）**: message body は `AttachmentRef` のみを保持し、実体は blob ファイルに置く。
- **runtime 状態は非正典**: `streaming_parts` / `turn_phase` / `pending_queue` / `latest_token_usage` は `AgentProcessMap`（runtime）が所有し、永続化正典に含めない（#977 の persistent/runtime 分離方針と一致）。page / get 応答時に in-flight message へ overlay する。

この定義により behavior.md「保存正典は復帰時 context restoration と矛盾しない」Rule を満たす。

---

## データモデルまたは型

### 新規（Rust, `session/mod.rs`）

```rust
/// meta.json。ChatSession から messages を除いた保存正典 + summary cache。
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub id: String,
    pub worktree_path: String,
    pub state: SessionState,
    pub created_at: f64,
    pub updated_at: f64,
    pub agent_session_id: Option<String>,
    pub context_carry: Option<ContextCarryState>,
    pub permission_mode: String,
    pub plan_mode: bool,
    pub selected_model: Option<String>,
    pub permission_profile_id: Option<String>,
    pub backend_id: Option<String>,
    pub workflow_step_session: bool,
    /// summary index 用。body を読まずに list を構成するためメタへ非正規化保持する。
    pub first_message_preview: String,
    pub message_count: usize,
    /// 保存レイアウトのバージョン。migration 判定に用いる。
    pub body_format_version: u32,
}

/// index.json の 1 エントリ。順序・cursor・page の軽量メタ。
#[serde(rename_all = "camelCase")]
pub struct MessageIndexEntry {
    pub id: String,
    pub seq: u64,              // 挿入順に単調増加。cursor 安定キー（A2）。
    pub role: MessageRole,
    pub timestamp: f64,
    pub content_hash: String, // diff 保存用。chunk 変更検知に使う。
    pub attachment_refs: Vec<AttachmentRef>,
    pub token_meta: Option<MessageTokenMeta>, // 将来の per-message token/run メタ（任意）。
}

#[serde(rename_all = "camelCase")]
pub struct AttachmentRef {
    pub id: String,          // content-addressed（hash）。attachments/{id}。
    pub media_type: String,  // 例 "image/png"
    pub byte_size: u64,
}

/// get_session_page の戻り。
#[serde(rename_all = "camelCase")]
pub struct SessionPage {
    pub messages: Vec<ChatMessage>,   // 指定 page 分のみ（attachment は ref 化済み）
    pub next_cursor: Option<PageCursor>, // さらに過去があるとき Some
    pub has_more: bool,
    pub total_count: usize,           // index 由来。message_count と一致
}

/// 不透明 cursor。返却 page の最古 message の seq を基準にした境界。
/// 「seq < cursor を新しい順に limit 件」で次 page を返す → 同一 cursor+limit は同一集合（A2 / 再取得シナリオ）。
#[serde(transparent)]
pub struct PageCursor(pub u64);
```

`MessageTokenMeta` は現状 per-message のトークン会計を持たないため任意フィールドとして予約する（後述「仮定」参照）。session 単位の `latest_token_usage` は従来どおり runtime overlay から供給する。

### 既存型への影響

- `ChatSession` / `ChatMessage` / `MessagePart` / `SessionSummary` の **serde 表現（camelCase / フィールド構成）は維持**する。`ChatSession` は「分割保存の組み立て結果」かつ「full 読み出し時の表現」として引き続き使う。
- `MessagePart::Image { data, media_type }` は**新規保存時のみ** body から外し、`AttachmentRef` として index/body に格納し blob へ実体を書く。旧データの inline `Image` は migration 時に外出しする（後述）。
- `SessionMeta` ↔ `SessionSummary` は 1:1 派生（`SessionSummary` は API 公開型として維持し、`SessionMeta` から構築）。

### Tauri command / 型（frontend 境界）

```ts
// useSessionStore.ts
interface GetSessionPageResponse {
  messages: ChatMessage[];
  nextCursor: string | null; // PageCursor を文字列化した不透明トークン
  hasMore: boolean;
  totalCount: number;
}
getSessionPage(sessionId, cursor: string | null, limit: number): Promise<GetSessionPageResponse>
getSessionAttachment(sessionId, attachmentId): Promise<{ data: string; mediaType: string }>
```

`get_session` / `init_agent_sessions` の活性 session は、`messages` を **最新 page のみ**に絞って返す（応答 shape は維持しつつ body を減らす）。

---

## 処理フロー

### 起動 / ensure_loaded（メタのみロード）

1. `sessions/` を走査。`{id}/meta.json` があれば新レイアウト、`{id}.json` flat があれば旧レイアウト。
2. 各 session の **メタだけ**を in-memory cache（`HashMap<String, SessionMeta>`）へロード。body / index は読まない。
3. 旧 flat は、メタ相当（summary・復帰識別子）を flat JSON のヘッダ部から取得してメタ cache に載せる。body は遅延 migration まで触らない。
4. 破損 / 不正 permission_mode の隔離（`invalid_sessions`）は現行どおり session 単位で維持。

> これにより「一覧取得は message body を読まない」「一覧取得コストは本文総量に支配されない」（behavior 一覧 Rule）を満たす。

### list_sessions / list_closed_sessions（summary index のみ）

1. メタ cache を worktree + predicate でフィルタ。
2. `SessionMeta → SessionSummary` 変換。`first_message` は `first_message_preview`、件数は `message_count`。
3. `session_titles.json` のカスタムタイトルを従来どおり上書き。
4. `updated_at` 降順ソートして返す。body 読み込みは発生しない。

### get_session_page（最新→過去ページング）

1. メタ cache から session を引く（無ければ `None`）。旧 flat なら遅延 migration を先に実行。
2. `index.json` を読む（軽量）。`cursor == None` なら最新（最大 seq）側から `limit` 件、`Some(c)` なら `seq < c` を新しい順に `limit` 件、対象 `seq` 群を決定。
3. 対象 `messages/{seq}.json` のみを読み、`ChatMessage` を構成（binary は `AttachmentRef` のまま）。
4. さらに過去があれば `next_cursor = Some(最古返却 seq)`、`has_more` を設定。
5. 同一 `cursor + limit` は同一 seq 群 → 同一集合を返す（A2 / 再取得シナリオ）。
6. runtime に in-flight streaming があり、対象 message が page 内なら streaming overlay を適用（`get_session_internal_with_data_dir` と同じ合成）。

> これにより「最新ページ取得」「cursor で過去方向」「先頭到達で has_more=false」「同一 cursor 再取得は同一」（behavior page Rule）を満たす。

### get_session（full / cold path）

- restoration reinject・search 等、全 messages が必要な内部経路向けに `load_full_session(id)`（全 chunk 読み）を提供。frontend 初期描画はここを通らず page 経路を使う。
- frontend 互換のため `get_session` command は残すが、返す `messages` は最新 page に絞る（full body の clone をやめる）。

### save_session（diff 保存・メタ操作は body 非依存）

1. `permission_mode` 検証・正規化（現行どおり）。
2. `meta.json` を常に書く（小さい・atomic write）。
3. messages は **index の `content_hash` と突き合わせ、変更/新規 chunk のみ書く**。状態変更・permission 変更・rename 等の「メタだけ変わる save」は chunk 書き込み 0 件。
4. 新規 `Image` part は blob 外出しして `AttachmentRef` 化し、`attachments/` へ書く。
5. `index.json` を更新（seq 採番・hash・ref・preview/件数のメタ反映）。

> これにより「メタ操作が本文全量を再書き込みしない」を満たす。

### streaming 永続化（persist_streaming_parts）

1. data_dir 解決後、対象 `message_id` の **chunk だけ**を読み（または runtime 保持分から）更新する。
2. 当該 message chunk・index エントリ・`meta.updated_at` のみ書き込む。他 message には触れない。
3. legacy `content`/`thinking`/`activities` は互換出力として当該 chunk 内で生成（`parts_to_legacy`）するが、保存正典は `parts`。

> これにより「streaming 中の保存が全量書き込みにならない」「保存後に summary index と body が整合」（behavior 保存 Rule）を満たす。`streaming_parts` の解放自体は #1194 の担当（本設計はそれと整合する read/write のみ提供）。

### fork_session（hardlink copy-on-write）

1. parent のメタを読み、`id` 新規採番・`state = Idle`・`agent_session_id = None`・`context_carry = None` で fork メタを作成（現行 `fork_session` の属性継承を踏襲）。
2. parent の `index.json` を fork ディレクトリへコピー。
3. parent の `messages/{seq}.json`（および参照される `attachments/`）を fork へ **hardlink** する（本文 byte をコピーしない）。同一ボリューム前提（`~/.releash` 配下）。hardlink 不可の環境ではファイル copy にフォールバック。
4. message chunk は finalize 後 immutable のため、fork 後に fork 側へ追記された message だけが独自 chunk になる（既存 chunk は hardlink 共有）。
5. カスタムタイトル継承は現行どおり。

> hardlink は本文 byte の即時複製を伴わないため「fork が即時全量複製を起こさない」、共有 chunk を page 取得できるため「fork した session を開いて続行できる」（behavior fork Rule）を満たす。

### migration（旧 flat → 新レイアウト, 遅延・非破壊）

1. トリガ: 旧 flat session に対する最初の `get_session_page` / `save_session` / `fork_session` / full 取得。
2. flat `{id}.json` を読み、`SessionMeta` と message 群へ分解。inline `Image` は blob 外出し。
3. `{id}/` 配下に `meta.json` / `index.json` / `messages/` / `attachments/` を atomic に書く（tmp ディレクトリ → rename）。
4. 新レイアウト確定後に旧 flat を削除（書き込み完了まで旧 flat は温存するため、途中失敗でも旧データは破壊されない）。
5. 読み取り専用操作（list 等）では migration せず、flat ヘッダからメタを供給して非破壊に列挙する。

> これにより「旧フォーマットの session を開いてもデータが壊れない」（behavior 互換 Rule / 受け入れ基準）を満たす。

### frontend 初期描画と追加取得

1. session を開く → `getSessionPage(id, null, INITIAL_LIMIT)` で最新 page のみ hydrate（全 body を読まない）。
2. 過去方向スクロールで `getSessionPage(id, nextCursor, LIMIT)` を追加要求し、取得分を先頭へ prepend。
3. streaming は表示中（最新 page 内）の in-flight message にのみ反映（`ViewableSessionRegistry` gating 維持）。

> これにより frontend Rule（初期描画は可視 page のみ・スクロールで過去 page 取得）を満たす。

---

## エラー処理

- **session 単位の隔離を維持**: 1 つの破損 session（壊れた `meta.json` / 不正 permission_mode / id 不一致）で全体ロードを失敗させない。現行 `invalid_sessions` の方針を新レイアウトでも踏襲し、メタ parse 失敗・index 不整合を session 単位で隔離する。
- **エラー文言の汎化**: フルパス・serde 生メッセージ・base64 実体を API へ漏らさない（Spec issues-947 と同じ `invalid_session_error_message*`）。
- **index と body の不整合**: index に在るが chunk 欠落／chunk に在るが index 欠落の場合、chunk 群から index を再生成して自己修復。再生成不能なメッセージはスキップしログ（warn）に残す。
- **migration の途中失敗**: 新レイアウトを tmp → rename で atomic 化。旧 flat は新レイアウト確定後にのみ削除。途中失敗時は旧 flat を残し、次回再試行。
- **fork の部分失敗**: hardlink/コピー途中失敗時は fork ディレクトリを破棄（現行 `remove_session_file_and_cache` 相当をディレクトリ削除へ拡張）。
- **streaming persist の失敗**: 現行どおり warn ログで継続（描画を妨げない）。chunk 単位なので失敗影響は当該 message に限定。
- **attachment blob 欠落**: `get_session_attachment` は blob 不在を明示エラーで返し、page 取得自体は ref を返して成功させる（描画は欠落プレースホルダ）。

---

## テスト方針

Rust は各モジュール `#[cfg(test)]`、frontend はロジック単体（Tauri は `vi.mock`）。CLAUDE.md のテスト規約に従う。

### Rust（usecase / adaptor gateway）

- **メタのみ list**: 本文量だけが異なる 2 状態で `list_sessions` が同一コスト傾向（読み込み chunk 0 件）。chunk read 回数をカウントして検証。
- **page 取得**: 最新 page / cursor 過去 page / 先頭到達（`has_more=false`）/ 同一 cursor 再取得の同一性（A2）。
- **page が他 page の body を読まない**: 指定 seq 群の chunk だけが read される（read フック / カウンタ）。
- **save の diff 性**: 状態のみ変更時に message chunk 書き込みが発生しない。1 message 追加時は 1 chunk のみ書く。
- **streaming persist**: in-flight message chunk のみ更新され、他 chunk の mtime/内容が不変。保存後に summary 件数・preview が body と一致。
- **fork**: hardlink により本文コピーが起きない（inode 共有 or コピー byte 0 を確認）。fork を page 取得して parent 由来 body を参照できる。属性継承（state=Idle・agent_session_id=None 等）は既存 `fork_session_creates_detached_copy` を新レイアウトで維持。
- **migration**: 旧 flat を list / page / full 取得しても messages・メタが破壊されない。inline `Image` が blob 外出しされ ref で取得できる。migration 途中失敗（rename 前 panic 注入）で旧 flat が残る。
- **保存正典 / 復帰整合（#1190）**: メタから `agent_session_id` / `context_carry` が body 無しで取れる。full 取得 → reinject prompt prefix 再構成が従来結果と一致。
- **隔離**: 破損 meta を含む状態で list が無関係な正常 session を返し、個別取得は汎化エラー。
- **既存テスト維持**: session usecase / gateway の現行テスト群（permission 正規化・title・archive・state listener 等）を新レイアウトで通す（`make_session` ヘルパは repository/gateway 経由で保存する形へ更新）。

### frontend（`useSessionStore.ts` / `useAgentChat.ts`）

- `getSessionPage` の cursor 受け渡しと page 連結（prepend）ロジック。
- 初期描画で最新 page のみ invoke される（`get_session` full を呼ばない）。
- streaming 反映が最新 page の in-flight message に限定される。

### 性能（相対基準・#1209 前提）

- 定量閾値は #1209 の telemetry に委ねる。本 Issue は「session 数・本文量を変えて list / page / save の読み書き量が本文総量に支配されない」ことを read/write カウンタで相対確認する（A4）。

---

## リスクと代替案

### リスク

- **多数の小ファイル化**: 長い会話で `messages/{seq}.json` が増え、inode / ディレクトリ走査コストが上がる。緩和: page は index 経由で対象 chunk のみ open（ディレクトリ全走査しない）。閾値超過時に chunk を束ねる pack 化は将来余地（本 Issue 外）。
- **既存 save_session 呼び出し元の広さ**: `save_session` は多数の経路から full `ChatSession` で呼ばれる。diff 保存で互換を保つが、呼び出し元の期待（全 message が常に渡る）と分割保存の差異に注意。`load_full_session` で読んで save する経路は body コストが残るため、ホットでない cold path に限定する。
- **migration の一過性負荷**: 初回アクセス時に分解コストが出る。緩和: 遅延・session 単位・1 回限り。
- **hardlink のボリューム/権限差**: 環境により hardlink 不可。フォールバック（copy）で機能維持、ただし fork コストは増える。
- **runtime overlay との二重表現**: in-flight message は body（永続）と streaming（runtime）で表現が割れる。get/page 双方で同一 overlay ロジックを共有して齟齬を防ぐ。

### 代替案（不採用理由つき）

- **単一 JSON + summary sidecar のみ**: list は救えるが、get/page が依然 full JSON を parse し body 量に比例。get/save/fork の body 非依存化（Background の主訴）を満たせず不採用。
- **append-only NDJSON ログ + offset index**: streaming で同一 message を追記し続けるとログが肥大し compaction 必須。backward paging に offset 管理が要り、chunk 方式と比べ複雑。監査の代替案として記載されるが、本設計では「immutable chunk + 可変 in-flight chunk」の方が streaming 書き込み境界が明確なため不採用。
- **fork: parent 参照（COW, hardlink なし）**: chunk byte も link も作らず最小コストだが、parent 削除時の lifetime 管理が必要で「fork は完全独立」という現行不変条件を崩す。hardlink は独立性を保ったまま byte 複製を避けられるため優先。parent 参照は hardlink 不可環境の将来選択肢として残す。
- **per-message token 会計の新規導入**: page の「token/run metadata」を per-message で持つには新たな会計が要る。現状 session 単位 `latest_token_usage`（runtime）で要求を満たせるため、per-message は任意予約に留める（スコープ拡大回避）。

---

## 仮定

本文中の仮定を集約する（`behavior.md` の A1〜A5 を前提とする）。

1. **保存先**: `~/.releash/sessions/` 配下を維持。session ごとに `{session_id}/` ディレクトリを切る（A5）。
2. **保存フォーマット**: per-message chunk file（`messages/{seq}.json`）+ メタ（`meta.json`）+ 順序 index（`index.json`）+ attachment blob を採用する。
3. **cursor**: 挿入順 `seq`（単調増加）を基準にした不透明トークン。最新側から過去方向へページング（A2）。同一 cursor+limit は同一集合。
4. **fork**: immutable な message chunk / attachment blob を hardlink で共有する copy-on-write。本文 byte の即時複製はしない。hardlink 不可環境は copy フォールバック。
5. **migration**: 旧 flat フォーマットは遅延 migration（初回の page/save/full 取得時に新レイアウトへ変換）。読み取り専用操作は flat 互換読みで非破壊に列挙。新レイアウト確定後に旧 flat を削除し、途中失敗では旧 flat を温存する。
6. **attachment 外出し**: 新規保存・migration 時に `MessagePart::Image` の binary を content-addressed blob へ外出しし、message body / page には `AttachmentRef` のみを載せる。binary は `get_session_attachment` で別途取得（A3）。
7. **token/run metadata**: session 単位 `latest_token_usage` は runtime overlay から供給する従来方針を維持。per-message の token/run メタは index に任意フィールドとして予約し、本 Issue では新規会計を導入しない。
8. **runtime 状態の非永続**: `streaming_parts` / `turn_phase` / `pending_queue` / `latest_token_usage` は runtime 所有で正典外。get / page 応答時に in-flight message へ overlay する。
9. **frontend スコープ**: page 消費と「初期描画で全 body を hydrate しない」までを本 Issue で満たす。仮想化・LRU・閉じた session の body 退避は #1195。
10. **full 取得の残存**: restoration reinject・search 等の全 message を要する cold path には `load_full_session` を残す。frontend ホット経路は page を使う。
11. **性能基準**: 定量閾値は #1209 telemetry に委ね、本 Issue は相対基準（本文総量にスケールしない）で検証する（A4）。

---

## Open Questions

なし。
