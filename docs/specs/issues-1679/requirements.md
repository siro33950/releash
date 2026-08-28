# Context

## 入力文書

- 要求の正本: [Issue #1679](https://github.com/siro33950/releash/issues/1679)「[event store] 事実行の append ごとに tokio ランタイムを構築しており fd 上限 256 を踏む」（state: OPEN、label: bug、milestone なし、comment なし）
- 補助資料（本 Issue の要求ではない。境界の確認にのみ使う）
  - [Issue #1678](https://github.com/siro33950/releash/issues/1678) — 起動に失敗した Session node が resume も retry も受け付けず復旧不能になる（OPEN）。本 Issue の EMFILE がその発端の 1 つ。
  - [Issue #1677](https://github.com/siro33950/releash/issues/1677) — セッション同時上限（`per_worktree_cap: 32` / `max_panes_total: 64`）の値と機構（OPEN）。同じく並列度を上げたときに踏む上限。
- 調査で参照した実装
  - `src-tauri/src/adaptor/gateway/workflow/fact_log.rs:427-450` — `append_pending_rows_blocking`（事実行 append の唯一の集約点。OS thread の spawn と tokio ランタイムの新規構築）
  - `src-tauri/src/adaptor/gateway/workflow/fact_log.rs:389-424` — `append_facts_for_events`（事実列を行へ写像して `:423` で上記へ渡す）
  - `src-tauri/src/adaptor/gateway/workflow/fact_log.rs:652-659` — `append_single_fact`（単独の事実を `:658` で上記へ渡す）
  - `src-tauri/src/adaptor/gateway/workflow/fact_log.rs:453-540` — `append_fact_batch_for_seed`（`#[cfg(test)]`。`:528` に 2 つ目のランタイム構築があるが test 専用）
  - `src-tauri/src/adaptor/gateway/local_event_store/store.rs:972-997` — `append_node_event`（async。writer thread へ request を渡して oneshot を await する。行単位 INSERT で原子性は 1 行を超えない）
  - `src-tauri/src/adaptor/gateway/local_event_store/store.rs:1005-1013` — `submit_indexed_query_blocking`（reader pool 上の同期読み出し。ランタイムを要さない）
  - `src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:1497` / `:2266` / `:687` — 失敗文言の合成箇所と、起動時 reconciliation の木ごとループ
  - `src-tauri/src/adaptor/gateway/workflow/event_log_writer.rs:22-30` / `:39-45` — 事実行 append の入口（後者は async fn から同期 append を呼ぶ）
  - `src-tauri/src/adaptor/gateway/workflow/worktree_ledger_repository.rs:124` — isolated worktree ledger からの事実行 append
  - `src-tauri/src/lib.rs:619-621` — アプリケーション全体の tokio ランタイム構築と Tauri async runtime への設定
  - `src-tauri/src/cli/common.rs:138-139` — CLI 側の事実行 append 呼び出しを含む `test_support` module が `#[cfg(test)]` であること
- 規約: `AGENTS.md`「構成で押さえる点」「アーキテクチャ原則」「セキュリティ」、`docs/glossary/DOMAIN.md`「使用禁止語」

## 確定済みの背景と制約

- 永続化は event store であり、事実を追記して読み側で projection を導出する。full-recompute 経路を増やさない（`AGENTS.md`）。
- node_events への追記は 1 行単位で、原子性が 1 行を超えることはない（`store.rs:972-976` のコメントが明示）。
- 事実行 append の入口は `append_facts_for_events` と `append_single_fact` の 2 つで、どちらも `append_pending_rows_blocking` へ集約される。この関数は同期関数であり、内部で OS thread を spawn し、その thread 内で `enable_all()` 付きの current-thread tokio ランタイムを新規構築し、`runtime.block_on` で async の `append_node_event` を呼ぶ。
- 事実行 append の production 呼び出し元は GUI プロセスに限られる。workflow host の activation / commit 経路、`event_log_writer`、`worktree_ledger_repository`、および起動時 reconciliation（`workflow_host.rs:687` の木ごとループから `fact_log.rs:857`）である。`cli/common.rs` の append 呼び出しは `#[cfg(test)]` の `test_support` 配下だけで、CLI プロセスは production では事実行を書かない。
- 事実行 append は同期文脈と async 文脈の両方から呼ばれる。async 文脈からの呼び出しは `event_log_writer.rs:45`（provider Stop 受理）と workflow host の activation 経路にある。現在は専用の OS thread を挟むことで、async 文脈からの `block_on` が成立している。
- アプリケーション全体の tokio ランタイムは `lib.rs:619` に 1 つ存在し、Tauri の async runtime として設定されている。事実行 append 経路だけがこれを共有していない。
- `src-tauri/` に `setrlimit` / `RLIMIT_NOFILE` の記述はない（grep により確認）。起動時に fd soft limit を引き上げる処理は存在しない。
- 対応プラットフォームは macOS（`AGENTS.md`）。macOS の GUI アプリは launchd から起動されるため、その soft limit を継承する。
- `docs/glossary/DOMAIN.md` の使用禁止語により、記録された事実を指すときに `WorkflowEvent` を用いない。本文書では「事実」「事実行」と呼ぶ。

# Outcome

- 対象者: Releash で workflow を実行する開発者。特に fanout や複数 worktree で workflow を並列に走らせる利用者。
- 現在の問題: node の事実行を 1 回 append するたびに、その append 専用の tokio ランタイムを新規構築する。ランタイム構築は I/O driver のために file descriptor を確保するので、事実の追記が SQLite への書き込みとは別に fd を要求する。GUI プロセスの fd soft limit は 256 のまま引き上げられていないため、並列実行で fd 使用量が上限に近づくと、事実行の append が EMFILE で失敗する。事実ログの append が失敗するということは workflow の実行状態そのものを記録できないということであり、失敗した append が `session_attached` だった場合、node は AgentSession を持たないまま停止する。
- 変更後に実現する状態: 事実行の追記が、追記の回数や同時数に応じた file descriptor を要求しない。事実を書けるかどうかは書き込み先の状態だけで決まり、追記処理自身が確保する OS リソースが失敗要因にならない。

# Current Behavior

## 実障害（Issue の報告、2026-08-23）

11:41:57、feat-issues-1652 worktree の `02_implement-existing-spec`（execution `1182b833`）で `create_detailed_design` attempt 2 の activation が次の理由で失敗した。

```
workflow runtime activation failed: session attachment event append failed:
failed to create fact append runtime: Too many open files (os error 24)
```

`session_attached` の事実を 1 行 append しようとして OS の fd 上限（EMFILE）に達し、node execution は `sessionId` を持たないまま `paused` で残った。Issue 起票時点の実測は、workflow 4 本が走っている状態で fd 168 / 上限 256。

この障害は Issue の報告であり、本文書の作成にあたって再実行はしていない。以下はコードと作業環境に対して確認した内容である。

## 失敗文言がどこで作られるか

| 文言 | 生成箇所 |
|---|---|
| `failed to create fact append runtime: {error}` | `fact_log.rs:439` |
| `session attachment event append failed` | `workflow_host.rs:1497`（commit 経路の `append_error_context`） |
| `workflow runtime activation failed: {error}` | `workflow_host.rs:2266`（`settle_runtime_failure_for_node`） |

EMFILE は SQLite への書き込み時ではなく、append 用ランタイムの構築時に起きている。

## append ごとに fd を要求している箇所

`fact_log.rs:427-450`。

```rust
pub(crate) fn append_pending_rows_blocking(
    store: &Arc<LocalEventStore>,
    rows: Vec<PendingFactRow>,
) -> Result<(), String> {
    if rows.is_empty() {
        return Ok(());
    }
    let store = Arc::clone(store);
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| format!("failed to create fact append runtime: {error}"))?;
                for pending in rows {
                    runtime
                        .block_on(store.append_node_event(pending.row, Some(pending.timestamp_ms)))
                        .map_err(|error| format!("node fact append failed: {error}"))?;
                }
                Ok::<(), String>(())
            })
            .join()
            .map_err(|_| "node fact append worker panicked".to_string())?
    })
}
```

- 呼び出しごとに OS thread を spawn し、その中でランタイムを新規構築する。`enable_all()` は I/O driver を有効にするため、構築時に kqueue とその起床用 descriptor を確保する。ファイルを開く処理ではないのに fd を要求するのはこのためである。
- ランタイムは関数の終了時に drop されるので、確保した fd は解放される。したがって蓄積し続ける leak ではなく、append の実行中にだけ増える一時的な使用量である。並列に走る append の数だけこの一時使用が重なり、ピークが上限を越える。
- 同じ file の読み出し側（`submit_indexed_query_blocking`、`store.rs:1005`）は store の reader pool 上で同期実行され、ランタイムを構築しない。ランタイムを構築しているのは append 経路だけである。

## append が起きる頻度

node の事実は node 開始・session attach・artifact 産出・完了ごとに書かれるため、workflow の実行中は継続的に発生する。加えてアプリ起動時の reconciliation が木ごとに走り（`workflow_host.rs:687`）、前進した分の事実を追記する（`fact_log.rs:857`）ため、起動直後にも集中する。

## fd soft limit

作業環境（macOS Darwin 25.5.0）で実測した値は次のとおりで、Issue の記載と一致する。

```
launchctl limit maxfiles:  soft 256 / hard unlimited
kern.maxfilesperproc:      245760
```

プロセスに効いているのは soft limit の 256 であり、`kern.maxfilesperproc` の 245760 ではない。hard limit が unlimited なので `setrlimit(RLIMIT_NOFILE, ...)` でプロセス自身が引き上げられる状態にあるが、`src-tauri/` にその記述はない。

セッション 1 本あたり PTY の master/slave、provider プロセスとのパイプ、terminal journal のファイルハンドルが積み上がり、そこに SQLite（本体 + WAL + shm）と、上記の一時ランタイムが乗る。

## 再現条件

GUI プロセスの fd soft limit が 256 の状態で、workflow を並列に実行して fd 使用量を上限付近まで押し上げ、その状態で node の activation（`session_attached` の追記）を行う。append 用ランタイムの構築が EMFILE で失敗し、上記の文言で activation が失敗する。fd 使用量が上限から十分離れていれば同じ append は成功する。

# Scope / Non-goals

## 変更する

- 事実行 append 経路（`fact_log` の `append_pending_rows_blocking` と、そこへ集約される `append_facts_for_events` / `append_single_fact`）が 1 回の追記のために確保する OS リソース。
- 上記の変更に伴って必要になる、呼び出し元（workflow host、`event_log_writer`、`worktree_ledger_repository`、起動時 reconciliation）からの呼び出し形。

## 変更しない

- 記録される事実の種類と内容、node_events の行構造、追記の原子性の単位（1 行）。純粋事実ログ（追記のみ、読み側で projection を導出する）という永続化の形。
- 起動に失敗した Session node の resume / retry / restart の責務分担（#1678）。本変更は append が fd を理由に失敗しないようにするものであり、既に `sessionId` を持たないまま `paused` で残った node の復旧手段は扱わない。
- セッション同時上限（`per_worktree_cap` / `max_panes_total`）の値と、上限到達時に拒否するか回収するかの機構（#1677）。
- プロセス起動時の fd soft limit。`setrlimit(RLIMIT_NOFILE, ...)` による引き上げは行わず、起動元から継承した値のまま動作する。
- PTY、provider プロセス、terminal journal、SQLite 本体が使う fd の量そのもの。fd 使用量の内訳のうち、事実行 append 以外の削減は扱わない。
- fd 使用量を観測・通知する仕組みの追加。

# Requirements

- R-001: 事実行の append が、その append のためだけの file descriptor を確保しない。事実行を追記している間も含めて、追記そのものを理由にプロセスの open fd 数が増えない。
- R-002: 事実行の append が、file descriptor を確保できないことを理由に失敗しない。現在この失敗として表面化している `failed to create fact append runtime: Too many open files (os error 24)` が発生しない。追記先である SQLite 自体に起因する失敗はこの限りでない。
- R-003: 事実行の append を、同期文脈と async 文脈のどちらからも呼び出せる。現在 async 文脈から呼ばれている経路（provider Stop の受理、node activation 時の事実追記）が、panic、deadlock、実行の停止を起こさずに動作する。
- R-004: 記録される事実行の内容と、同一 node に対する追記順序が変更前と一致する。追記の原子性の単位を 1 行から変えない。
- R-005: 事実行の append が失敗した場合に、その失敗が呼び出し元へ伝わり、node の失敗として扱われる現在の性質を変えない。追記できなかった事実を、追記できたものとして扱わない。追記先の store が内部状態の破壊により利用不能になっている場合は、事実行の読み出しと同じ扱いとし、この要件の対象に含めない。

# Assumptions / Open Questions

## Assumptions

- なし。

## Open Questions

- なし。
