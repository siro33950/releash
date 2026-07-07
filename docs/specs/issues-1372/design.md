# Design

関連: #1372
前提ドキュメント: [`requirements.md`](./requirements.md) / [`behavior.md`](./behavior.md)

## 概要

app data（`app_data_dir()` ＝ `com.releash.app` 配下）に無期限蓄積する session / workflow / cache / 旧形式 comment / process record を、`requirements.md` の「削除ルール（正典）」に従って backend 起動時に 1 回 GC する。ロジックは Rust の usecase 層に置き、frontend は関与しない。UI/CLI の新しい操作面は追加しない。観測点は「GC 実行後の app data の残存／削除」と「backend log への削除件数・回収 byte 数出力」。

設計の中核方針:

- **保守的削除**: 各ルールに合致すると確実に判定できたデータのみ削除する。判定不能・読み取り失敗は「残す」に倒す。
- **安全ガード最優先**: active session / running workflow に紐づくデータは、どの経路でも削除しない。判定は削除実行の前段で確定させる。
- **起動を過度にブロックしない**: GC は setup 内で同期実行せず、controller/wiring の composition root から background task として spawn する（既存の `cleanup_orphan_processes`（`lib.rs:74`）と同じ起動フックの直後に非同期で回す）。
- **既存 storage の read-only 走査**: 既存のパス組み立て関数（`layout.rs` 等）と型を再利用し、フォーマットは変更しない。削除は `remove_file` / `remove_dir_all` に限定する。

## 変更対象

### 新規

- `src-tauri/src/usecase/app_data_gc/mod.rs`（GC オーケストレーションとルール評価）
  - サブモジュール分割案: `plan.rs`（削除対象の収集＝判定）/ `sweep.rs`（削除実行＋集計）/ `report.rs`（`GcReport` 型）/ `ports.rs`（協調オブジェクトの trait）。
- `src-tauri/src/domain/app_data_gc/`（`GcReport` / `GcCategory` などの値オブジェクト。外部依存を持たない集計・境界判定のみ）
- `src-tauri/src/adaptor/gateway/app_data_gc/`（filesystem 走査・ディレクトリサイズ計算・削除の具体実装。usecase の port trait 実装）

### 変更

- `src-tauri/src/lib.rs` の `.setup(...)` ブロック（`lib.rs:71` 以降）
- `src-tauri/src/adaptor/controller/wiring.rs`
  - `cleanup_orphan_processes`（`lib.rs:74-84`）の後に `spawn_startup_app_data_gc` を呼び、controller/wiring 側で `spawn_blocking`、request 構築、usecase 呼び出しを行う。`lib.rs` は `data_dir` と `SharedRepoPaths` を渡すだけに留める。

### 参照のみ（再利用する既存資産）

- `adaptor/gateway/agent_session/session_storage/layout.rs`（sessions レイアウト。`sessions_dir` / `session_dir` / `meta_file_in_dir` / `index_file_in_dir` / `tool_outputs_dir_in_dir` / `attachments_dir_in_dir` / `tool_output_file_in_dir` / `attachment_file_in_dir`）
- `usecase/agent_session/session/mod.rs`（`SessionMeta` / `SessionState` / `MessageIndexEntry` / `ToolOutputRef` / `AttachmentRef`）
- `adaptor/gateway/workflow/run.rs`（`WorkflowRun` / `RunStatus::is_terminal()` / `RunStore` / `resolve_worktree_by_run`）
- `adaptor/gateway/workflow/archive_repository.rs`（`workflow_run_archives.json` / `WorkflowRunArchiveRecord.archived_at` / `WorkflowRunArchiveRepository`）
- `infrastructure/process/pid_registry.rs`（`PidFileV1` / 記録 pid・pgid の liveness helper）
- `adaptor/gateway/repository/worktree.rs`（`list_worktrees(repo_path)` → `Vec<Worktree>`、`Worktree.path`）
- `adaptor/gateway/repository/repo_paths.rs`（`SharedRepoPaths = Arc<RwLock<Vec<String>>>`）
- `adaptor/gateway/comment/mod.rs`（現行 `review-comments/` の safe key 生成 `worktree_storage_key()`。旧形式 `comments/` `diff-comments/` `threads/` は保存箇所を持たない廃止フォーマット）
- `adaptor/gateway/workspace_state/repository_impl.rs`（`workspace_state/{worktree_name.replace(['/','\\'],"_")}.json`）

## アーキテクチャと責務分割

```
lib.rs setup（起動フック）
  └─ adaptor::controller::wiring::spawn_startup_app_data_gc
       └─ spawn_blocking ─► gateway::app_data_gc::build_startup_gc_request
                                   └─ usecase::app_data_gc::run_startup_gc(ctx)
                                      ├─ plan:  ルールごとに削除候補を収集（read-only 走査）
                                      ├─ guard: runtime protection snapshot で削除候補をフィルタ（安全ガード）
                                      ├─ sweep: 確定した対象を削除しつつ byte / 件数を集計
                                      └─ report: GcReport を backend log に出力
```

責務分割:

- **domain（`domain/app_data_gc/`）**: 副作用なし。retention 境界判定（`now - updated_at > threshold`）、`GcReport` の集計、`GcCategory` の enum。時刻・閾値は引数で受け取り、`Clock` や fs には触れない。
- **usecase（`usecase/app_data_gc/`）**: ルール評価のオーケストレーション。request から「生存 worktree resolution」「runtime protection snapshot」「process record」「現在時刻」を受け取り、port trait 経由の filesystem 走査・サイズ計測・削除で削除計画を組み立て、安全ガードでフィルタし、sweep を指示する。ここが本 Issue のロジックの中心。
- **adaptor/gateway（`adaptor/gateway/app_data_gc/`）**: port trait の具体実装と request 用 read model の構築。`std::fs` によるディレクトリ列挙・`mtime` 取得・再帰サイズ計算・削除。live worktree 解決は `SharedRepoPaths` を読み各 repo で `list_worktrees()` を呼ぶ。active session / running workflow の保護判定は 1 つの runtime protection snapshot（active session id、running worktree path、storage key 保護集合）として構築し、全削除経路が同じ snapshot を使う。
- **controller**: Tauri command / WebSocket handler は追加しない。起動 runner は controller/wiring の composition root に置き、`spawn_blocking`、panic 捕捉、gateway request 構築、usecase 呼び出しをここで配線する。

port trait を挟む理由: GC ロジックを usecase 層で完結させ、tempdir + fake collaborator で単体テスト可能にするため（`docs/architecture/TEST.md` / USECASE 規約に沿う）。

## データモデルまたは型

### GC 入力コンテキスト（usecase 引数）

```rust
struct GcContext {
    app_data_dir: PathBuf,
    // 生存 worktree の絶対パス集合（正規化済み）と、list_worktrees 失敗 repo の保守スキップ情報。
    live_worktrees: Option<LiveWorktreeResolution>,
    // state=Active / live pid / running workflow から 1 回だけ構築した保護 snapshot。
    runtime_protection: RuntimeProtection,
    now: f64,                                // Unix 秒。domain の境界判定に渡す
    retention: RetentionPolicy,              // archived_log=30d, cache=7d
}
```

- `LiveWorktreeSet`: worktree 絶対パスの `HashSet`（`FsWorktreePathNormalizer` 相当で正規化）と、そこから派生する storage key（`workspace_state` 用 `name.replace(['/','\\'],"_")`、`review-comments` 用 `worktree_storage_key()`）の集合を保持。ファイル名ベースの領域はこの派生キー集合で「生存」を判定する。
- `LiveWorktreeResolution`: 成功 repo の `LiveWorktreeSet` に加え、`list_worktrees()` に失敗した repo path と `workspace_state` 用 key prefix を保持する。metadata に `worktreePath` を持つ session / workflow は failed repo 配下なら保持し、それ以外は成功 repo の live 集合で判定を継続する。hash key で元 path へ戻せない `review-comments` は failed repo がある回の workspace cleanup を保守的にスキップする。
- `RuntimeProtection`: `active_session_ids`（state=Active もしくは live pid を持つ session）、`running_worktrees`（running workflow が占有する worktree_path）、`protected_worktrees`（両者から派生した storage key 保護集合）をまとめた read model。session / workflow 本体、workspace_state、review-comments、worktree との対応付けが証明できる checkpoint はこの同じ snapshot を使って保護判定する。保護 source の読み取り・解析が不完全な場合は workspace-keyed cleanup を保守的にスキップする。
- `RetentionPolicy { archived_log_secs: 30*86400, cache_secs: 7*86400 }`（定数。UI 設定は非スコープ）。

### 判定に用いる既存型（再掲・確定事項）

- `SessionMeta`（`usecase/agent_session/session/mod.rs`）: `worktreePath: String` / `state: SessionState` / `updatedAt: f64`。
- `SessionState`（snake_case）: `active` / `idle` / `done` / `error` / `closed` / `archived`。soft-delete の `deleted` state は存在しない。requirements.md の正典に合わせ、session state は次のように扱う:
  - **ルール2-3「使用中」（無期限保持）= `Active` / `Idle` / `Done` / `Error`**（`list_sessions()` に表示される使用中の session。古さに関わらず保持）。
  - **ルール2-2「アーカイブ済み」（30日で削除）= `Archived`**（`meta.json` の `state = "archived"`。`updatedAt` から 30 日以内は保持、30 日超で削除）+ **復元可能な workflow archive**（`restore_manual`/`archived_at` を持つ。`archived_at` から 30 日超で削除）。
  - `Closed` session は既存 UI の復元候補として残る棚上げ状態であり、破壊的即削除にはしない。容量回収上は `Archived` と同じ 30 日 retention の対象に含める。
  - **ルール2-1「削除済み」（即削除）= 復帰不可の残骸**（物理削除の中途失敗・fork rollback 残骸・`meta.json` 欠落／解析不能な orphan session ディレクトリ）。
- `ChatMessage`（`messages/*.json` の要素）: `MessagePart::ToolResult.content_ref.id` / `MessagePart::ImageRef.attachment.id`。参照集合は message 本体から union で得る。`index.json` は stale になり得るため、参照切れ blob 判定の source of truth にはしない。
- `WorkflowRun`（`workflow_runs/{run_id}.json`）: `worktreePath: String` / `status: RunStatus` / `updatedAt: f64` / `completedAt: Option<f64>`。`RunStatus::is_terminal()` で running 判定。
- `WorkflowRunArchiveRecord`（`workflow_run_archives.json`）: `archived_at: Option<f64>` / `restored_at: Option<f64>`。archive 判定と 2-2 の起点時刻に使用。
- `PidFileV1`（`agent-processes/{session}.{backend}.{pid}.json`）: 記録された `pid` の liveness が `Live` / `Stale` / `Unknown` を返す。`Stale` を削除、`Unknown` は保守的に残す。legacy `pids/{session}.pid` は記録 pgid の liveness で判定する。

### 出力型

```rust
struct GcReport {
    categories: BTreeMap<GcCategory, CategoryStat>, // 件数 + 回収 byte
    total_files: u64,
    total_bytes: u64,
    errors: u64,      // 削除失敗（残置）
}
struct CategoryStat { deleted: u64, reclaimed_bytes: u64 }
enum GcCategory {
    DeletedWorkspace,       // ルール1（worktree 消滅＝復帰不可）
    UnrecoverableSession,   // ルール2-1（orphan 残骸）即削除
    RecoverableExpired,     // ルール2-2（Archived/Closed session / archived workflow, 30日超）
    RegenerableCache,       // ルール2-4
    LegacyComments,         // 旧形式全削除
    OrphanBlob,             // 参照切れ tool_outputs/attachments
    StaleProcessRecord,     // stale pid
}
```

## 処理フロー

`run_startup_gc(ctx)` は以下を順に実行する。1〜6 で「削除候補」を集め、各段で安全ガードを適用し、最後に一括 sweep + log 出力。

### 0. コンテキスト構築（保護集合の確定を最優先）

1. `SharedRepoPaths` を読み、各 repo_path で `list_worktrees()` を呼び、生存 worktree 絶対パス集合 `live_worktrees` を構築（正規化）。派生 storage key 集合も生成。
2. running workflow: `RunStore` の active 集合から `is_terminal()==false` の run の `worktree_path` を収集 → `running_worktrees`。
3. active session: `sessions/` 各 session の `meta.json` を読み `state==Active` を収集。加えて `agent-processes/` を走査し、記録 pid が live な pid record が指す `session_id` も active 扱いで追加（保護を過剰側に倒す）。
4. `now` を取得（`Clock`）。

### 1. ルール1 — 削除済み Workspace のログ

- `sessions/` 配下の各 session の `meta.json.worktreePath` が `live_worktrees` に無ければ、その session ディレクトリ一式を削除候補にする。
- `workflow_runs/*.json` の `worktreePath` が `live_worktrees` に無ければ、その run の `workflow_runs/{id}.json` と `workflow_logs/{id}.json` を候補にする。
- `workspace_state/*.json` / `review-comments/*` は、ファイル名の storage key が生存キー集合に無いものを候補にする。`agent-worktree-checkpoints`・`-backups` は命名規約から worktree との対応付けを確実に証明できるまで保守的にスキップする。
- **ガード**: 候補 session が active_session_ids に含まれる、または候補 worktree が running_worktrees に含まれる場合は除外。

### 2. ルール2 — 使用中 Workspace のログ（Session/Workflow 状態）

生存 worktree に紐づく session / workflow に対して:

- **2-1 削除済み（復帰不可＝即削除）**: `meta.json` 欠落／解析不能で active でもない orphan session ディレクトリなど、復帰不可と判定できる残骸を削除。workflow は削除経路が無いため 2-1-workflow は対象なし（log で 0 件を明示）。
- **2-2 アーカイブ済み（30日猶予）**: session は `state==Archived` か `state==Closed` かつ `now - updatedAt > 30d` を削除、30 日以内は保持。workflow は `workflow_run_archives.json` に `archived_at` があり `restored_at` が無く `now - archived_at > 30d` の run（および対応 log）を削除。
- **2-3 使用中（無期限保持）**: `state` が Active/Idle/Done/Error（`list_sessions` に表示される使用中）の session 本体は古さに関わらず保持。ただし参照切れ blob は 4 で例外的に削除。
- **2-4 再生成 cache**: `lsp/`（`jdtls` / `jdtls-workspaces` / `typescript`）配下のエントリを、`mtime` が `now - 7d` より古ければ削除。7 日以内は保持。state に依存しない。

#### retention 判定の詳細（30日 / 7日の測り方）

- **時刻の単位**: app 内タイムスタンプ（`meta.updatedAt` / `createdAt` / `SessionClosed.at` / workflow `archived_at`）は `now_timestamp()` = `unix_timestamp_seconds()` による **Unix 秒（f64）**。GC の現在時刻も同関数で取得する。ファイル `mtime` は `std::fs::Metadata::modified()`（`SystemTime`）を `UNIX_EPOCH` からの秒に変換して同一軸で比較する。
- **判定式**: `now_sec - 起点sec > 閾値sec`（30日 = `30*86400` = 2_592_000、7日 = `7*86400` = 604_800）。境界は「超えたら削除」（`>`）＝ちょうど 30日/7日は保持。
- **now の固定**: GC 実行開始時に `now` を 1 回取得し、全判定で共有する（実行中の経過で境界がブレないようにする）。domain の境界判定は `fn is_expired(now: f64, at: f64, threshold: f64) -> bool` の純関数で、`Clock` にも fs にも触れない。
- **起点時刻（category 別）**:
  - 2-2 Archived/Closed session … `meta.updatedAt`。state 変更時に `set_session_state` で `updated_at = now_timestamp()`（`store.rs:686`）に更新されるため、実質「archive/close した時刻」を表す。restore されると Idle に戻り 2-2 対象から外れる。
  - 2-2 archived workflow … `workflow_run_archives.json` の `archived_at`。`restored_at` が入っているものは復帰済みとして対象外。
  - 2-4 cache … 対象エントリ（例: `lsp/jdtls-workspaces/{workspace}`, `lsp/typescript`）配下の**最新 mtime**を代表値に採る。一部だけ古いファイルがあっても、直近に更新があれば「使用中 cache」とみなして残す（誤削除防止）。エントリ全体の最新 mtime が 7日超で削除。
- **updatedAt 更新タイミングの注意**: `set_session_state` は state 変更のたび `updated_at` を更新する。Archived/Closed 後に何らかの理由で `updated_at` が再更新されると 2-2 の起点が後ろへずれる（＝削除が遅れる方向＝安全側）。逆に「archive/close より後の最終活動」を起点にしたい場合も同義になり、いずれも保守的。

### 3. 旧形式データの全削除

- `comments/` / `diff-comments/` / `threads/` の各ディレクトリを丸ごと削除候補にする（状態・worktree に無関係）。`review-comments/` は対象外。

### 4. 参照切れ blob（ルール2-3 の例外）

- 生存・active を問わず全 session を対象に、その session の `messages/*.json` から `ToolOutputRef.id` ∪ attachment id を集める。
- `tool_outputs/` / `attachments/` 配下の各ファイル id がその集合に無ければ、そのファイル単位で削除。messages / 会話本体には触れない。
- `messages/` が存在しない session は参照 0 件として扱う。`messages/` の列挙が権限エラー・一時的 I/O error などで失敗した場合、または message が読めない／解析できない場合は、その session の参照切れ判定を保守的にスキップする。

### 5. stale process / pid record

- `agent-processes/*.json`（`PidFileV1`）は記録 `pid`、`pids/*.pid` は記録 pgid を走査し、`Stale` のレコードを削除。`Live` / `Unknown` は残す。

### 6. sweep + 集計 + log

- 1〜5 で確定した候補を安全ガードで最終フィルタし削除。各削除前にサイズ（ファイル or 再帰ディレクトリサイズ）を測り `GcReport` に加算。削除失敗は `errors` に計上し残置（保守的）。
- 完了後、`cleanup_orphan_processes` と同様に `log::info!` で category 別件数・回収 byte・total を 1 行で出力。

## エラー処理

- **read-only 走査中の失敗**（ディレクトリ列挙・JSON 解析・mtime 取得の失敗）: そのエントリを「判定不能」として削除候補から除外し残す。GC 全体は継続。個別失敗は `warn` ログ。
- **削除失敗**（`remove_file` / `remove_dir_all` のエラー）: `GcReport.errors` に加算し、他候補の処理を継続。
- **collaborator 失敗**（`list_worktrees()` がある repo で失敗など）: その repo の worktree を「生存側」に倒せないため、**live 集合を過小に見積もると誤削除に繋がる**。よって repo 単位で `list_worktrees()` が失敗した場合は、その repo に属し得るデータ（判定に生存集合を要するルール1・2）の削除を**その回の GC ではスキップ**する（保守的）。成功した repo の live 集合では判定を継続する。参照切れ blob（4）・stale pid（5）・旧形式（3）・cache（2-4）は生存集合に依存しないため継続可能。
- **原子性**: session ディレクトリ削除は `remove_dir_all` 一括。途中失敗で中途半端に残っても、次回起動時の GC が再度候補化して回収するため冪等。個別 blob / record 削除も冪等。
- **起動処理保護**: GC は background task。setup 完了・UI 起動をブロックしない。panic は task 内で捕捉し log 出力（アプリ本体に伝播させない）。

## テスト方針

`usecase/app_data_gc` を tempdir + fake collaborator で単体テスト（`#[cfg(test)] mod tests`）。各ルールの削除／保持を fixture で検証:

- ルール1: 生存 worktree に紐づく session/workflow/workspace_state を保持、非生存を削除。`list_worktrees()` 失敗時に該当 repo 分を削除しない（保守）ことを検証。
- ルール2-1: `meta.json` 欠落の orphan dir を削除。
- ルール2-2: `state==Archived` / `state==Closed` の session を updatedAt 31日で削除・30日で保持。archived workflow を `archived_at` 31日で削除・30日で保持。`restored_at` があるものは保持。
- ルール2-3: 使用中（Active/Idle/Done/Error）の session を任意の古さで保持。
- ルール2-4: `lsp/` cache を mtime 8日で削除・7日で保持。
- 旧形式: `comments/`・`diff-comments/`・`threads/` を削除、`review-comments/` を保持。
- 参照切れ: `messages/*.json` に無い id の blob を削除、参照中 blob を保持、`messages/` を保持。stale な `index.json` だけに残る id は参照中扱いにしない。
- stale pid: 記録 pid/pgid liveness を fake で `Stale`→削除 / `Live`・`Unknown`→保持。
- 安全ガード: active session・running workflow に紐づくデータが他ルールに合致しても保持。
- 集計: `GcReport` の件数・回収 byte が実削除と一致。削除失敗が `errors` に計上され残置。

domain の retention 境界判定は純関数として境界値テスト。`cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` を通す。

## リスクと代替案

- **誤削除リスク（最重要）**: live worktree 集合の取りこぼしはルール1の誤爆に直結する。緩和として (a) `list_worktrees()` 失敗 repo はスキップ、(b) active/running 保護を過剰側（Idle・live pid も保護）に倒す、(c) 判定不能は残す、で多重に保守化。代替案として初回リリースを「削除件数のみ log 出力し実削除しない observe モード」から始める案があるが、`requirements.md` の非スコープ（dry-run 不要）と齟齬するため採用しない。
- **session state の解釈**: `Archived` は requirements.md の「アーカイブ済み Session」として 30 日 retention の対象にする。`Closed` も既存 UI の復元候補として即削除せず、同じ 30 日 retention に含める。将来 state の意味や復元 UI が変わる場合は、本 GC 分類を見直す。
- **`SharedRepoPaths` が起動直後は config 由来の `last_repo_paths` のみ**で、ユーザーがまだ開いていない既知リポジトリの worktree を取りこぼす可能性。→ その worktree に紐づくデータを「削除済み Workspace」と誤判定し得る。緩和: GC は setup 内で `shared_repo_paths` 初期化（`lib.rs:124-135`）の**後**に spawn する。なお UI 操作で後から repo が増えるが、GC は起動時 1 回のため、当該起動時点の既知集合で判定する（次回起動で是正）。誤判定を避けるため、`last_repo_paths` が空のときはルール1（生存集合を要する経路）を丸ごとスキップする。
- **checkpoint ディレクトリの命名規約が Rust 層で未確認**（`agent-worktree-checkpoints/`）。実データには存在するが生成箇所が Rust に無い。→ 実装時にディレクトリ実体を走査してキー体系を確認し、確実に worktree と対応付けられる場合のみルール1対象にする。対応付け不能なら保守的にスキップ（残す）。
- **`updatedAt` の単位/更新タイミング（確認済み）**: 単位は Unix 秒（`unix_timestamp_seconds()`）。state 変更は `set_session_state` を経由し `updated_at` を更新するため、Archived/Closed session の 2-2 起点＝archive/close 時刻として使える（`store.rs:483,686`）。archive/close 後に再更新された場合も起点が後ろへずれる＝削除が遅れる安全側。workflow は専用の `archived_at`。
- **性能**: 数十 GB・多数ファイルの走査コスト。background task なので UI はブロックしないが、GC 自体は数秒〜数十秒かかり得る。参照切れ判定は `messages/*.json` の参照集合を source of truth とし、ディレクトリサイズ計算は削除対象のみに限定する。

## 仮定

- GC は起動時 1 回、background task で実行（同期実行しない）。二重起動時の排他は単一プロセス前提で不要とみなす（複数 Releash プロセス同時起動は既存 orphan cleanup と同様、レコード単位冪等でカバー）。
- retention 閾値は定数（archived log 30 日 / cache 7 日）。UI 設定は非スコープ。
- retention 起点時刻: 2-2 の Archived/Closed session は `meta.updatedAt`、archived workflow は `workflow_run_archives.json.archived_at`、cache はファイル `mtime`。
- 参照切れ判定は `messages/*.json` の `ToolOutputRef.id` / attachment id の union を正とする。message が読めない session は保守的にスキップする。
- active 判定は `state==Active` ∪ live pid record の session。running workflow は `RunStore` の非 terminal run。両者を保護集合とする。
- 削除ルールの中核は復帰可能性で、spec のラベルはコード state 名と名前一致しない。**使用中（Active/Idle/Done/Error）は無期限保持（2-3）**、**Archived/Closed session と archived workflow は 30日で削除（2-2）**、**復帰不可の orphan 残骸は即削除（2-1）**。
- 旧形式 `comments`/`diff-comments`/`threads` は移行せず全削除（使用中分の旧コメント喪失をユーザー承知）。
- 生存 worktree 集合は「その起動時点の `SharedRepoPaths` × `list_worktrees()`」で確定。

## Open Questions

なし。
