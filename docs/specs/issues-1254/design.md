# Design

関連: #1254 / requirements.md / behavior.md

本書は requirements.md のスコープ 4 項目（①〜④）に対する実装方針・責務分割・データ構造・処理フロー・エラー処理・テスト方針を定義する。外部から観測可能な review CLI の振る舞いは不変（behavior.md の「不変として維持される観測可能な振る舞い」）とし、本書の変更はすべて内部処理の効率化に閉じる。

## 概要

`releash review`（`list` / `get` / `create` / `comment` / `resolve` / `history`）の起動コストを、セッション総数・セッション本文量・thread 件数から切り離す。具体的には次の 4 つを行う。

- **①** review CLI のセッション解決（actor / worktree 解決）を、`sessions/` 全走査（`ensure_loaded()`）を経由しない「指定 1 セッション直接読み出し」経路に置き換える。
- **②** 上記直接読み出しを dir 形式 meta（`<sessions>/<id>/meta.json`）のみに一本化し、legacy flat / sidecar は解決対象外にする。
- **③** 読み取り専用 review コマンド（`list` / `get` / `history`）の OS 排他ロック取得を廃し、atomic rename による整合性保証に基づくロックレス読み取りにする。
- **④** `project_threads` を events 1 パスの projection に置き換え、`O(threads × events)` を解消する。

## 変更対象

| 区分 | ファイル | 変更内容 |
|---|---|---|
| ① ② | `src-tauri/src/usecase/agent_session/session/mod.rs` | review 解決用の狭い read model `SessionReviewContext` を追加 |
| ① ② | `src-tauri/src/adaptor/gateway/agent_session/session_storage/meta_repository.rs` | 単一セッション直接読み出し `get_session_review_context()` を追加 |
| ① ② | `src-tauri/src/adaptor/gateway/agent_session/session_storage/legacy.rs` | legacy 解析の全撤去によりファイル名と実態（dir 形式 meta reader）が乖離したため、`read_meta_from_dir()` を `meta_repository.rs` へ移設し `legacy.rs` を削除 |
| ① | `src-tauri/src/usecase/agent_session/session/store.rs` | review 解決用の狭い port `SessionReviewContextReader` と `SessionStore::get_session_review_context()` を追加 |
| ① | `src-tauri/src/cli/mod.rs` | `review_actor_and_worktree()` / `review_worktree_from_session()` が `get_session_review_context()` を呼ぶ |
| ③ | `src-tauri/src/review_comments/mod.rs` | `list_threads` / `get_thread` / `history` からロック取得を除去。`file_lock` は `Mutex<()>` のまま維持し書き込みのみ lock guard |
| ④ | `src-tauri/src/review_comments/mod.rs` | `project_threads` を 1 パス projection へ。per-thread 蓄積ロジックを `ThreadAccumulator` に抽出し `project_thread` と共有 |

非変更（requirements 非スコープ）: `events.json` スキーマ、CLI サブコマンド体系・引数・出力、session storage の保存正典・ディレクトリ構造、watcher のポーリング/debounce、Desktop 側の全件ロード経路（`list_metas` / `ensure_loaded`）、書き込み系コマンドのロック戦略。

## アーキテクチャと責務分割

レイヤー方針（`src-tauri/AGENTS.md`）に従い、最適化はすべて gateway / usecase / controller 内に閉じる。domain の `AgentSessionReader` は agent_session の汎用読み取り契約のまま維持し、review CLI の actor/worktree lookup 用 read model は usecase 側の `SessionReviewContextReader` port と concrete gateway API に閉じる。これにより preview/count を含む汎用 `SessionMeta` API と review 専用の軽量 read model を分離し、無関係な `AgentSessionReader` 実装にも review-specific method を要求しない。

### ① セッション解決の軽量経路

現状の経路:

```
cli review_*  →  build_session_store()  →  SessionStore::get_session_meta()
              →  FileSessionStorage::get_session_meta()  →  ensure_loaded()  ← sessions/ 全走査
```

`ensure_loaded()`（`meta_repository.rs:85-154`）は `sessions/` 配下の全ディレクトリを走査してキャッシュを構築するため、必要なのは 1 セッションでもセッション総数に比例した I/O + パースが走る。

新経路を追加する（既存経路は Desktop 用にそのまま残す）:

```
cli review_*  →  SessionStore::get_session_review_context()
              →  FileSessionStorage::get_session_review_context()  ← 指定 1 セッションのみ open
```

`get_session_review_context()` は **キャッシュも `ensure_loaded()` も使わず**、指定 `session_id` のファイルだけを直接 open する。1 コマンド = 1 プロセスでどのみちキャッシュはコールドなため、CLI 解決にキャッシュ構築は不要。

解決の優先順位:

1. `<sessions>/<id>/meta.json` が存在 → `read_meta_from_dir()`（既存。dir 形式 meta は本文と分離済みなので元から body-independent）。
2. それ以外 → `Ok(None)`（呼び出し側で NotFound に変換）。

パース失敗（壊れた JSON 等）は `Err(invalid_session_error_message_with_id(id))` を返し、既存の汎化エラー文言ポリシー（フルパス・serde 生メッセージを露出しない）を踏襲する。`invalid_sessions` キャッシュへの記録は直接経路では行わない（キャッシュを一切触らない方針のため）。

#### usecase port の追加

`usecase/agent_session/session/store.rs` に `SessionReviewContextReader` port を追加する。戻り型を `SessionMeta` と分けることで、`SessionMeta::from_session()` が保証する `first_message_preview` / `message_count` の正典性と、review 解決専用の軽量読み取りを型で分離する。

```rust
fn get_session_review_context(
    &self,
    app_data_dir: &Path,
    session_id: &str,
) -> Result<Option<SessionReviewContext>, String>;
```

`FileSessionStorage` は `SessionReviewContextReader` を実装し、`SessionStore`（usecase facade）はこの port へ forward する。`AgentSessionReader` は変更しないため、prompt suggestion や codex restore plan など review CLI と無関係な reader double は review-specific method を実装しない。

### ② dir 形式 meta のみを対象にする

review CLI の actor / worktree 解決が使うフィールドは `id` / `worktree_path` / `state` / `backend_id` / `selected_model` のみであり、これらは dir 形式の `<sessions>/<id>/meta.json` に本文と分離して保存されている。したがって直接経路では指定された 1 件の dir meta だけを読み、`SessionReviewContext` に変換する。

legacy flat (`<id>.json`) と legacy sidecar (`<id>.meta.json`) は #1254 の解決対象から外す。これらしか存在しない `session_id` は、存在しないセッションと同じく `Ok(None)` を返し、呼び出し側で `NotFound` に変換する。これにより review CLI / list / restore は dir 形式 meta のみを正規入力として扱い、legacy flat 本文や sidecar を読む経路を持たない。

### ③ 読み取り専用コマンドのロックレス化

現状、`list_threads` / `get_thread` / `history`（`mod.rs:941-985`）は読み取りだけなのに

1. in-process `gateway.file_lock`（`Mutex<()>`）を取得し、
2. `acquire_worktree_file_lock()`（514-542）で OS 排他ロック（`fs2::try_lock_exclusive`、失敗時 10ms × 最大 10 秒リトライ）

を取得する。書き込み（`write_events`、`mod.rs:857-875`）は tmp ファイルへ書いて `sync_all` 後に **atomic rename**（POSIX: `std::fs::rename` / Windows: `ReplaceFileW`・`MoveFileExW` with WRITE_THROUGH）する。

**整合性の根拠**: 読み取りは `load()`（`mod.rs:842-855`）で `std::fs::read_to_string` により live ファイル全体を一括読みする。atomic rename により live ファイルは常に「書き込み前の完全な内容」か「書き込み後の完全な内容」のいずれかであり、中途半端な状態は観測されない。したがって読み取り整合性のために OS ロックも in-process ロックも不要である。

**方針**: 読み取り専用 3 メソッドからロック取得を除去する。

- OS ロック（`acquire_worktree_file_lock`）の取得をやめる → 読み取りが書き込み・他読み取り・watcher のいずれにもブロックされない。
- in-process `file_lock` は `Mutex<()>` のまま維持する。
  - 読み取り 3 メソッド: 取得しない。プロセス内で複数読み取りが直列化しないようにする。
  - 書き込み 4 メソッド（`create_thread` / `append_comment` / `resolve_thread` / `delete_thread`）: lock guard + 従来どおり OS 排他ロックを維持（requirements 非スコープ: 書き込みは排他を維持）。

watcher（`watcher.rs`）はそもそもロックを取得せず `read_dir` + メタデータ stat でシグネチャを取るだけなので、本変更により読み取り CLI と watcher は相互に独立する。

### ④ events 1 パス projection

現状の `project_threads`（`mod.rs:781-794`）は ① 全 `ThreadCreated` の id を集め、② id ごとに `project_thread`（681-779）を呼んで **events 全体を再走査** するため `O(threads × events)`。

per-thread の蓄積ロジック（`ThreadCreated` / `CommentAppended` / `ThreadResolved` / `ThreadDeleted` の適用、version カウント、created_at / updated_at 更新）を `ThreadAccumulator` 構造体に抽出し、events を 1 回走査しながら `thread_id` ごとの accumulator を更新する。

```rust
struct ThreadAccumulator {
    author: Option<ReviewActor>,
    target: Option<ReviewTarget>,
    comments: Vec<ReviewComment>,
    resolve: Option<ReviewResolveInfo>,
    created_at: f64,
    updated_at: f64,
    version: u64,
    deleted: bool,
}
impl ThreadAccumulator {
    fn apply(&mut self, event: &ReviewEvent);            // 1 event を反映
    fn finish(self, worktree_name, thread_id) -> Option<ReviewThread>; // deleted/author/target 欠落で None
}
```

`project_thread`（単一 thread、`get_thread` が使用）も `ThreadAccumulator` を `events.iter().filter(thread_id)` で駆動する形に書き換え、ロジックを単一化して挙動同一性を保証する。

`project_threads`:

1. 挿入順を保持する索引（`Vec<String>` の出現順 + `HashMap<String, ThreadAccumulator>`）で events を 1 周。`ThreadCreated` で新規 thread を出現順に登録。
2. 各 accumulator を `finish()` し、`deleted == true` を除外。
3. `sort_by(|a, b| b.updated_at.total_cmp(&a.updated_at))`（既存と同一の安定ソート）。

挿入順を「最初の `ThreadCreated` 出現順」に保つことで、updated_at 同値時のタイブレークが現行（ids を作成順に並べてから安定ソート）と一致する。出力は ThreadDeleted 除外・updated_at 降順で現行と完全一致する。

## データモデルまたは型

- 新規型は `SessionReviewContext`（`usecase/agent_session/session/mod.rs`）と `ThreadAccumulator`（`review_comments/mod.rs` 内、private）。
- `SessionMeta`（`usecase/agent_session/session/mod.rs`）は変更しない。review 直接経路は dir 形式 meta から必要フィールドだけを `SessionReviewContext` として返す。
- `ReviewThread` / `ReviewEvent` / `ReviewComment` 等の外部型は不変。
- 公開境界: `cli::run()` 以外に clap AST を公開しない既存方針を維持。新メソッドは usecase port / facade 経由でのみ露出する。

## 処理フロー

### review get / history（worktree 解決のみ）

```
review_worktree_from_session(data_dir, session_id)
  └ session_id 空チェック
  └ SessionStore::get_session_review_context(data_dir, session_id)
       └ FileSessionStorage::get_session_review_context
            ├ <id>/meta.json → read_meta_from_dir            (body-independent)
            └ none           → Ok(None) → CliError::NotFound
  └ worktree_path を返す（Closed 許可・actor フィールド不問）
→ store.get_thread / history（ロックレス読み取り、project_thread / 1 パス）
```

### review list / create / comment / resolve（actor 解決）

```
review_actor_and_worktree(data_dir, session_id)
  └ session_id 空チェック
  └ get_session_review_context → 上記同様 1 セッションのみ読む
  └ state == Closed → InvalidInput
  └ backend_id 無し → InvalidInput
  └ selected_model 無し → InvalidInput
  └ ReviewActor::agent(backend_id, model, session_id) + worktree_path
→ list は store.list_threads（ロックレス読み取り + project_threads 1 パス）
→ create/comment/resolve は従来どおり Mutex lock guard + OS 排他で書き込み
```

## エラー処理

- セッション未存在: 直接経路が `Ok(None)` → `CliError::NotFound`（behavior「存在しないセッションは not found」）。
- legacy flat / sidecar しか存在しない session-id: 直接経路が `Ok(None)` → `CliError::NotFound`。list には列挙されず、restore は復元対象外になる。
- 空 session-id: 既存どおり helper 先頭で `CliError::InvalidInput`（behavior「空の session-id は拒否」）。
- 壊れた dir 形式 meta: 直接経路が `Err(invalid_session_error_message_with_id(id))`。`get_session_meta` のエラー伝播（`CliError::Other`）と同じ扱い。汎化文言ポリシー（パス・serde 生メッセージ非露出）を踏襲。
- Closed / backend_id 欠落 / selected_model 欠落: `review_actor_and_worktree` の既存 InvalidInput を維持（actor 要件、behavior の各 Scenario）。Get / History は worktree のみ要求し Closed・欠落を許可（既存挙動維持）。
- 読み取りロックレス化に伴うエラー経路: `acquire_worktree_file_lock` の I/O エラー（`ReviewError::Io`）は読み取り経路から消える。書き込み経路では従来どおり保持。
- ④ projection: `author` / `target` 欠落、`deleted` の thread は `None`（既存 `project_thread` と同一判定）。

## テスト方針

CI と同じ `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` を通す。配置は各モジュール内 `#[cfg(test)] mod tests`。

### ①（セッション総数非依存）

- `session_storage` テスト（`session_storage/tests.rs`）: `get_session_review_context` が `ensure_loaded`（=`loaded` フラグ / `cache`）を経由しないことを検証。具体的には N 件のセッションを配置し、直接経路で 1 件解決後に `cache` が空のまま（全件ロードされていない）であることを assert。
- behavior「指定外セッションの meta を読まずに解決する」: 指定外セッションの `meta.json` を意図的に壊れた JSON にしても、対象 `S1` の解決が成功することで「他セッションを読んでいない」を間接的に検証（total = 2 / 50 で同一結果）。
- CLI テスト（`cli/mod.rs` tests）: `review_actor` / get / history が既存の actor / worktree 解決挙動（behavior の actor 系・worktree 系 Scenario）を維持することを既存テストで担保。

### ②（dir 形式 meta への一本化）

- legacy flat / sidecar しか持たない session-id が `get_session_review_context` で `Ok(None)` になることを検証する。
- review CLI の actor 解決が legacy flat / sidecar を `CliError::NotFound` として扱うことを検証する。
- list は legacy flat / sidecar のみの session を列挙せず、restore は `Ok(None)` を返すことを検証する。

### ③（読み取り非ブロック）

- 既存テスト（`acquire_worktree_file_lock` を保持した状態でのテスト、`mod.rs:1530` 付近）を読み取り経路に合わせて見直す。
- 書き込み中（OS 排他ロック保持を模擬）に読み取り 3 メソッドがブロックせず一貫した結果を返すことを検証。書き込み完了前後どちらか一貫した状態（atomic rename 由来）を返し、中途半端な内容を返さないこと。
- 書き込み系同士は従来どおり排他されること（既存テスト維持）。

### ④（1 パス projection）

- `project_threads` と旧実装（id ごと再走査）の出力が、作成 / 更新 / 削除 / resolve を含む events に対して完全一致することを検証（version / created_at / updated_at / comments / state / 並び順）。
- updated_at 同値の複数 thread でタイブレーク順が現行と一致すること。
- ThreadDeleted を含む thread が除外され、updated_at 降順で並ぶこと（behavior の list 系 Scenario）。
- events 1 周で投影されること（thread 件数ぶん再走査しない）を、events スキャン回数を計測する形（テスト用フックまたは accumulator 駆動の構造）で担保。

## リスクと代替案

- **③ をロックレスではなく共有ロックにする代替案**: 読み取りで `fs2::try_lock_shared` を取得する。書き込みとの順序が OS ロックで明示される利点があるが、書き込み（fsync 含む）中は読み取りがブロックされ、requirements ③「書き込み中の競合で不要にブロックしない」を完全には満たさない。atomic rename で整合性は保証されるため、本設計はロックレスを採る。共有ロックは fallback 候補として記載。
- **review 解決 read model の用途拡大リスク**: `SessionReviewContext` は actor / worktree 解決用の狭い型で、preview/count を持たない。将来 session 一覧等に転用できないことを型で表す。
- **port 実装漏れリスク**: `SessionStore` が受け取る storage port は `AgentSessionStorage` と `SessionReviewContextReader` の合成 trait にし、review CLI の軽量解決を提供できる concrete gateway だけを注入可能にする。無関係な `SessionReaderPort` / `AgentSessionReader` 実装には review-specific API を要求しない。
- **④ のタイブレーク順序差異リスク**: 挿入順保持を誤ると updated_at 同値時の順序が変わり得る → 旧実装との出力一致テストで担保。
- **直接経路と Desktop キャッシュの不整合**: 直接経路はキャッシュを読まず・書かないため、同一プロセスで Desktop 経路（cache）と直接経路が混在しても、直接経路は常にファイル現物を読む。CLI は 1 プロセス 1 コマンドで両経路が混在しないため実害なし。

## 仮定

- #1254 のゴール 4 項目（①②③④）すべてを本変更の対象に含める（requirements 仮定に準拠）。
- ① は `ensure_loaded()` をデフォルト経路から外すのではなく、**review 解決用の直接読み出しメソッドを追加** して review CLI 解決のみが使う形で実現する。Desktop の全件ロード（`list_metas` / セッション一覧）は無変更。
- ② の直接経路は dir 形式 meta のみを対象とし、legacy flat / sidecar は NotFound 経路へ集約する。
- ③ は atomic rename による読み取り整合性を前提に、読み取り専用コマンドをロックレスにする。書き込みは従来どおり in-process Mutex lock guard + OS 排他を維持する。
- ④ の 1 パス projection は出力（updated_at 降順・ThreadDeleted 除外・各フィールド・タイブレーク順）を現行 `project_threads` と完全一致させる。
- 性能要件は絶対レイテンシ閾値ではなく「全走査・本文 reader・二乗走査の不在」という構造で担保する（requirements / behavior に準拠）。

## Open Questions

なし。
