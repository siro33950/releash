## 要求

**種別**: 改善
**ゴール**: ワークスペースを切り替えて戻ってきたとき、以前Diffビューで選択していたファイルが復元される
**背景**: 現在、Diffビューの選択ファイル（`selectedFile`）は`useReviewPanel`内のローカルstateで管理されており、ワークスペース切り替え時にnullにリセットされる。ワークスペースを頻繁に切り替えるユーザーにとって、毎回ファイルを選び直す必要があり不便
**影響範囲**: WorkspaceState型の拡張、useReviewPanel/useWorktreeStateの永続化フロー

## 振る舞い定義

```gherkin
Feature: Diffビュー選択ファイルのワークスペース永続化
  ワークスペースを切り替えて戻ったとき、以前Diffビューで選択していたファイルが復元される

  Rule: ワークスペース切り替え時に選択ファイルが保存される
    Scenario: ワークスペースAでファイルを選択し、Bに切り替え、Aに戻ると選択が復元される
      Given ワークスペースAのDiffビューでファイル "src/main.rs" を選択している
      When ワークスペースBに切り替える
      And ワークスペースAに戻る
      Then Diffビューでファイル "src/main.rs" が選択されている

  Rule: 選択ファイルが存在しなくなった場合はリセットされる
    Scenario: 保存されていた選択ファイルが削除されている場合、選択なしになる
      Given ワークスペースAのDiffビューでファイル "src/deleted.rs" を選択した状態で保存されている
      And ファイル "src/deleted.rs" は既に削除されている
      When ワークスペースAを開く
      Then Diffビューでファイルは選択されていない

  Rule: 初回オープン時はデフォルト状態で表示される
    Scenario: 保存状態がないワークスペースを開くと選択なしで表示される
      Given ワークスペースCに保存された状態がない
      When ワークスペースCを開く
      Then Diffビューでファイルは選択されていない
```

## 実装仕様

**対応方針**: 既存のワークスペース状態永続化フロー（`WorkspaceState` → `InternalWorktreeState` → `useWorkspacePersistence` → `workspace_state_store.rs`）に `selectedDiffFile` フィールドを追加し、Diffビューの選択ファイルをワークスペースごとに保存・復元する。

**対象コンポーネント**:
- **`src/types/workspace-state.ts`**: `WorkspaceState.layout` に `selectedDiffFile?: string | null` を追加。`InternalWorktreeState` にも `selectedDiffFile` を追加。`buildWorkspaceState` で変換。
- **`src-tauri/src/workspace_state_store.rs`**: `WorkspaceLayoutState` に `selected_diff_file: Option<String>` を追加（`serde(skip_serializing_if)`付き）。diff対象ファイルはGit管理のためディスク存在チェック不要。
- **`src/screens/useWorktreeState.tsx`**: `selectedDiffFile` を `useState` で管理（初期値は `initialWorkspaceState?.layout.selectedDiffFile`）。`internalStateMapRef` への同期useEffectに追加。`setSelectedDiffFile` を返す。
- **`src/hooks/useReviewPanel.ts`**: `UseReviewPanelOptions` に `initialSelectedFile?: string | null` を追加し、`useState` の初期値として使用。
- **`src/hooks/useWorkspacePersistence.ts`**: 変更不要（`internalStateMapRef` と `buildWorkspaceState` 経由で自動永続化）。

**データフロー**:
1. `useWorktreeState` が `selectedDiffFile` のstateを保持し、`internalStateMapRef` に同期
2. `ReviewPanel` に `initialSelectedDiffFile` と `onSelectedDiffFileChange` コールバックを渡す
3. `useReviewPanel` の `selectFile` 呼び出し時に `onSelectedDiffFileChange` 経由で `useWorktreeState` に通知
4. ワークスペース切り替え時、既存の `buildWorkspaceState` → `flushState` フローで自動保存
5. 復元時、`initialWorkspaceState.layout.selectedDiffFile` が `useWorktreeState` → `ReviewPanel` → `useReviewPanel` に伝播

**影響するテスト**:
- **`src/hooks/useWorkspacePersistence.test.ts`**: A→B→Aの往復で `selectedDiffFile` が復元されるシナリオを追加
- **`src-tauri/src/workspace_state_store.rs`**: `make_state()` に `selected_diff_file` フィールドを追加、既存テストの更新
- **`src/hooks/useReviewPanel.test.ts`** (既存 or 新規): `initialSelectedFile` で初期値が設定されることを検証
