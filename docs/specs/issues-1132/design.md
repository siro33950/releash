# Design

対象 Issue: #1132（[impl] review_comments / comment migration）
マイルストーン: [12] クリーンアーキテクチャ移行

本 design は `requirements.md` / `behavior.md` を入力とし、`src-tauri/src/review_comments/` を
clean architecture の層（domain / usecase / adaptor / infrastructure）へ再配置する実装方針を定める。
本移行は配置移動であり、外部から観測可能な振る舞い（CLI 出力契約 / Tauri command の I/O / 永続化フォーマット）を
変更しない（requirements R9, behavior 全 Scenario の非退行）。

---

## 概要

現状、review comment のロジックは layer 区分を持たない `review_comments/` 配下（`mod.rs` 約 2,656 行ほか）に
entity / value object / event / error / 純粋ロジック（validation・projection・filter）・file-backed event store
（lock・JSON serialization・atomic replace）・Tauri command・handoff・watcher が同居している。

これを次の層へ再配置する。

- `domain/comment/` — entity / value object / event / error / 純粋ロジック（infrastructure 非依存）。
- `usecase/comment/` — list / get / create / append / resolve / delete / history / handoff の application flow と、
  storage / clock / id 生成の **port 定義**。
- `adaptor/gateway/comment/` — file-backed event store（storage port の実装）。
- `adaptor/controller/command/comment/` — 8 つの Tauri command wrapper（request/response mapping のみ）。
- `infrastructure/comment/` — review state file の watcher（notify ベース）。

移行後、`review_comments/` を削除し、`lib.rs` の `mod review_comments` と
`adaptor/controller/command/mod.rs` の `crate::review_comments::*` 直接登録を除去する。
CLI（`releash review ...`）は `crate::review_comments` への直接依存をやめ、新 usecase 境界経由で動作させる。

### 設計上の基本判断

- **型はリネームしない**（requirements A2）。`ReviewThread` 等の public 型・Tauri command 名・CLI 出力・
  永続化フォーマットは現状維持。module path のみ変わる。
- **module 名は `comment`** とする（requirements スコープが `domain/comment/` 等を明示）。既存の無関係な
  `usecase/review_usecase.rs`（code review snapshot を扱う別ドメイン）との名前衝突を避ける意味でも `comment` を採用する。
- **read-modify-write の原子性を port 境界で保つ**（後述「アーキテクチャと責務分割」）。現実装は per-worktree lock 下で
  load → validate → append → atomic replace を 1 critical section で実行している。これは behavior の永続化 Rule
  （worktree 単位で直列化・原子的置換・event を失わない）を満たす契約であり、層分割でこの原子性を壊さない。

---

## 変更対象

### 新規作成

```text
domain/comment/
├── mod.rs                 # re-export と module 宣言
├── entity.rs              # ReviewThread / ReviewComment / ReviewResolveInfo / ReviewThreadState
├── actor.rs               # ReviewActor / ReviewActorDto / ReviewActorKind / participant_key
├── target.rs              # ReviewTarget
├── filter.rs              # ReviewThreadFilter / AuthorScope / apply_filter
├── event.rs               # ReviewEvent / ReviewHistoryEntry
├── error.rs               # ReviewError / ReviewErrorCode / ReviewErrorDto
├── validation.rs          # validate_content / validate_target / validate_filter /
│                          #   validate_review_file_path / validate_line_range /
│                          #   ensure_thread_open / ensure_can_delete / is_unread_for_viewer
└── projection.rs          # ThreadAccumulator / project_thread(s) / project_threads_from_iter

usecase/comment/
├── mod.rs                 # module 宣言と公開 API
├── ports.rs               # ReviewEventStore（storage port）/ ReviewClock / ReviewIdGenerator
├── service.rs             # list/get/create/append/resolve/delete/history の application flow
└── handoff.rs             # build_review_thread_handoff_message（純粋）と handoff usecase

adaptor/gateway/comment/
├── mod.rs
└── event_store.rs         # FileReviewEventStore（ReviewEventStore 実装）: lock / serialize / atomic replace /
                           #   破損・欠損ファイル処理 / worktree storage key 導出

adaptor/controller/command/comment/
├── mod.rs                 # register / invoke_handler（COMMAND_NAMES）
└── commands.rs            # 8 command wrapper

infrastructure/comment/
├── mod.rs
└── watcher.rs             # spawn_review_comments_watcher
```

> サブモジュールのファイル分割粒度は上記を基準とする（requirements A3）。1 ファイルに収まる場合は統合してよいが、
> domain は「infrastructure 非依存の塊」として独立させることを優先する。

### 更新

- `cli/mod.rs` — `use crate::review_comments::{...}` を新 path（`domain/comment` の型、`usecase/comment` の flow）へ。
  `review_error_to_cli_error` の参照 path 更新。`cmd_review` 内の `ReviewCommentStore::default()` を
  新 gateway/usecase 構成へ差し替え。変更は review 接続に必要な最小限（requirements A6 / #1134 へ踏み込まない）。
- `adaptor/controller/command/mod.rs` — `crate::review_comments::*` 8 行を
  `crate::adaptor::controller::command::comment::commands::*` 登録へ置換（または既存の `register()` 方式に合わせる）。
- `lib.rs` — `mod review_comments;` 除去。`mod` 宣言（`domain`/`usecase`/`adaptor`/`infrastructure` 配下は既存宣言を利用）。
  `.manage(Arc::new(ReviewCommentStore::default()))` を新 store/gateway の manage へ差し替え。
  `spawn_review_comments_watcher(...)` の呼び出し path を `infrastructure::comment::watcher::...` へ。
- 各層の `mod.rs`（`domain/mod.rs`・`usecase/mod.rs`・`adaptor/gateway/mod.rs`・
  `adaptor/controller/command/mod.rs`・`infrastructure/mod.rs`）に `comment` module 宣言を追加。

### 削除

- `review_comments/`（`mod.rs` / `commands.rs` / `handoff.rs` / `watcher.rs`）全体。

---

## アーキテクチャと責務分割

層と依存方向は agent_session 移行の確立パターン（`domain/agent_session/storage.rs` に port、
`usecase/agent_session/.../ports.rs` に usecase port、gateway/infrastructure に実装）に倣う。

依存方向: `controller → usecase → domain`、`gateway → usecase(port)・domain`、`infrastructure → (Tauri runtime)`。
domain は他層へ依存しない。

### domain/comment（純粋ロジック）

- entity / value object / event / error / DTO を保持。
- domain Entity 側の既存 public 型 `ReviewActorDto` は、A2（型不変）を優先して本 Issue では据え置く。
  本来は転送都合の DTO を domain に置かない方針と名前がずれるが、既存 surface の互換性維持を優先する例外として扱う。
- 純粋関数: `validate_*`・`ensure_thread_open`・`ensure_can_delete`・`is_unread_for_viewer`・`apply_filter`・
  `project_thread(s)`・`project_threads_from_iter`・`ThreadAccumulator`。
- **副作用を持たない**。`now()` / `event_id()` / filesystem / Tauri / notify への依存を持たない（R1）。
  現実装が `mod.rs` に同居させている `now()` / `event_id()` / `worktree_storage_key` / `state_file` /
  `lock_file` / `replace_file` は domain から除外し、clock/id は usecase port、storage key 導出・atomic replace は
  gateway へ移す。

### usecase/comment（application flow + port 定義）

application flow（service.rs）は次を担う。

1. read 系（list / get / history）: port で event 列を load し、domain の projection / filter を適用して返す。
2. write 系（create / append / resolve / delete）: port の **transactional mutate** で
   「load → domain validation → 新 event 生成（clock/id port 使用）→ append」を 1 critical section 内で実行する。
3. handoff: thread を get し、`build_review_thread_handoff_message(alias, &thread)` で指示文を生成（CLI 名解決は呼び出し側 alias 引数）。

port（ports.rs）は次を定義する（R2: storage / time / id を port 経由）。

- `ReviewEventStore`（storage port）— **原子性を保つため transaction クロージャ方式を採用する**。

  ```rust
  pub trait ReviewEventStore: Send + Sync {
      // read: lock 不要。欠損 worktree は空 Vec、破損は Err（黙って空にしない）。
      fn load(&self, app_data_dir: &Path, worktree_name: &str) -> Result<Vec<ReviewEvent>, ReviewError>;

      // write: worktree 単位 lock 下で load → mutate(現 events)→ 返った追記 event を atomic replace で永続化。
      // mutate が Err を返したら永続化せず Err を伝播（validation 失敗時に副作用を起こさない）。
      fn mutate<F>(&self, app_data_dir: &Path, worktree_name: &str, mutate: F)
          -> Result<Vec<ReviewEvent>, ReviewError>
      where
          F: FnOnce(&[ReviewEvent]) -> Result<Vec<ReviewEvent>, ReviewError>;
  }
  ```

  - `mutate` のクロージャ内に usecase が validation と新 event 生成を渡す。これにより
    behavior「load→validate→append が worktree 単位で直列化され event を失わない / validation 失敗で副作用を起こさない」を
    layer 分割後も保つ。`FnOnce` が trait object 化を妨げる場合は、ジェネリックメソッドを持つ trait のまま使うか、
    `Box<dyn FnOnce...>` を取る非ジェネリックシグネチャに調整する（実装時に確定。代替は「リスクと代替案」参照）。
- `ReviewClock` — `fn now(&self) -> f64`（UNIX 秒）。
- `ReviewIdGenerator` — `fn event_id(&self) -> String`（UUID v4）。

clock / id を port 化することで、domain と usecase が時刻・ID 生成の実体に依存せず、テストで決定論的な値を注入できる。
本番実装（`SystemReviewClock` / `UuidReviewIdGenerator`）は gateway もしくは infrastructure に置く（小さいので gateway 配下で可）。

### adaptor/gateway/comment（storage port 実装）

- `FileReviewEventStore` が `ReviewEventStore` を実装。現 `ReviewCommentStore` / `ReviewPersistenceGateway` の
  永続化責務を引き継ぐ:
  - in-process `Mutex`（file_lock）+ OS file lock（`.events.lock`, fs2 exclusive, 10s リトライ）による worktree 単位排他。
  - worktree storage key 導出（SHA256 + label）、`review-comments/{key}.events.json` への配置（R3 / behavior 永続化 Rule）。
  - temp file へ pretty JSON 書き出し → `fsync` → atomic rename（`replace_file`）。
  - read は lock-free。欠損 file は空 Vec、JSON parse 失敗は `ReviewError::Serialize`（上書きしない）。
- `mutate` 実装は: lock 取得 → `load` → `mutate(&events)` 呼び出し → 返った event 列を既存に連結して書き出し。
- `SystemReviewClock` / `UuidReviewIdGenerator` の本番実装もここ（または近接 module）に置く。

### adaptor/controller/command/comment（Tauri command）

- 8 command（`list_review_threads` / `get_review_thread` / `create_review_thread` / `append_review_comment` /
  `resolve_review_thread` / `delete_review_thread` / `get_review_thread_history` / `build_review_thread_handoff`）。
- 各 command は `tauri::State` から store/usecase 依存を受け取り、request を usecase 呼び出しへ変換、
  結果を現行と同じ型（`ReviewThread` / `Vec<ReviewHistoryEntry>` / `String` / `()`）で返す。business behavior は持たない（R4）。
- I/O は現行どおり `tokio::task::spawn_blocking` で逃がす。
- mutation 後の `emit_changed`（"review-comments-changed" 相当のフロント通知）は **delivery 副作用**として controller に残す
  （AppHandle を持つのは controller。usecase に notification port を新設しない。現行挙動を変えない）。
- 登録は notification module 同様 `mod.rs` の `COMMAND_NAMES` + `invoke_handler()` 方式に合わせる。

### infrastructure/comment（watcher）

- `spawn_review_comments_watcher(app, app_data_dir)` を移設。notify_debouncer_mini による
  `review-comments/*.events.json` 監視と "review-comments-changed" emit、fallback polling を維持。
- gateway（storage port 実装）に自然に収まらない file watching 副作用のため infrastructure に置く（R5）。
- Tauri AppHandle への emit を伴うため infrastructure を配置先とする（runtime 連携の副作用）。

---

## データモデルまたは型

型定義は現行を**そのまま**移送する（リネーム・フィールド変更なし。R9 / A2）。配置先のみ変える。

| 型 / 関数 | 現在地 | 移送先 |
| --- | --- | --- |
| `ReviewThread` / `ReviewComment` / `ReviewResolveInfo` / `ReviewThreadState` | `review_comments/mod.rs` | `domain/comment/entity.rs` |
| `ReviewActor` / `ReviewActorDto` / `ReviewActorKind` | `mod.rs` | `domain/comment/actor.rs` |
| `ReviewTarget` | `mod.rs` | `domain/comment/target.rs` |
| `ReviewThreadFilter` / `AuthorScope` / `apply_filter` | `mod.rs` | `domain/comment/filter.rs` |
| `ReviewEvent` / `ReviewHistoryEntry` | `mod.rs` | `domain/comment/event.rs` |
| `ReviewError` / `ReviewErrorCode` / `ReviewErrorDto` | `mod.rs` | `domain/comment/error.rs` |
| `validate_*` / `ensure_*` / `is_unread_for_viewer` | `mod.rs` | `domain/comment/validation.rs` |
| `ThreadAccumulator` / `project_*` | `mod.rs` | `domain/comment/projection.rs` |
| `now()` / `event_id()` | `mod.rs` | usecase port（`ReviewClock` / `ReviewIdGenerator`）+ gateway 実装 |
| `worktree_storage_key` / `state_dir` / `state_file` / `lock_file` / `replace_file` / write/load | `mod.rs` | `adaptor/gateway/comment/event_store.rs` |
| `ReviewCommentStore` / `ReviewPersistenceGateway` の public API | `mod.rs` | usecase service + gateway `FileReviewEventStore` に分割 |
| 8 Tauri command | `commands.rs` | `adaptor/controller/command/comment/commands.rs` |
| `build_review_thread_handoff_message` | `handoff.rs` | `usecase/comment/handoff.rs`（純粋関数として） |
| watcher | `watcher.rs` | `infrastructure/comment/watcher.rs` |

- serde の `#[serde(rename_all = ...)]` 等の属性は **event JSON / DTO のフィールド名を維持するため変更しない**
  （behavior「event JSON の構造が保たれる」/「actor に既存スキーマの sessionId を記録する」契約）。
- 永続化フォーマット: `eventType` / `eventId` / `threadId` / `actor`（participant 識別、optional `sessionId`）/ `at` を含む既存スキーマ、
  state file 名（worktree 解決パスから決定論的に導出）を維持。

---

## 処理フロー

### create_thread（write 系の代表）

1. controller: request（worktree_name, file_path?, line/end?, content, actor）を受け取り usecase へ。
2. usecase `create`:
   - `store.mutate(app_data_dir, worktree_name, |events| { ... })` を呼ぶ。
   - クロージャ内: `validate_content` / `validate_target`（file path・line range）→ NG なら `Err`（永続化されない）。
   - `clock.now()` / `id.event_id()` で `ReviewEvent::ThreadCreated`（初回 comment 含む）を生成し、追記 event 列を返す。
   - gateway は lock 下で現 events に連結し atomic replace。
3. usecase: 永続化後の全 event を `project_thread` で射影し `ReviewThread` を返す。
4. controller: 結果を返し、`emit_changed` でフロント通知。

### append / resolve / delete

- 同じ `mutate` 経路。append は `ensure_thread_open` + NotFound 判定、resolve は二重 resolve→`AlreadyResolved`、
  delete は `ensure_can_delete`（human のみ、agent は `PermissionDenied`）・既削除→`NotFound`。
  判定はすべて domain 関数で行い、event を生成して返す。

### list / get / history（read 系）

1. usecase: `store.load(app_data_dir, worktree_name)` で event 列取得（欠損は空）。
2. domain projection:
   - list: `project_threads` → `apply_filter(threads, filter, viewer)`（file/state/author/unread/thread_id を AND、thread_id は OR）。
   - get: `project_thread(thread_id)` → 削除済み/不在は `NotFound`。
   - history: 対象 thread の event を追記順で `ReviewHistoryEntry` へ変換。一度も作成されていなければ `NotFound`。

### handoff

1. usecase: get で thread を取得（不在は `NotFound`）。
2. `build_review_thread_handoff_message(alias, &thread)` で指示文生成。alias は呼び出し側が
   `path_aliases::alias_name_for_profile(BuildProfile::current())` で解決（releash / releash-dev）。CLI 名はハードコードしない。

### CLI（`releash review ...`）

- `cli/mod.rs` は `domain/comment` の型と `usecase/comment` の flow を使う。
- `cmd_review` は `FileReviewEventStore` + 本番 clock/id を構築し usecase を呼ぶ（`ReviewCommentStore::default()` 相当の置換）。
- `review_actor(...)` / `review_worktree_from_session(...)` の actor/worktree 解決ロジックは現状維持。
- 出力整形（`print_review_thread`、`--json`、"(no review threads)" / "(no review history)"、桁揃え）と
  終了コード（成功 0 / InvalidInput 2 / NotFound 4 / その他 1）を維持（behavior CLI Rule）。

---

## エラー処理

- domain `ReviewError`（`InvalidInput` / `NotFound` / `AlreadyResolved` / `PermissionDenied` / `Io` / `Serialize`）を
  単一の error 表現として維持（型・variant を変えない。R9）。
- usecase は domain error をそのまま伝播。gateway は I/O / serialize 失敗を `ReviewError::Io` / `ReviewError::Serialize` に map。
- controller（Tauri）: 現行どおり `ReviewError` を `String`（または `ReviewErrorDto` 経由）へ map して返す。形状を変えない。
- CLI: `review_error_to_cli_error` を維持（`AlreadyResolved` / `PermissionDenied` → `InvalidInput`(2)、`NotFound`→4、
  `Io`/`Serialize`→Other(1)）。参照 path のみ更新。
- 破損 state file: `load` は `ReviewError::Serialize` を返し、黙って空一覧にフォールバックしない（behavior 契約 / A4）。
- validation 失敗は `mutate` クロージャ内で `Err` を返し、永続化・通知の前に拒否する（behavior「副作用前に拒否」）。

---

## テスト方針

既存テスト（`review_comments` module 内 34 件の `#[test]` + handoff 2 件 + watcher 2 件）を
**対応する層へ移設し、R9 の非退行回帰として用いる**（A5）。期待値は実装に合わせて変えない（仕様が正）。

- **domain test**（`domain/comment/` 各 module 内 `#[cfg(test)]`）:
  - validation（content の空/空白/NUL/65536 byte 超、file path の絶対/`..`/`\`/NUL/4096 byte 超、line range の 0/逆転/end のみ）。
  - filter（file/state/author/unread/thread_id・AND/OR 結合）、`is_unread_for_viewer`（他者後続投稿 / 自分最終 / resolve は対象外）。
  - projection（updated_at 降順・同値時の作成順保持・削除除外・iterator 一度消費）、`public_projection_redacts_session_id`。
  - port を持たない純粋 test なので fixture の event 列を直接構築して検証。
- **usecase test**（`usecase/comment/`）:
  - list / get / create / append / resolve / delete / history / handoff の正常系・エラー系
    （NotFound / AlreadyResolved / PermissionDenied / InvalidInput）。
  - in-memory な fake `ReviewEventStore`（`Vec<ReviewEvent>` を保持）+ 固定 `ReviewClock` / `ReviewIdGenerator` を注入し決定論化。
  - handoff の alias 反映（releash / releash-dev）。
- **gateway test**（`adaptor/gateway/comment/`）:
  - 永続化（multi-store write が全 comment / resolve once を保持）、lock（既存 lock 再利用・read が write guard/file lock を待たない）、
    破損ファイル（`load_propagates_parse_error_without_overwriting_file`）、欠損ファイル（空一覧）、
    worktree scope 独立・同名 basename の storage key 分離、torn JSON を読まない並行 stress test。
- **CLI test**: `cmd_review` の create/list/get/json/comment/resolve/history/各拒否（既存 `cli/mod.rs` test）を path 更新で維持。
- **watcher test**: `.events.json` 書き込みで emit / `.lock` で emit しない（infrastructure へ移設）。
- 受け入れ: `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` green（R10）。R1〜R8 の構造要求は
  ディレクトリ構成と `review_comments` 不在の確認で担保（behavior A5）。

---

## リスクと代替案

- **storage port の transaction 形状**（採用: `mutate` クロージャ）。
  - リスク: `FnOnce` ジェネリックメソッドは trait object（`Arc<dyn ReviewEventStore>`）化を妨げる。
  - 対策/代替: (a) `mutate` を `Box<dyn FnOnce(&[ReviewEvent]) -> Result<Vec<ReviewEvent>, ReviewError> + '_>`
    を取る非ジェネリックシグネチャにする、(b) trait をジェネリックのまま usecase を具体型に対してジェネリック化する、
    (c) port を `load` + `append_under_lock` の 2 メソッドに割り、lock を gateway が握ったまま callback を呼ぶ
    専用メソッドを別途用意する。**(a) を第一候補**とし、`dyn` 越しに DI できる形を保つ。
  - 不採用案: `load` と `append` を別 lock で 2 回呼ぶ素朴分割は、lock がまたがらず behavior の原子性・直列化契約を破るため不可。
- **clock/id port 化のスコープ増**。port を 2 つ増やすが、決定論テストと domain の副作用排除に必要（R2）。
  小さい trait なので過剰設計にはならない。
- **emit_changed の置き場所**。usecase に notification port を新設せず controller に残す判断。
  将来 WebSocket / daemon から同 usecase を使う際に通知が controller 依存になる懸念はあるが、本 Issue では
  挙動不変を優先（R9）。notification port 化は別 Issue 候補として「現タスク外の改善」に留める。
- **CLI への波及**。`cli/mod.rs` は import path と store 構築のみ更新し、全体分割（#1134）には踏み込まない（A6）。
- **大規模移動による diff**。型・テストを機械的に移すため挿入位置ミスのリスク。各層移設ごとに `cargo test` を回し、
  既存テストの green を移設の正しさの indicator とする。

---

## 仮定

- A1: module 名は `comment`（requirements スコープ準拠）。型名・command 名・CLI 出力・永続化フォーマットは不変（A2）。
- A2: storage port は原子性維持のため `mutate` クロージャ方式を採用する（「リスクと代替案」の代替 (a)〜(c) を実装時の逃げ道とする）。
- A3: time / id 生成は `ReviewClock` / `ReviewIdGenerator` の 2 port に分け、本番実装を gateway 近接に置く。
- A4: watcher は infrastructure（`infrastructure/comment/`）。Tauri AppHandle emit を含む file watching 副作用のため。
- A5: mutation 後のフロント通知 `emit_changed` は controller（delivery）に残し、usecase に notification port を新設しない。
- A6: 本番 clock/id 実装の配置（gateway 配下か infrastructure か）は実装時に最終決定するが、port 契約は usecase が所有する。
- A7: 既存 34 + 2 + 2 件のテストは対応層へ移設し、期待値を変えずに非退行検証へ用いる（requirements A5）。

## Open Questions

なし。
