# Requirements

## Type

性能改善 / リファクタリング（内部処理の効率化。外部から観測可能な review CLI の振る舞いは不変）。

関連: #1254 / #1213 / #1247 / `docs/releash-performance-architecture-audit.md`

## 背景と目的

`releash review`（Thread 系 CLI: `list` / `get` / `create` / `comment` / `resolve` / `history`）の動作が、セッション数の増加に伴い重くなる。原因はThread ストア本体ではなく、**その手前のセッション解決処理** にある。CLI は 1 コマンド = 1 プロセスで、`build_session_store()` が毎回新規 `FileSessionStorage::default()` を作るため、セッションキャッシュは常にコールド状態で起動する。この前提のもとで以下のコストが毎プロセス発生する。

本変更の目的は、milestone「性能・メモリ効率改善（Workbench State / Read Model）」の大方針「全量を読む・保持する・再送する・再計算する設計を縮小する」に従い、**review CLI の起動コストをセッション総数・セッション本文量から切り離す** ことである。1 セッションの meta が欲しいだけなのにセッション全件を読む現状の構造を解消し、review CLI のセッション解決は dir 形式 meta のみを対象にする。

### 現状のコスト要因（寄与順、コード調査による事実）

#### ① 全 review コマンドが毎回「全セッション」をロードする（最大要因）

各 review サブコマンドは Thread に触れる前に必ず `--session-id` から actor / worktree を解決する。

- `src-tauri/src/cli/mod.rs`
  - `review_actor_and_worktree()`（行 698-733）— List(849) / Create(902) / Comment(919) / Resolve(932) が呼ぶ。state != Closed・backend_id・selected_model を必須とし actor を構築する。
  - `review_worktree_from_session()`（行 741-753）— Get(888) / History(950) が呼ぶ。meta の存在チェックのみで worktree_path を返す（Closed 許可、読み取り専用）。
- どちらも `build_session_store()`（`src-tauri/src/adaptor/controller/wiring.rs:175-177`、毎回新規 `FileSessionStorage::default()`）→ `get_session_meta()` を経由する。
- `get_session_meta()`（`meta_repository.rs:40-55`）は最初に `ensure_loaded()`（同 85-154）を呼ぶ。`ensure_loaded()` は `<app_data_dir>/sessions/` 配下の **全セッションディレクトリを走査し、各 `meta.json` を逐次 open + parse** してキャッシュを構築する。必要なのは指定 1 セッションだけなのに、毎プロセスでセッション総数に比例した I/O + パースが走る。
- 現状、指定 1 セッションの `meta.json` を直接 open する単独経路は存在せず、必ず `ensure_loaded()` の全走査を経由する。

#### ② review CLI の解決対象を dir 形式 meta に限定する

review CLI の actor / worktree 解決に必要なフィールドは dir 形式の `<sessions>/<id>/meta.json` に本文と分離して保存されている。legacy flat（`<id>.json`）や legacy sidecar（`<id>.meta.json`）を解決対象に含めると本文・旧形式互換の専用 reader が必要になるため、本変更ではこれらを対象外にし、dir 形式 meta が無い session-id は NotFound 経路へ集約する。

#### ③ 読み取り専用コマンドでも排他ファイルロックを取得

`list_threads`（`review_comments/mod.rs:941-957`）/ `get_thread`（959-970）/ `history`（972-985）は読み取りだけなのに、in-process mutex（`gateway.file_lock`）に加えて `acquire_worktree_file_lock()`（514-542）で **OS 排他ロック**（`fs2::try_lock_exclusive`）を取得する。取得失敗時は 10ms 間隔で最大 10 秒リトライする。常駐の review-comments watcher（`review_comments/watcher.rs`: 1 秒ポーリング + 500ms debounce の OS notify）と、書き込み時の `sync_all`（fsync, `mod.rs:871`）が同ディレクトリに同居しており、書き込み中は読み取り CLI が不要に待たされ得る。

#### ④ `project_threads` が O(threads × events)

`project_threads`（`review_comments/mod.rs:781-794`）は全 `ThreadCreated` id を集め、id ごとに `project_thread`（681-779）で events 全体を再走査する。`events.json` 肥大時に `list` が二乗的に遅くなる（件数が小さいうちは無視できる規模）。

## スコープ

- **①** `get_session_meta()` 系に「指定 1 セッションの dir 形式 `meta.json` を直接 open する」経路を設け、review CLI のセッション解決が全件 `ensure_loaded()` を回避できるようにする。
- **②** review CLI / list / restore のセッション解決を dir 形式 meta のみに一本化する。legacy flat / sidecar しか持たない session-id は NotFound 相当にする。
- **③** 読み取り専用 review コマンド（`list` / `get` / `history`）の排他ロックを見直し、共有ロックまたはロックレス読み取りにする（書き込みとの排他は維持しつつ、読み取り同士・読み取りと watcher が不要にブロックしないようにする）。
- **④** `project_threads` を events 1 パスで全 thread を投影する projection に置き換え、`O(threads × events)` を解消する。

## 非スコープ

- Thread ストア（`events.json`）のフォーマット・スキーマ自体の変更。
- review CLI のサブコマンド体系・引数・出力フォーマットの変更。
- session storage の保存正典・ディレクトリ構造の変更（#1247 / #1213 の領域。本変更は既存構造の上での読み出し最適化に限定）。
- CLI プロセス間でセッションキャッシュを永続化・共有する仕組みの導入（1 コマンド = 1 プロセス前提は維持）。
- 書き込み系コマンド（`create` / `comment` / `resolve`）のロック戦略の緩和（書き込みは従来どおり排他を維持する）。
- watcher のポーリング間隔・debounce 値の変更。
- Desktop（Tauri）側 agent session のロード経路に対する挙動変更（review CLI 経路の最適化に閉じる。共有関数を変更する場合も観測可能な振る舞いは不変とする）。

## 要求事項

- review CLI のセッション解決（actor / worktree 解決）が、`sessions/` 配下の全セッションを走査せず、指定 1 セッションの meta を直接読み出せること（①）。
- review CLI の meta 解決が dir 形式 meta だけを読み、legacy flat / sidecar を読まないこと（②）。
- 読み取り専用 review コマンド（`list` / `get` / `history`）が、他の読み取りや書き込み中の競合によって **不要にブロックしない** こと。書き込みとの整合性（書き込み途中の中途半端な状態を読まない）は維持すること（③）。
- `project_threads` が events を 1 パスで投影し、thread 件数 × events 件数の二乗コストにならないこと（④）。
- 既存の外部から観測可能な振る舞いを壊さないこと。具体的には:
  - worktree scope の独立性（あるworktree の thread が別 worktree に混入しない）。
  - actor 解決の挙動（List/Create/Comment/Resolve が state != Closed・backend_id・selected_model を要求し、Closed/欠落時は従来どおりエラー）。
  - Get/History が Closed セッション・過去セッションでも読み取れる挙動。
  - `list` の出力順序（updated_at 降順）と各コマンドの出力内容。
- 上記の最適化を担保するテストが存在すること。

## 受け入れ基準の概要

- `releash review get` / `list` / `history` のセッション解決レイテンシが **セッション総数に比例しない**（全件走査が発生しない）ことを確認できる。
- legacy flat / sidecar しか持たない session-id が review CLI では NotFound、list では非列挙、restore では復元対象外になることを確認できる。
- 読み取り専用 review コマンドが、書き込み中・他読み取り中の競合で不要にブロックしないことを確認できる。
- `project_threads` が events 走査 1 パスで全 thread を投影することをテストで確認できる。
- 既存の振る舞い（worktree scope 独立性、actor 解決、Closed セッション扱い、`list` の並び順）が維持されることを既存・追加テストで確認できる。
- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る。

## 仮定

- 本変更は #1254 のゴール 4 項目（①②③④）すべてを対象に含める。Issue のゴール・受け入れ基準が 4 項目を列挙しているため、部分対応ではなく全項目を扱う方針とする。
- ① の「直接 open 経路」は、`ensure_loaded()` による全件キャッシュ構築をデフォルト経路から外し、単一セッション解決用の軽量経路を追加する形で実現する想定。既存の Desktop 側の全件ロード（session 一覧表示等）の経路はそのまま残し、review CLI 解決のみが軽量経路を使う。具体的な関数分割・公開境界は design.md で精査する。
- ② の最適化は、legacy flat / sidecar の互換 reader を持たず、dir 形式 meta の有無だけで解決可否を決める方針とする。
- ③ の読み取りロックは「共有ロック or ロックレス読み取り」のいずれかとし、どちらを採るか・atomic rename による書き込み（`mod.rs:857-875`）との整合性をどう担保するかは design.md で決定する。本 requirements では「読み取りが不要にブロックしない」かつ「書き込み途中の状態を読まない」という性質のみを要求とする。
- ④ の 1 パス projection は、events を 1 周しながら thread_id ごとに状態を蓄積し、ThreadDeleted を反映する実装を想定する。出力（updated_at 降順、ThreadDeleted の除外）は現行 `project_threads` と一致させる。
- 性能要件は「セッション総数 / 本文量に比例しない」ことを構造（全走査・本文 reader の不在）で担保する方針とし、絶対レイテンシ閾値（ミリ秒）は受け入れ基準に設けない。

## Open Questions

なし。
