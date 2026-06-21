# Requirements

## Type

新機能。Workspace サイドバーを `Repository -> Worktree -> Session / Workflow -> WorkflowSession` のツリー構造へ再編する。

関連: #1220 / #1023

## Goal

左サイドバーで worktree 配下の作業単位を俯瞰・選択できるようにする。

現在の Workspace サイドバーは `Repository -> Worktree` までしか表示せず、Session の切替は中央 `AgentChatPanel` 上部のタブバー、Workflow の確認は中央 `WorkflowView` に分かれている。複数 Session / Workflow を扱うと、作業単位の全体像と切替先が分散する。

本 Issue では、左サイドバーを次のツリーへ再編し、選択ナビゲーションを左ツリーへ一本化する。

```text
Repository
  Worktree
    Session
    Workflow
      WorkflowSession
```

最終状態では、中央 `AgentChatPanel` 上部の Session タブバーを廃止し、中央は左ツリーで選択された1件のみを表示する。

## Current UI Decision

Issue 本文のゴールに対して、対話で確定した Workspace UI の理想状態は以下。

- 既存の `Workspaces` ヘッダーは維持する。
  - ツリー一本化に伴い `Group` / `Filter` と Status グループ表示は廃止する。
  - ヘッダーの操作は `Add Worktree` のみを残す。
  - `プロジェクト / all / ...` のような新規トップ行は作らない。
- Worktree 行:
  - 左側は `Home` または worktree アイコン、Worktree 名、hover 時だけ Worktree 名の右に開閉 chevron。
  - main worktree は `Home`、それ以外は worktree らしいアイコンを表示する。
  - 右側に menu ボタンと新規 Session ボタンを置く。
  - Worktree 行クリックは展開/折りたたみのみ。中央の選択対象は変更しない。
  - Worktree 行には folder icon、色付き dot、状態テキスト、相対時刻を出さない。
- Session 行:
  - Worktree で使っていた agent state icon を Session 側へ移す。
  - 状態は icon で示し、`Open` / `Closed` のような状態テキストは出さない。
  - hover 時に右端へ閉じるボタンを出す。
  - クリックすると worktree を選択し、中央の選択対象を `agentSession` にして該当 Session を表示する。
- Workflow 行:
  - Workflow 親は展開/折りたたみのみ。クリックで中央を Workflow view に切り替えない。
  - hover 時だけ Workflow 名の右に開閉 chevron を出す。
  - Workflow 親には相対時刻や `Open` / `Closed` を出さない。
- WorkflowSession 行:
  - Workflow 配下の Session として表示する。
  - クリックすると worktree を選択し、中央の選択対象を `workflowRun` にして該当 run / session を表示する。
- インデント:
  - Worktree 直下の Session / Workflow は Worktree 名の開始位置から始める。
  - Workflow 配下の WorkflowSession は Workflow 名の開始位置から始める。
  - Session と Workflow の同階層インデントは一致させる。
- 並び順:
  - Worktree 配下は固定で `Session 優先 -> 名前順`。
  - 並び替え UI は置かない。
- 表示省略:
  - `もっと表示する` は置かず、取得済みノードを全件表示する。
- Worktree menu:
  - クリックで menu を開く。
  - `SessionHistory` と `WorkflowHistory` は hover で子 menu を開く。
  - `PRリンク` と `削除` を同じ menu 内に置く。
- Worktree create menu:
  - `NewSession` と `NewWorkflow` を表示する。
  - `NewWorkflow` は hover で workflow 一覧の子 menu を開き、選択後に task 入力 dialog から `start_workflow` を起動する。

## Scope

- Workspace サイドバーのツリー UI 実装。
- Session / Workflow / WorkflowSession を返す Rust 側 Tauri command または既存 command の整理。
- 左ツリー選択から中央表示を駆動する配線。
- 独立した `centerMode` 状態を廃止し、中央表示を `CenterSelection.kind` から導出する配線。
- 中央 `AgentChatPanel` の Session タブバー廃止。
- 廃止タブバーが持っていた機能の移設。
  - 新規 Session 開始。
  - Session 選択。
  - Session close。
  - Session restore / archive を含む History 操作。
- Worktree create menu からの新規 Workflow run 起動。
- Workflow run と WorkflowSession の選択。
- 仮 UI データと不要化したタブバー関連コードの削除。

## Non-goals

- Workflow engine の新しい実行モデル追加。
- Workflow template marketplace / remote UI / Web UI への移植。
- Source Control / Review / Editor / Terminal の再設計。
- Worktree 作成・削除・PRリンクなど既存 worktree 管理機能の意味変更。
- 左サイドバー外の visual redesign。

## Requirements

- 左サイドバーで Repository -> Worktree -> Session / Workflow -> WorkflowSession を表示できること。
- Worktree 行の UI は Current UI Decision に従うこと。
- Worktree 行クリックでは `CenterSelection` を変更しないこと。
- Session 行クリックで該当 worktree を開き、`CenterSelection.kind = "agentSession"` として該当 Session を表示すること。
- Workflow 親クリックでは `CenterSelection` を変更せず、展開/折りたたみのみ行うこと。
- WorkflowSession 行クリックで該当 worktree を開き、`CenterSelection.kind = "workflowRun"` として該当 run / session を表示すること。
- Worktree menu から SessionHistory / WorkflowHistory の hover 子 menu、PRリンク、削除に到達できること。
- Session hover action から Session close ができること。
- Worktree の新規 Session ボタンから新規 Session を開始できること。
- Worktree create menu の `NewWorkflow` サブメニューから workflow を選択し、task 入力 dialog の Start で `start_workflow` を起動できること。
- 中央 `AgentChatPanel` の Session タブバーは削除されていること。
- Workflow step session は自由対話 Session と同格の agent tab として中央に並ばないこと。
- Session を持たない終端 Workflow run は `archive_reason = auto_no_sessions` で自動 archive され、ツリーから除外されるが WorkflowHistory には `archived_at` 付きで残ること。
- それ以外の終端 Session / Workflow は明示削除または archive までツリー・history から勝手に消えないこと。
- Session / Workflow / WorkflowSession の親子関係、状態、並び順の決定は Rust 側で行うこと。フロントは表示、入力受付、Tauri command 呼び出しに徹すること。
- 中央の Agent / Workflow 表示は独立した toggle や mode state ではなく、現在の `CenterSelection` から導出すること。
- `ViewToolbar` の Agent / Workflow 切替は最終状態では表示しないこと。
- フロントの仮データ生成（branch 名から Session / Workflow を作る処理）は実装完了時に削除されていること。

## Data Sources

- Worktree: 既存 `useWorktreeList` / Rust worktree command が返す `WorktreeBranch` 相当。
- Session: 既存 `list_sessions(worktreePath)` が返す `SessionSummary`。
  - 直下 Session 判定: `workflowStepSession !== true`。
- Workflow: 既存 `list_workflow_runs(worktreePath)` が返す `WorkflowRunSummary`。
- WorkflowSession:
  - `get_workflow_run_state(worktreePath, runId)` または同等の Rust projection から得る。
  - `currentSessionId`、`stepHistory[].sessionId`、必要に応じて parallel child / active parallel step の session id を Rust 側で集約する。
  - `workflowStepSession === true` の Session と run の session id 集合を Rust 側で対応付ける。

## Acceptance Criteria

- Workspace サイドバーで4階層ツリーを展開・折りたたみできる。
- Worktree / Session / Workflow / WorkflowSession の UI が Current UI Decision と一致する。
- Session 選択で `agentSession` selection の該当 Session が表示される。
- WorkflowSession 選択で `workflowRun` selection の該当 run / session が表示される。
- Worktree 親と Workflow 親は `CenterSelection` を変更しない。
- Worktree menu から SessionHistory / WorkflowHistory / PRリンク / 削除に到達できる。
- Session hover close が既存 close semantics を壊さず動作する。
- 中央 AgentChat の Session タブバーが表示されない。
- `centerMode` を独立 state として保持する実装が残っていない。
- `pnpm lint` / `pnpm test` / `pnpm build` が成功する。
- Rust 変更を含む場合、`cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が成功する。

## Open Questions

- SessionHistory / WorkflowHistory の中身は、既存 History popover をどの粒度で再利用するか。
- WorkflowHistory の削除 / archive semantics を Session と同じ menu に置くか、Workflow 専用 action として分けるか。
- WorkflowSession hover close を通常 Session と同じ見た目にするか、Workflow 配下では close を出さないか。
- workflow run の子 Session 名は `stepName`、Session title、first message のどれを優先表示するか。
