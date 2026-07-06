# Requirements

関連: #1372

## Type

新機能（backend GC）。無期限に蓄積する app data を、削除ルール（削除済み Workspace・Session/Workflow 状態別のログ・再生成 cache・旧形式データ・参照切れファイル・stale process record）に従って削除し容量を回収する。外部から観測可能な UI/CLI の振る舞いは追加しない（アプリ起動時の内部処理）。

## 背景と目的

Releash は app data ディレクトリ（macOS では `~/Library/Application Support/com.releash.app`）配下に、session data・workflow log/artifact・LSP cache・comment/thread・checkpoint・process record・runtime file などを保存する。これらが無期限に蓄積し、通常利用だけで数十 GB の SSD 容量を消費する。

実測（開発機、2026-07-05 時点）でも次のとおり `sessions` と `lsp` が支配的であり、放置されたデータが容量の大半を占める:

- `sessions/` … 約 29 GB（最大要因。agent session 本体・`events.json`・`messages/`・`tool_outputs/`・`attachments/`）
- `lsp/` … 約 1.4 GB（`jdtls` / `jdtls-workspaces` / `typescript` の再生成可能 cache）
- `agent-worktree-checkpoints/` … 約 31 MB
- `workflow_logs/` … 約 18 MB
- `review-comments/` … 約 11 MB
- ほか `comments/`・`diff-comments/`・`threads/`（旧形式）、`agent-processes/`・`pids/`（process/pid registry）、`workflow_runs/`・`workflow_pending/` 等

本変更の目的は、プロダクト方針「全量を保持する設計を縮小する」に従い、**backend 側に GC を実装し、不要になった app data を安全に削除して容量を回収する**ことである。削除は「もう参照されていない／再生成可能である」ことが判定できるデータに限り、実行中の作業（active session / running workflow）や、まだ参照されている Workspace の生きたデータを壊さないことを最優先とする。

### 削除ルール（正典）

GC の削除判定は次の階層で行う。中核は**復帰可能性**であり、単純な古さ（保持日数）ではない。

1. **削除済み Workspace のログ** — 復帰不可能なので削除する。
2. **使用中 Workspace のログ** — 中の Session / Workflow の状態で判断する。
   1. 削除済みの Session / Workflow のログ — 削除する。
   2. アーカイブ済みの Session / Workflow のログ — 30 日を超えたら削除する。
   3. 使用中の Session / Workflow のログ — 削除しない。
   4. 再生成可能なキャッシュ — 状態に関わらず 7 日を超えたら削除する。

上記のログ／キャッシュ判定に加えて、Issue #1372 が名指しする次の3種類を削除する。これらは「Session/Workflow のログ」でも「キャッシュ」でもないため、上の階層とは別軸で扱う。

- **旧形式 `comments` / `diff-comments` / `threads`** — 現行 `review-comments/` に置換済みの廃止フォーマット。状態に関わらず全て削除する（新形式への移行はしないため、使用中 Workspace 分の旧コメントも失われる。ユーザー確認済み）。
- **参照切れ `tool_outputs` / `attachments`** — Session が**使用中でも**、その Session のどの message からも参照されていない blob はファイル単位で削除する（ルール 2-3 の例外。生きている会話には触れず、誰も参照していない実体だけを消す）。
- **stale な process / pid record** — `agent-processes/{session}.{backend}.{pid}.json` / `pids/` のうち、記録された pid のプロセスが既に死んでいるものを削除する。

**安全ガード**: active session / running workflow に紐づくデータは、上記いずれの経路でも削除しない。

#### 補足（実装調査による事実）

- **Archived の定義**: session は `meta.json` の `state = "archived"`、workflow は run archive（`workflow_run_archives.json` / `archive_workspace_workflow_run`）。復帰されうるため、古くても本体は消さない（ログは 30 日 retention の対象）。
- **worktree 削除は波及しない**: worktree（Workspace）の削除は紐づく session / workflow へ cascade しない（`adaptor/gateway/repository/worktree.rs` に session 掃除経路なし、`repository_state/worktree.rs` にも cascade なし）。そのため worktree 削除後、`meta.json` の `worktreePath` がその worktree を指す session 一式・checkpoint・`workspace_state`・workflow データが app data に取り残される。これがルール 1 の主対象であり、容量主因（`sessions/` 約 29 GB）の多くもここに該当すると見込まれる。checkpoint は命名規約から worktree との対応付けを確実に証明できる場合のみ削除対象にし、証明できない場合は保守的に残す。
- **参照切れの判定**: 大きな tool 出力は `tool_outputs/{hash}` に blob として書き出し、message は `ToolOutputRef { id: hash }` で参照する（`tool_output_blob.rs`）。その Session の全 `messages/*.json` から参照 id を集め、`tool_outputs/` のファイル名(id)がその集合に無ければ参照切れ。`attachments/{id}` も同構図。fork（親 `tool_outputs/` を丸ごとコピー）や compaction・メッセージ作り直しで発生する。

### app data の実ディレクトリ構成（GC の対象領域、コード・実データ調査による事実）

app data ルートは Tauri の `app_data_dir()`（＝ `com.releash.app`）で、CLI 経路は環境変数 `RELEASH_DATA_DIR` でも解決される（`src-tauri/src/lib.rs:73`, `src-tauri/src/cli/common.rs:49-71`）。主要サブディレクトリと保存元:

- `sessions/` — agent session。dir 形式 `sessions/{session_id}/`（`meta.json`・`index.json`・`private_context.json`・`events.json`・`messages/{seq}.json`・`attachments/{id}`・`tool_outputs/{id}`）と legacy flat（`{session_id}.json`）・legacy sidecar（`{session_id}.meta.json`）。パス定義は `adaptor/gateway/agent_session/session_storage/layout.rs`。`meta.json` は Workspace への紐付けキー `worktreePath`（絶対パス）と `state`（active/archived 等）を持つ。
- `workflow_runs/` / `workflow_logs/` / `workflow_pending/` / `workflow/` / `workflow_run_archives.json` — workflow run・log・artifact・archive index。
- `lsp/`（`jdtls` / `jdtls-workspaces` / `jdtls.version` / `typescript`）— LSP workspace cache と TypeScript cache。いずれも再生成可能。
- `comments/` / `diff-comments/` / `threads/` — **旧形式**の comment/thread。現行の review comment 系は `review-comments/` に保存される。
- `agent-processes/{session_id}.{backend}.{pid}.json` / `pids/` — process record・pid registry（`pid`・`pgid`・`owner_app_pid` 等を保持）。
- `agent-worktree-checkpoints/` / `agent-worktree-checkpoint-backups/` — Workspace(worktree) 単位の checkpoint。
- `workspace_state/{workspace_id}.json` — Workspace ごとの UI 状態（editor tab / layout）。
- `plans/`・`shell-integration/`・`bin/`・`tls/`・`releash.toml`・`available_models*.json` — 設定・実行資材（本 GC の主対象ではない）。

「参照されている Workspace」は git worktree 一覧（`adaptor/gateway/repository/worktree.rs` の `list_worktrees()`）から解決される worktree に対応する。session 等はその `worktreePath` を通じて Workspace に紐づく。

## スコープ

backend（Rust）に GC を実装し、上記「削除ルール（正典）」に従って app data を削除する。具体的には次を対象とする。

1. **ルール 1 — 削除済み Workspace のログ**: worktree が存在しない Workspace に紐づく app data（session 一式・`workspace_state`・workflow データ等）を削除する。checkpoint は worktree との対応付けを確実に証明できる場合のみ削除し、証明できない場合は残す。
2. **ルール 2 — 使用中 Workspace のログ（Session/Workflow 状態で判断）**:
   - 2-1: 削除済みの Session / Workflow のログを削除する。
   - 2-2: アーカイブ済みの Session / Workflow のログを 30 日超で削除する。
   - 2-3: 使用中の Session / Workflow のログは削除しない。
   - 2-4: 再生成可能なキャッシュ（LSP workspace cache `lsp/`、TypeScript cache `lsp/typescript`）を状態問わず 7 日超で削除する。
3. **旧形式データの全削除**: `comments/` / `diff-comments/` / `threads/`（廃止フォーマット）を状態問わず全て削除する。
4. **参照切れファイルの削除**: 使用中 Session を含め、どの message からも参照されていない `tool_outputs` / `attachments` の blob をファイル単位で削除する（ルール 2-3 の例外）。
5. **stale process/pid record の削除**: `agent-processes/` / `pids/` のうち、記録 pid のプロセスが死んでいるレコードを削除する。
6. **安全ガード**: active session / running workflow に紐づくデータは、上記いずれの経路でも削除しない。
7. **可観測性**: GC の削除件数と回収 byte 数を backend log に出力する。
8. GC ロジックは frontend ではなく Rust/backend 側に置く。
9. **起動トリガ**: GC は backend（アプリ）起動時に 1 回実行する。

## 非スコープ

- Dry-run 機能。
- 手動 cleanup を起動する UI / CLI コマンド。
- 削除前の確認 UI・削除対象のプレビュー表示。
- app data のディレクトリ構造・保存フォーマット自体の変更（既存構造の上で削除する）。
- 旧形式データの新形式への移行（旧形式 `comments`/`diff-comments`/`threads` は移行ではなく削除する）。
- 削除済みデータのゴミ箱・復元機構。
- retention 期間（30 日 / 7 日）のユーザー設定 UI。

## 要求事項

- GC が backend（Rust）で動作し、frontend にロジックを持たないこと。
- GC が backend（アプリ）起動時に実行されること。起動処理を過度にブロックしないこと。
- **ルール 1**: worktree が存在しない Workspace に紐づく app data が削除されること。
- **ルール 2-1**: 削除済みの Session / Workflow のログが削除されること。
- **ルール 2-2**: アーカイブ済みの Session / Workflow のログが 30 日超で削除され、30 日以内は保持されること。
- **ルール 2-3**: 使用中の Session / Workflow のログが削除されないこと。
- **ルール 2-4**: 再生成可能 cache（LSP / TypeScript cache）が状態問わず 7 日超で削除され、7 日以内は保持されること。
- **旧形式削除**: `comments` / `diff-comments` / `threads` が状態問わず全て削除されること。
- **参照切れ削除**: 使用中 Session を含め、どの message からも参照されていない `tool_outputs` / `attachments` の blob が削除され、参照中の blob は残ること。
- **stale process/pid 削除**: 記録 pid のプロセスが死んでいる process record / pid registry が削除され、生存中のものは残ること。
- **安全ガード**: active session / running workflow に紐づくデータが、いずれの削除経路でも削除されないこと。
- GC の削除件数と回収 byte 数が backend log に出力されること。
- 上記を担保するテスト（各ルールの削除／保持、参照切れ判定、retention 境界、active/running 保護）が存在すること。

## 受け入れ基準の概要

- worktree が無い Workspace に紐づく app data が GC 実行後に削除されている（ルール 1）。
- 削除済み Session / Workflow のログが削除される（ルール 2-1）。
- アーカイブ済み Session / Workflow のログが 30 日境界で保持／削除される（ルール 2-2）。
- 使用中 Session / Workflow のログが GC 実行後も残る（ルール 2-3）。
- 再生成可能 cache（LSP / TypeScript）が 7 日境界で保持／削除される（ルール 2-4）。
- 旧形式 `comments` / `diff-comments` / `threads` が状態問わず削除される。
- 使用中 Session 内でも参照切れ `tool_outputs` / `attachments` が削除され、参照中の blob は残る。
- pid が死んだ process / pid record が削除され、生存中は残る。
- active session / running workflow に紐づくデータが GC 実行後も残る（安全ガード）。
- GC 実行後、削除件数と回収 byte 数が backend log に出力される。
- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る。

## 仮定

- 削除ルール（正典）はユーザー確定。中核は復帰可能性で、**古さ（保持日数）は使用中 Session/Workflow のログ本体の削除基準にはしない**（ルール 2-3）。30 日 / 7 日はそれぞれアーカイブ済みログ（2-2）・再生成 cache（2-4）にのみ適用する。
- 「使用中 Workspace」は、既知リポジトリの git worktree 一覧（`list_worktrees()`）に現存する worktree として解決する想定。session は `meta.json` の `worktreePath` がいずれの現存 worktree とも一致しなければ「削除済み Workspace」（ルール 1）とみなす。厳密な判定境界（対象リポジトリ集合の決め方、絶対パスの正規化）は design.md で確定する。
- worktree 削除は session / workflow へ波及しないため、GC 起動時点の worktree 現存集合と `worktreePath` を突き合わせてルール 1 の対象を判定する。
- Session/Workflow の「使用中／アーカイブ済み／削除済み」区別: session は `meta.json` の `state`（`archived` はアーカイブ）、workflow は run archive（`workflow_run_archives.json`）で判定する想定。「削除済み」の表現（物理削除の取り残し／マーカー）と正確な source of truth は design.md で確定する。
- retention（30 日 / 7 日）の起点時刻は対象データの更新時刻（ログは `meta.json` の `updatedAt` またはファイル mtime、cache はファイル mtime）を基準とする想定。正確な基準時刻は design.md で確定する。
- 参照切れ判定は、対象 Session の全 `messages/*.json` から `ToolOutputRef.id`・attachment id を集め、`tool_outputs/` / `attachments/` の各ファイル id がその集合に無いものを参照切れとする想定。
- active session / running workflow の判定は、backend が保持する実行中 registry と `agent-processes/` の pid 生存確認を用いる想定。判定の source of truth は design.md で確定する。
- 旧形式 `comments` / `diff-comments` / `threads` は現行 `review-comments/` に置換済みで、移行せず全削除する（使用中 Workspace 分の旧コメントは失われることをユーザー承知）。
- GC は破壊的操作のため、各ルールに合致すると確実に判定できたデータに限って削除し、判定できないデータは残す（保守的削除）方針とする。削除の原子性（途中終了時の中途半端な状態の許容度）は design.md で確定する。

## Open Questions

なし。
