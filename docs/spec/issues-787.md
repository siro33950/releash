## 要求

**種別**: 新機能
**ゴール**: Diff表示内に行インラインコメントとファイルコメントを付けられる機能を実装し、コメントをローカルファイルに保存してAgentに明示的に送信できるフローを作る。既存のComment/Thread機能は完全に破棄し0から再構築する。
**背景**: ローカルでDiffにコメントをつけ、Agentに渡すワークフローを実現したい。既存のComment/Thread機能は破棄して再設計する。
**制約**:
- コメントデータはローカルファイルに永続化する
- AgentへのコメントはRust内で完結する（ユーザーの明示的な送信操作がトリガー）
- 既存のComment/Thread関連コードはこのIssueで削除する
- 前提: #785 Reviewパネル上部の実装が完了していること

**コメントの種類**:
- 行インラインコメント: diffの特定行にコメントを付ける（GitHub PRレビューと同様）。行をクリック/ホバーでコメント追加UIを表示。コメントはdiffの行間にインラインで表示。
- ファイルコメント: ファイルのdiffヘッダ部分にコメントを付ける。ファイル全体に関するコメント用。

## 振る舞い定義

```gherkin
Feature: Diffインラインコメント
  Diff表示内に行コメントとファイルコメントを付け、ローカル保存してAgentに送信できる

  # --- 行インラインコメント ---

  Rule: 行インラインコメントの作成
    Scenario: diffの特定行にコメントを追加する
      Given diffが表示されている
      When ユーザーが特定の行にコメントを追加する
      Then その行に紐づくコメントが作成される
      And コメントがローカルファイルに永続化される

    Scenario: diffの複数行範囲にコメントを追加する
      Given diffが表示されている
      When ユーザーが複数行を範囲選択してコメントを追加する
      Then その行範囲に紐づくコメントが作成される
      And コメントがローカルファイルに永続化される

  Rule: 行インラインコメントの表示
    Scenario: コメント付きの行がdiffに表示される
      Given 行インラインコメントが存在する
      When ユーザーがそのファイルのdiffを表示する
      Then コメントがdiffの行間にインラインで表示される

    Scenario: 複数行範囲コメントがdiffに表示される
      Given 複数行範囲に紐づくコメントが存在する
      When ユーザーがそのファイルのdiffを表示する
      Then コメントが範囲の末尾行の後にインラインで表示される
      And コメントの対象行範囲が視覚的に示される

  # --- ファイルコメント ---

  Rule: ファイルコメントの作成
    Scenario: ファイル全体に対するコメントを追加する
      Given diffが表示されている
      When ユーザーがファイルのdiffヘッダにコメントを追加する
      Then そのファイルに紐づくコメントが作成される
      And コメントがローカルファイルに永続化される

  Rule: ファイルコメントの表示
    Scenario: ファイルコメントがdiffヘッダに表示される
      Given ファイルコメントが存在する
      When ユーザーがそのファイルのdiffを表示する
      Then コメントがdiffヘッダ部分に表示される

  # --- コメント操作 ---

  Rule: コメントの編集
    Scenario: 既存コメントの内容を編集する
      Given コメントが存在する
      When ユーザーがコメントの内容を編集する
      Then コメントの内容が更新される
      And 変更がローカルファイルに永続化される

  Rule: コメントの削除
    Scenario: コメントを削除する
      Given コメントが存在する
      When ユーザーがコメントを削除する
      Then コメントが除去される
      And 変更がローカルファイルに永続化される

  # --- Agentへの送信 ---

  Rule: コメントの個別Agent送信
    Scenario: 特定のコメントをAgentに送信する
      Given 未送信のコメントが存在する
      When ユーザーが特定のコメントを選んでAgentへの送信を実行する
      Then コメント内容が@ファイル指定としてAgentのチャットに渡される
      And 送信されたコメントのステータスが「送信済み」になる

  Rule: コメントの一括Agent送信
    Scenario: 未送信コメントを一括でAgentに送信する
      Given 複数の未送信コメントが存在する
      When ユーザーが一括送信を実行する
      Then 全ての未送信コメント内容が@ファイル指定としてAgentのチャットに渡される
      And 送信された全コメントのステータスが「送信済み」になる

    Scenario: 未送信コメントがない状態で一括送信を試みる
      Given 全てのコメントが送信済みである
      When ユーザーが一括送信を実行する
      Then 送信対象がないことが示される

  # --- 永続化 ---

  Rule: コメントのローカル永続化
    Scenario: アプリ再起動後もコメントが復元される
      Given コメントが保存されている
      When アプリを再起動してdiffを表示する
      Then 保存されたコメントが復元されて表示される

  # --- 既存コード削除 ---

  Rule: 既存Comment/Thread機能の破棄
    Scenario: 既存のComment/Thread関連コードが削除されている
      Given 新しいコメント機能が実装されている
      When ビルドを実行する
      Then 旧Comment/Thread関連のコード・型・コマンドが存在しない
```

## 実装仕様

**対応方針**: 振る舞い定義を実現するために、既存のComment/Thread機能を全削除し、Rust側に新しい`DiffCommentStore`を実装、フロントエンドはShikiDiffViewer内にインラインコメントUIを組み込む方式で対応する。

**対象コンポーネント**:

### Rust（バックエンド）— 新規

- `src-tauri/src/diff_comment_store.rs`: DiffComment型定義 + CRUD + JSON永続化 + Tauriコマンド。`RwLock<HashMap>` + worktree単位JSON永続化パターン
- `src-tauri/src/diff_comment_sender.rs`: コメントのAgent送信を完結させるTauriコマンド。フォーマット変換 → Agent送信 → ステータス更新を1コマンドで実行

### Rust（バックエンド）— 削除

- `src-tauri/src/comment_store.rs`: 旧CommentStore（全削除）
- `src-tauri/src/thread_store.rs`: 旧ThreadStore（全削除）
- `src-tauri/src/protocol/comment.rs`: 旧Comment型（全削除）
- `src-tauri/src/protocol/thread.rs`: 旧Thread型（全削除）

### Rust（バックエンド）— 修正

- `src-tauri/src/protocol/mod.rs`: 旧Comment/Thread関連のmod宣言・WsMessage variant削除
- `src-tauri/src/ws_server/handlers.rs`: 旧Comment/Thread handler関数削除
- `src-tauri/src/ws_server/routing.rs`: 旧Comment/Thread routing削除
- `src-tauri/src/lib.rs`: 旧store登録削除 + 新diff_comment_store/diff_comment_senderのmod宣言・コマンド登録・manage追加

### フロントエンド — 新規

- `src/types/diffComment.ts`: DiffComment型（行コメント/ファイルコメント共通）、CommentStatus型
- `src/hooks/useDiffComments.ts`: Tauri invoke経由のCRUD + イベントリスナー + ステータス管理
- `src/components/panels/DiffInlineComment.tsx`: インラインコメント表示/編集コンポーネント（diff行間に挿入）
- `src/components/panels/DiffFileComment.tsx`: ファイルコメント表示/編集コンポーネント（diffヘッダ下に表示）

### フロントエンド — 修正

- `src/components/panels/ShikiDiffViewer.tsx`: コメントUIの組み込み（行ホバーで追加ボタン表示、コメントをdiff行間にインライン表示）
- `src/components/panels/DiffViewerSection.tsx`: ファイルコメント表示領域の追加
- `src/components/panels/ReviewPanel.tsx`: useDiffCommentsフック接続、Agent一括送信ボタンの配置

### フロントエンド — 削除

- `src/types/comment.ts`, `src/types/thread.ts`
- `src/hooks/useLineComments.ts`, `src/hooks/useThreads.ts`
- `src/components/panels/CommentList.tsx`, `src/components/panels/CommentThread.tsx`
- `src/lib/commentThreadWidget.ts`, `src/lib/threadAnchor.ts`, `src/lib/formatCommentForClipboard.ts`, `src/lib/formatCommentsForTerminal.ts`, `src/lib/prCommentMapping.ts`
- `src/remote/components/RemoteCommentList.tsx`, `src/remote/components/RemoteThreadList.tsx`, `src/remote/hooks/useRemoteThreads.ts`
- 上記の関連テストファイル全て

### データモデル

```rust
struct DiffComment {
    id: String,                    // UUID
    file_path: String,             // worktree相対パス
    comment_type: String,          // "line" | "file"
    line_number: Option<u32>,      // 行コメント時: 開始行（newLineNumber基準）
    end_line: Option<u32>,         // 複数行範囲コメント時: 終了行
    content: String,               // コメント本文
    status: String,                // "unsent" | "sent"
    created_at: f64,               // Unix timestamp
}
```

### 永続化

- パス: `~/.local/share/releash/diff-comments/{worktree_name}.json`
- 形式: JSON配列
- パターン: worktree名サニタイズ（`/` `\` → `_`）、`RwLock<HashMap>`インメモリキャッシュ、CRUD後に即ファイル保存 + Tauriイベント(`diff-comments-changed`)発火

### Agent送信フロー

1. ユーザーがコメント送信ボタンをクリック（個別 or 一括）
2. フロントが `invoke("send_diff_comments_to_agent", { worktreeName, commentIds })` を呼び出し
3. Rust側で以下を1コマンド内で完結:
   - コメントを `@ファイルパス` メンション付きテキストにフォーマット変換
   - Agent SDKを通じてAgent chatにメッセージ送信
   - 送信済みコメントのステータスを `"sent"` に更新・永続化
4. フロントはコマンド完了後にUIを更新（Tauriイベント経由）

送信フォーマット例:
```
@src/components/Example.tsx L42: ここのロジックは〇〇すべき
@src/components/Example.tsx L10-15: この範囲のエラーハンドリングが不足
@src/lib/utils.ts (file comment): 全体的にテストが不足している
```

### ShikiDiffViewer内のコメントUI

- **行コメント追加**: diff行のホバーで `+` アイコン表示 → クリックでテキストエリア展開（diff行間に挿入）
- **行コメント表示**: コメントが存在する行の直下にインラインウィジェット表示（virtualizerのアイテムとして挿入）
- **ファイルコメント追加/表示**: DiffViewerSectionのヘッダ直下にコメント領域
- **複数行範囲**: 行選択状態でコメント追加 → end_line付きコメント。対象範囲はボーダーでハイライト

**影響するテスト**:
- **Rust単体テスト**: `diff_comment_store.rs`内に`#[cfg(test)]`でCRUD・永続化のテスト。`diff_comment_sender.rs`内にフォーマット変換のテスト
- **フロントエンド単体テスト**: `useDiffComments.test.ts`（フック状態遷移）、`DiffInlineComment.test.tsx`（コンポーネント表示・操作）、`DiffFileComment.test.tsx`
- **ShikiDiffViewer.test.tsx**: コメント表示・追加UIの統合テスト追加
