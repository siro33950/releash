## 要求

**種別**: 改善
**ゴール**: Agent / Editor のトップタブ、右パネルのアイコンタブ（FileTree / Search / PR / SourceControl）、コミットUIを削除し、中央をAgentChat常時表示、右パネルをReviewパネル専用の領域にする。UIの削除に伴い、アクセスされなくなる機能そのもの（バックエンドのロジック・コマンド含む）も削除する。WebSocketサーバー・リモートアプリからも同様にSourceControl・Diff・Commit関連を廃止する。ただし、#785（Reviewパネル上部）・#786（Reviewパネル下部）で流用可能なコンポーネント・コマンドは削除せず保持する
**背景**: AgentChatが主要な操作手段になったため、使われなくなったエディタ・SourceControl・FileTree・Search・PR・コミットUIを廃止し、画面をシンプルにする
**影響範囲**:
- 削除対象（UI + 背後の機能）:
  - Agent / Editor のトップタブ切り替えUI + Editorタブで使われていた機能
  - 右パネル上部のアイコンタブUI + SourceControl / FileTree / Search / PR の機能
  - コミットUI（Commit summary / Description / Commit / Push ボタン）+ コミット関連の機能
  - リモートアプリのChanges / Diffタブ + WebSocketサーバーのGit操作・ファイルコンテンツハンドラー
- 保持:
  - #785で流用するDiff表示コンポーネント・ユーティリティ・Rustコマンド
  - #786で流用するTerminal・Comments/Threads
- 結果:
  - 中央は常にAgentChat
  - 右パネルはReviewパネル専用の領域になる（Reviewパネル自体の実装は#785・#786）

## 振る舞い定義

```gherkin
Feature: UI簡素化 - 旧UI廃止・AgentChat常時表示
  AgentChatが主要な操作手段になったため、使われなくなったUI要素と背後の機能を削除し、
  中央をAgentChat常時表示にする。Reviewパネルの実装は#785・#786で行う

  Rule: 中央パネルはAgentChat専用である
    Scenario: アプリケーション起動時に中央パネルにAgentChatが表示される
      Given ワークツリーが開かれている
      When ユーザーが画面を表示する
      Then 中央パネルにAgentChatが表示されている

    Scenario: Agent/Editorのタブ切り替えUIが存在しない
      Given ワークツリーが開かれている
      When ユーザーが画面を表示する
      Then Agent/Editorを切り替えるタブUIは存在しない

  Rule: Editorタブとその関連機能は廃止されている
    Scenario: エディタのタブ管理UIが存在しない
      Given ワークツリーが開かれている
      When ユーザーが画面を表示する
      Then ファイルを開くタブバーは存在しない
      And エディタのコンテンツ表示領域は存在しない

  Rule: 右パネル上部のアイコンタブUIは廃止されている
    Scenario: 右パネル上部のアイコンタブ切り替えUIが存在しない
      Given ワークツリーが開かれている
      When ユーザーが画面を表示する
      Then FileTree/Search/PR/SourceControl/Symbolsを切り替えるアイコンタブUIは存在しない

  Rule: SourceControl・FileTree・Search・PR・Symbols機能は廃止されている
    Scenario: SourceControlパネルが存在しない
      Given ワークツリーが開かれている
      When ユーザーが画面を表示する
      Then SourceControlパネルは存在しない

    Scenario: コミットUIが存在しない
      Given ワークツリーが開かれている
      When ユーザーが画面を表示する
      Then コミットサマリ入力・説明入力・Commitボタン・Pushボタンは存在しない

    Scenario: FileTreeパネルが存在しない
      Given ワークツリーが開かれている
      When ユーザーが画面を表示する
      Then FileTreeパネルは存在しない

    Scenario: Searchパネルが存在しない
      Given ワークツリーが開かれている
      When ユーザーが画面を表示する
      Then Searchパネルは存在しない

    Scenario: PullRequestパネルが存在しない
      Given ワークツリーが開かれている
      When ユーザーが画面を表示する
      Then PullRequestパネルは存在しない

    Scenario: SymbolOutlineパネルが存在しない
      Given ワークツリーが開かれている
      When ユーザーが画面を表示する
      Then SymbolOutlineパネルは存在しない

  Rule: 右パネル下部のTerminal・Commentsは維持される
    Scenario: 右パネル下部にTerminalとCommentsが表示される
      Given ワークツリーが開かれている
      When ユーザーが画面を表示する
      Then 右パネル下部にTerminalタブとCommentsタブが表示されている

  Rule: リモートアプリのSourceControl・Diff・Commitは廃止されている
    Scenario: リモートアプリのChangesタブが存在しない
      Given リモートアプリでワークツリーが選択されている
      When ユーザーがタブバーを表示する
      Then Changesタブは存在しない

    Scenario: リモートアプリのDiffタブが存在しない
      Given リモートアプリでワークツリーが選択されている
      When ユーザーがタブバーを表示する
      Then Diffタブは存在しない

    Scenario: リモートアプリのTerminal・Comments・Threadsタブは維持される
      Given リモートアプリでワークツリーが選択されている
      When ユーザーがタブバーを表示する
      Then Terminalタブ・Commentsタブ・Threadsタブが表示されている
```

## 実装仕様

**対応方針**: 旧UI（Editor・SourceControl・FileTree・Search・PR・Symbols・CommitUI）を廃止し、中央をAgentChat常時表示にする。WebSocketサーバー・リモートアプリからも同様にGit操作・Diff関連を廃止する。ただし、#785（Reviewパネル上部: 差分ファイル一覧 + 差分表示）・#786（Reviewパネル下部: コメント一覧 + ターミナル）で流用可能なコンポーネント・コマンドは保持する

**対象コンポーネント**:

### デスクトップ フロントエンド — ファイル削除

| カテゴリ | ファイル | 理由 |
|---------|---------|------|
| UIコンポーネント | `EditorTabContent.tsx` | Editor廃止 |
| | `RightSidebarTop.tsx` | アイコンタブUI廃止 |
| | `SourceControlPanel.tsx` | SourceControl廃止 |
| | `SourceControlCommitForm.tsx` | コミットUI廃止 |
| | `SourceControlContextMenu.tsx` | SourceControl廃止 |
| | `SearchPanel.tsx` | Search廃止 |
| | `PullRequestPanel.tsx` | PR廃止 |
| | `SymbolOutlinePanel.tsx` | Symbols廃止 |
| | `SidebarPanel.tsx` | FileTree廃止 |
| | `FileTree.tsx` | FileTree廃止 |
| | `FileTreeContextMenu.tsx` | FileTree廃止 |
| | `EmptyState.tsx` | Editor空状態 |
| | `MonacoDiffViewer.tsx` | #785はMonaco Diff不使用（#785要件） |
| Hooks | `useEditorLayout.ts` | エディタタブ管理 |
| | `useFileContents.ts` | ファイルコンテンツ管理 |
| | `useFileTree.ts` | FileTree |
| | `useSearch.ts` | Search |
| | `useAheadBehind.ts` | SourceControl専用 |
| | `useBranchPr.ts` | PRパネル用 |
| | `usePrDetail.ts` | PRパネル用 |
| | `usePrDiff.ts` | PRパネル用 |
| | `useGitLog.ts` | 未使用 |
| | `useFileOperations.ts` | SidebarPanel専用 |
| | `useMonacoDiffEditor.ts` | Monaco Diff専用 |
| ユーティリティ | `monaco-definition-provider.ts` | Editor専用（LSP定義ジャンプ） |
| Contexts | `GitStatusContext.tsx` | SourceControl/SidebarPanel専用 |
| テスト | 上記ファイルに対応する全テストファイル | 対象コード削除 |

### デスクトップ フロントエンド — 保持（#785/#786で流用）

| カテゴリ | ファイル | #785/#786での用途 |
|---------|---------|-----------------|
| Diff表示 | `DiffViewerSection.tsx` | 差分表示ルーティング |
| | `MarkdownDiffViewer.tsx` | カスタムdiff表示 |
| | `ImageDiffViewer.tsx` | 画像diff表示 |
| | `DiffToolbar.tsx` | diff操作ツールバー |
| Diffユーティリティ | `computeHunks.ts` | diff計算 |
| | `generatePatch.ts` | パッチ生成 |
| | `markdownDiff.ts` | Markdown diff計算 |
| Hooks | `useGitOriginalContent.ts` | diffベースコンテンツ取得 |
| | `useHunks.ts` | hunk計算・ナビゲーション |
| | `useDiffOperations.ts` | stage/unstage操作 |
| | `useImageDiff.ts` | 画像diff |
| | `useBranchDiffFiles.ts` | ブランチ差分ファイル一覧 |
| Contexts | `EditorContext.tsx` | diffBase, diffMode, threads関連（#785で再構成） |

### デスクトップ フロントエンド — 修正

| ファイル | 変更内容 |
|---------|---------|
| `MainLayout.tsx` | Agent/Editorタブ切り替えUI削除→AgentChat常時表示。RightSidebarTop削除。EditorContext.Providerは#785で再構成するため一旦削除。GitStatusProvider削除。DraggableTabs等のEditor用import削除 |
| `useWorktreeState.tsx` | 削除対象hooks（useEditorLayout, useFileContents, useBranchPr等）の使用箇所削除。editorLayout関連state削除。monaco-definition-provider削除。LSP関連（useLsp）削除 |

### リモートアプリ — ファイル削除

| ファイル | 理由 |
|---------|------|
| `RemoteSourceControl.tsx` | Changesタブ廃止 |
| `DiffTabContent.tsx` | Diffタブ廃止 |
| `RemoteDiffPanel.tsx` | Diffタブ廃止 |
| `DiffRenderer.tsx` | Diffタブ廃止 |
| `RemoteCommentInput.tsx` | DiffPanel内コメント入力（DiffPanel削除で不要） |
| `useRemoteGitStatus.ts` | SourceControl廃止 |
| `useRemoteGitActions.ts` | SourceControl/Commit廃止 |
| `useRemoteFileContent.ts` | Diff廃止 |

### リモートアプリ — 修正

| ファイル | 変更内容 |
|---------|---------|
| `RemoteApp.tsx` | 削除対象hooks/コンポーネント除去。タブをTerminal/Comments/Threadsのみに |
| `TabBar.tsx` | Changes/Diffタブ削除 |
| `useRemoteAppActions.ts` | Git操作関連ハンドラー削除（handleStageAll, handleUnstageAll, handleSelectFile, handleDiffBaseChange, handleNavigateToDiff等） |
| `useRemoteNavigation.ts` | `selectedPath`, `diffBase`等のGit/Diff関連state削除 |

### WebSocketサーバー — ハンドラー削除

| ハンドラー | 理由 |
|----------|------|
| Git status request/sync | SourceControl廃止 |
| Git stage/unstage/stage_hunk | SourceControl廃止 |
| Git commit/push | Commit UI廃止 |
| File content request/response | Diff廃止 |

関連: `routing.rs`から削除対象メッセージのルーティング除去。`handle_worktree_select_request`からGitStatusSyncブロードキャスト部分を削除

### WebSocketサーバー — 保持

| ハンドラー | 理由 |
|----------|------|
| PTY系全ハンドラー | Terminal保持 |
| Comment系全ハンドラー | Comments保持 |
| Thread系全ハンドラー | Threads保持 |
| Worktree list/select | ワークツリー選択保持 |
| Branch info | ブランチ情報表示保持 |

### プロトコル — メッセージ型削除

| メッセージ | 理由 |
|----------|------|
| `GitStatusRequest`, `GitStatusSync` | SourceControl廃止 |
| `GitStage`, `GitUnstage`, `GitStageHunk`, `GitStageResult` | SourceControl廃止 |
| `GitCommitRequest`, `GitCommitResult` | Commit UI廃止 |
| `GitPushRequest`, `GitPushResult` | Commit UI廃止 |
| `FileContentRequest`, `FileContentResponse`, `FileChange` | Diff廃止 |

### バックエンド（Rust） — コマンド削除

| コマンド | 理由 |
|---------|------|
| `search_files` | SearchPanel専用 |
| `list_document_symbols` | SymbolOutlinePanel専用 |
| `find_definition` | Monaco Editor専用 |
| `find_references` | Monaco Editor専用 |
| `get_pr_detail` | PRパネル専用 |
| `get_pr_files` | PRパネル専用 |
| `get_pr_review_comments` | PRパネル専用（内部関数はWebSocketが使用→保持） |
| `get_binary_file_at_ref` | Editor専用 |
| `get_current_branch_ahead_behind` | SourceControl専用 |
| `git_commit` | コミットUI廃止・WSハンドラー廃止 |
| `git_push` | コミットUI廃止・WSハンドラー廃止 |
| `git_discard` | SourceControl専用 |
| LSP系全コマンド | Editor廃止でLSP不要（`spawn_lsp`, `lsp_send`, `shutdown_lsp`, `kill_lsp`等） |

### バックエンド（Rust） — 保持（#785で流用）

| コマンド | #785での用途 |
|---------|-------------|
| `get_git_status` | 差分ファイル一覧 |
| `get_staged_content` | staged差分表示 |
| `get_file_at_branch_base` | branch base差分表示 |
| `get_binary_file_at_branch_base` | 画像diff |
| `get_binary_staged_content` | 画像diff |
| `get_branch_diff_summary` | ブランチ差分ファイル一覧 |
| `git_stage`, `git_unstage`, `git_stage_hunk`, `git_unstage_hunk` | Reviewパネルからのstage/unstage操作 |
| `get_branch_base`, `set_branch_base`, `list_branches` | 差分ベース選択・BranchSelector |
| `get_current_branch`, `list_branches_with_status` | WSハンドラー（branch info/worktree）保持 |
| Thread/Comment系全コマンド | #786 コメント一覧 |
| PTY系全コマンド | #786 ターミナル |
| Agent SDK系全コマンド | AgentChat保持 |

### バックエンド（Rust） — モジュール削除

| モジュール | 理由 |
|-----------|------|
| `search.rs` | 全コマンドが削除対象 |

### バックエンド（Rust） — モジュール保持

| モジュール | 理由 |
|-----------|------|
| `git/branch_diff.rs` | #785で流用（`get_branch_diff_summary`） |

**影響するテスト**:
- フロントエンド: 削除対象コンポーネント・hooksのテストファイルを全削除。MainLayout関連テストがあれば修正。保持対象のdiff関連テストはそのまま維持
- Rust: `search.rs`内テスト削除、`commands.rs`からコマンド削除に伴うテスト修正、`lib.rs`のinvoke_handler修正
- リモートアプリ: 削除対象コンポーネントのテストを削除
