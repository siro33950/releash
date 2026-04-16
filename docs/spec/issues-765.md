## 要求

**種別**: 改善（機能削除）
**ゴール**: レビューコメントへのAgent返信機能を全面廃止する。スレッド機能自体（ローカルコメントの作成・編集・削除・解決）は維持する
**背景**: Agent返信機能が使われておらず不要。コードベースの簡素化のため削除する

**廃止対象**:
1. MCPツール `post_review_comment` — Agentがレビューコメントを投稿する機能
2. MCPツール `add_thread_entry` — Agentがスレッドにエントリを追加する機能
3. MCPツール `resolve_comment` — Agentがスレッドを解決する機能
4. `thread_ai` / `useThreadAI` — スレッド上でAgentに質問/要約させる機能
5. `reply_to_pr_review_comment` — PR上のレビューコメントに返信する機能（Agent・人間両方）
6. CommentThread UIのAgent関連UI（AIモーダル、Agentボタン等）

**維持する機能**:
- スレッド機能自体（ローカルコメントの作成・編集・削除・解決）
- ThreadStore による永続化
- スレッドの表示・アンカー管理

**影響範囲**:
- Rust: `thread_ai.rs`, `mcp/server.rs`（post_review_comment, add_thread_entry, resolve_comment）, `git_host/mod.rs`（reply_to_pr_review_comment, post_pr_comment）, `review_prompt.rs`
- フロントエンド: `useThreadAI.ts`, `usePrDiff.ts`（replyToThread, postPrComment）, `CommentThread.tsx`（AI関連UI）, `ThreadAIModal.tsx`
- プロンプトテンプレート: `thread_ask.txt`, `thread_ask_pr.txt`, `thread_summarize.txt`, `thread_summarize_pr.txt`

## 振る舞い定義

```gherkin
Feature: Agent返信機能の廃止
  レビューコメントへのAgent返信機能を全面廃止し、
  スレッド機能自体（ローカルコメントの作成・編集・削除・解決）は維持する

  Rule: MCPサーバーにAgent用スレッド操作ツールが存在しない
    Scenario: MCPツール一覧の取得
      Given MCPサーバーが起動している
      When クライアントがツール一覧を取得する
      Then post_review_comment ツールが含まれない
      And add_thread_entry ツールが含まれない
      And resolve_comment ツールが含まれない

  Rule: スレッドの基本操作は維持される
    Scenario: コメントスレッドの作成
      Given ファイルが開かれている
      When ユーザーがコメントスレッドを作成する
      Then スレッドが保存される

    Scenario: スレッドエントリの編集
      Given スレッドにエントリが存在する
      When ユーザーがエントリの内容を編集する
      Then エントリが更新される

    Scenario: スレッドの削除
      Given スレッドが存在する
      When ユーザーがスレッドを削除する
      Then スレッドが削除される

    Scenario: スレッドの解決
      Given 未解決のスレッドが存在する
      When ユーザーがスレッドを解決する
      Then スレッドが解決済みになる

  Rule: スレッドUIにAgent関連の要素が表示されない
    Scenario: コメントスレッドの表示
      Given コメントスレッドが存在する
      When ユーザーがスレッドを表示する
      Then AI質問・要約ボタンが表示されない
      And エントリにAIバッジが表示されない
      And AI実行中インジケーターが表示されない
      And AIログ閲覧ボタンが表示されない
      And Post to PRボタンが表示されない
      And ThreadAIモーダルが存在しない
```

## 実装仕様

**対応方針**: Agent返信機能を全面廃止するために、Rust側のAgent用コマンド・MCPツール・プロンプトテンプレートと、フロントエンド側のAgent関連フック・コンポーネント・UIを削除する。スレッド基本機能（作成・編集・削除・解決）とThreadStoreは維持する。

**対象コンポーネント**:

### Rust（完全削除）
| ファイル | 行数 | 内容 |
|---------|------|------|
| `src-tauri/src/thread_ai.rs` | 346行 | AIプロンプト生成コマンド（全体削除） |
| `src-tauri/resources/prompts/thread_ask.txt` | — | テンプレート（全体削除） |
| `src-tauri/resources/prompts/thread_ask_pr.txt` | — | テンプレート（全体削除） |
| `src-tauri/resources/prompts/thread_summarize.txt` | — | テンプレート（全体削除） |
| `src-tauri/resources/prompts/thread_summarize_pr.txt` | — | テンプレート（全体削除） |

### Rust（部分削除）
| ファイル | 削除対象 |
|---------|---------|
| `src-tauri/src/mcp/server.rs` | `post_review_comment()`, `add_thread_entry()`, `resolve_comment()` の3メソッド + MCPツール定義 |
| `src-tauri/src/git_host/mod.rs` | `reply_to_pr_review_comment()`, `reply_to_pr_review_comment_inner()`, `post_pr_comment()`, `post_pr_comment_inner()` の4関数 |
| `src-tauri/src/lib.rs` | `thread_ai` モジュール宣言、`build_thread_ai_prompt`/`build_thread_summarize_prompt` コマンド登録、`reply_to_pr_review_comment`/`post_pr_comment` コマンド登録 |

### フロントエンド（完全削除）
| ファイル | 行数 | 内容 |
|---------|------|------|
| `src/hooks/useThreadAI.ts` | 335行 | AIタスク管理フック（全体削除） |
| `src/components/panels/ThreadAIModal.tsx` | 195行 | AIログ表示モーダル（全体削除） |
| `src/lib/formatImplementPrompt.ts` | — | Agent実装プロンプト生成（全体削除） |

### フロントエンド（部分削除）
| ファイル | 削除対象 |
|---------|---------|
| `src/hooks/usePrDiff.ts` | `replyToThread()`、`postPrComment()` 関数と戻り値型のプロパティ |
| `src/components/panels/CommentThread.tsx` | AI関連Props（`aiRunningThreadIds`, `aiTaskThreadIds`, `onOpenThreadAIModal`）、"AI is thinking..."ボタン、"View AI Log"ボタン、`isRunning`変数、"Post to PR"ボタンセクション |
| `src/screens/useWorktreeState.tsx` | `useThreadAI` インポート・初期化・threadAI関連状態、`handlePostToPr`/`handlePostToPrConfirm`、`summarizeForPr`呼び出し |
| `src/screens/useWorktreeComments.ts` | `handleImplementThread`、`formatImplementPrompt`インポート |
| `src/screens/MainLayout.tsx` | `ThreadAIModal` インポート・コンポーネント |
| `src/components/panels/EditorTabContent.tsx` | AI関連Props受け渡し |
| `src/contexts/EditorContext.tsx` | `onPostToPr` 型定義 |

### 維持するもの（変更なし）
- `thread_store.rs` — スレッド永続化・管理（全メソッド維持）
- `review_prompt.rs` — レビュータスク生成（Agent返信と無関係）
- `protocol/thread.rs` — Thread/ThreadEntry型定義
- `ws_server/handlers.rs` — WebSocket経由のThread操作

**検討した代替案**:
- Agent返信機能のコードを残してUI非表示のみ → 却下理由: ゴールが「コードベースの簡素化」のため、不要コード残存は目的に反する

**リスク**:
- CommentThread.tsxの部分削除でUI崩れ → 緩和策: "Post to PR"セクション削除後のフッターレイアウトを確認
- MCPツール削除でAgentが既存ツールを呼ぼうとする → 緩和策: MCPツール一覧から消えるため、呼び出し自体が発生しない

**影響するテスト**:
- Rust: `thread_ai.rs`内テスト削除、`mcp/server.rs`内の該当テスト削除
- フロントエンド: `useThreadAI.ts`、`ThreadAIModal.tsx`のテストファイル削除（存在する場合）、`CommentThread.tsx`のテストからAI関連アサーション削除
- 既存のスレッド基本操作テストは変更不要

---

## 追加スコープ: AgentChat以外のAI対話・回答機能を全廃止

初回レビューで「Review実行機能・Thread AI設定・isAi識別フィールド・関連CSS」がAgent返信の上位概念であるAI対話機能として残存していたため、ユーザー指示により**AgentChat以外のAI対話・回答仕組みは一切残さない**方針で追加削除する。

### 追加削除対象

**Rust**:
- `src-tauri/src/review/` ディレクトリ全体（`mod.rs`, `orchestrator.rs`, `commands.rs`）
- `src-tauri/src/review_prompt.rs` 全体
- `src-tauri/resources/prompts/review.txt`, `review_file.txt`
- `src-tauri/src/lib.rs`: `mod review;`, `mod review_prompt;` とレビュー実行系Tauriコマンド登録（`start_review`, `cancel_review`, `get_review_status`, `reset_review`, `get_review_prompt`, `get_per_file_review_tasks`）
- `src-tauri/src/git/review.rs` の Review専用関数 `get_review_diff`、および `get_review_diff_summary` コマンドと関連テスト
- `src-tauri/src/mcp/server.rs` の `review_diff` ツール、`get_review_comments` ツール（Deprecated）
- `src-tauri/src/protocol/thread.rs`, `comment.rs`, `mod.rs` の `is_ai` フィールド
- `src-tauri/src/thread_store.rs` の `is_ai`/`target="review"` 関連判定
- `src-tauri/src/ws_server/handlers.rs` の `is_ai` 受け渡し
- `src-tauri/src/git_host/mod.rs` の `is_ai: false` 記述

**フロントエンド**:
- `src/hooks/useReviewExecution.ts` + test
- `src/hooks/useReviewDiffFiles.ts` + test
- `src/components/panels/ReviewModal.tsx`
- `src/components/panels/RightSidebarBottom.tsx` の Review タブ・AI Review起動ボタン
- `src/types/settings.ts` の `AgentConfig.reviewCommand/threadCommand`、`AppSettings.reviewAgent/reviewModel/customReviewCommand/reviewConcurrency`、`buildReviewCommand/buildReviewCommandTemplate/buildThreadCommand`
- `src/components/panels/SettingsModal.tsx` の Review 設定セクション
- `src/types/thread.ts` の `ThreadEntry.isAi`、`ThreadOrigin` から `"ai-review"`
- `src/types/protocol.ts` の `CreateThread.is_ai`、`AddThreadEntry.is_ai`、`CommentItem.target` から `"ai"/"review"`、author.type から `"ai"`
- `src/types/comment.ts` の `"ai"/"review"` 対象
- `src/hooks/useThreads.ts` の `isAi` パラメータ
- `src/remote/components/RemoteThreadList.tsx`, `src/remote/hooks/useRemoteAppActions.ts`, `src/lib/prCommentMapping.ts` のAI関連参照
- 各種テストファイルからの `isAi` 参照

**CSS**:
- `src/index.css`: `.comment-thread-ai-thinking`, `.comment-thread-ai-log`

**統合テスト**:
- `tests/review-panel.spec.ts`, `tests/screenshots/review-panel.screenshot.ts`

### 置換・リネーム（機能保持）

Source Control パネルの "Branch Base" モード表示（AI レビューとは無関係のGit diff機能）は維持するため、以下のリネームを行う:

| 変更前 | 変更後 |
|--------|--------|
| `src-tauri/src/git/review.rs` | `src-tauri/src/git/branch_diff.rs` |
| `get_review_diff_summary` Tauriコマンド | `get_branch_diff_summary` |
| `src/hooks/useReviewDiffFiles.ts` | `src/hooks/useBranchDiffFiles.ts` |
| 型 `ReviewChangedFile` | `BranchDiffChangedFile` |
| `SourceControlPanel.tsx` の `mapReviewStatus` | `mapBranchDiffStatus` |

`get_hunk_ranges` / `is_line_in_hunk_ranges` は MCP `check_diagnostics` ツールの `diff_context` フィルタから利用されるため、名称変更のみで機能は維持する。

### 維持する機能

- **AgentChat系全般**: `useAgentChat`, `agent_sdk.rs`, `AgentChatPanel/`, `batch_spawn_agent_ptys`
- **AgentConfig の共通フィールド**: `command`, `bypassFlag`, `label`, `modelFlag`（AgentChat/batch_spawnで使用）
- **AgentChat用MCPツール**: `list_threads`, `get_thread`, `read_file`, `check_diagnostics`, `get_file_symbols`, `explore_symbol`, `worktrees_list`
- **スレッド基本機能**: 作成・編集・削除・解決、ThreadStore永続化

### 後方互換性

既存の永続化JSON に `is_ai: true` のエントリが存在する可能性があるが、deserialize時は `#[serde(default)]` で無視される前提でロード時migrationは追加しない。
