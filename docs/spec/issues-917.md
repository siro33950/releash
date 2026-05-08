## 要求

**種別**: 改善
**ゴール**: ユーザーが設定画面（SettingsModal）のRepositoriesセクションから、登録済みリポジトリを削除できる
**背景**: バックエンド（`remove_repo_path` Tauriコマンド）とフロントエンドフック（`useRepoList.removeRepo`）は実装済みだが、UIに削除操作の導線がない。ユーザーは登録したリポジトリを解除する手段がなく、不要なリポジトリが一覧に残り続ける。

### 現状
- `src-tauri/src/repo_registry.rs`: `remove_repo_path` コマンド実装済み
- `src/hooks/useRepoList.ts`: `removeRepo` 関数実装済み
- `src/components/panels/SettingsModal.tsx`: `RepositoriesSection` にはベースブランチ設定のみ表示。削除UIなし

### 要件
- 設定画面のRepositoriesセクションで、各リポジトリに削除ボタンを表示する
- 削除ボタン押下時に確認ダイアログを表示し、確認後に削除を実行する
- 削除は登録解除のみ（ディスク上のリポジトリは削除しない）
- 削除操作はワークスペースマネージャー画面（`App.tsx`）の設定画面からのみ可能とする。worktreeビュー（`MainLayout.tsx`）の設定画面では削除ボタンを表示しない（現在作業中のリポジトリを削除した場合の画面遷移が未定義のため）
- 確認ダイアログは既存の `DeleteConfirmDialog` を再利用するが、説明文をカスタマイズ可能にし、登録解除であることが伝わるメッセージ（例: "Remove from list? The repository will not be deleted from disk."）を表示する
- 削除処理のエラーは現時点ではログのみとし、ユーザー通知は行わない

## 振る舞い定義

```gherkin
Feature: 登録済みリポジトリの削除

  Rule: リポジトリの登録解除
    Scenario: 確認後にリポジトリが登録解除される
      Given リポジトリが登録されている
      When そのリポジトリの削除を確認する
      Then リポジトリが登録一覧から除外される
      And ディスク上のリポジトリは削除されない

    Scenario: 削除をキャンセルするとリポジトリは維持される
      Given リポジトリが登録されている
      When そのリポジトリの削除をキャンセルする
      Then リポジトリは登録一覧に残る

  Rule: Repositoriesセクションの削除UI表示
    Scenario: ワークスペースマネージャーの設定画面で各リポジトリに削除操作が表示される
      Given ワークスペースマネージャーの設定画面のRepositoriesセクションを表示している
      When リポジトリが登録されている
      Then 各リポジトリに削除ボタンが表示される

    Scenario: worktreeビューの設定画面では削除ボタンが表示されない
      Given worktreeビューの設定画面のRepositoriesセクションを表示している
      When リポジトリが登録されている
      Then リポジトリに削除ボタンが表示されない

    Scenario: 削除開始時に確認ダイアログが表示される
      Given ワークスペースマネージャーの設定画面のRepositoriesセクションを表示している
      When リポジトリの削除ボタンを押す
      Then 確認ダイアログが表示される
      And 確認ダイアログにはリポジトリがディスクから削除されない旨のメッセージが表示される

    Scenario: 登録解除後にリポジトリが一覧から消える
      Given ワークスペースマネージャーの設定画面のRepositoriesセクションにリポジトリが表示されている
      When そのリポジトリの登録が解除される
      Then リポジトリが一覧から消える
```

## 実装仕様

**対応方針**: 振る舞い定義を実現するために、`SettingsModal` に `onRemoveRepo` コールバックを追加し、`RepositoriesSection` の各リポジトリ項目に削除ボタンを配置する。削除確認には既存の `DeleteConfirmDialog` コンポーネントを再利用し、説明文をカスタマイズ可能にする。バックエンド（`remove_repo_path`）とフック（`useRepoList.removeRepo`）は実装済みのため、UI層の接続のみで完結する。

**対象コンポーネント**:
- `src/components/panels/DeleteConfirmDialog.tsx`:
  - `DeleteConfirmDialogProps` にオプショナルな `description?: string` を追加
  - `description` が指定された場合はデフォルトメッセージの代わりに表示する
- `src/components/panels/SettingsModal.tsx`:
  - `SettingsModalProps` に `onRemoveRepo?: (path: string) => void` を追加
  - `RepositoriesSection` に `onRemoveRepo` を伝播
  - `onRemoveRepo` が未指定の場合、削除ボタンを表示しない
  - `RepoBaseBranchItem` の各行に `Trash2` アイコンの削除ボタンを追加（`onRemoveRepo` が指定されている場合のみ）
  - 削除ボタン押下で `DeleteConfirmDialog` を表示する状態管理を `RepositoriesSection` に追加
  - `DeleteConfirmDialog` の `description` に登録解除である旨のメッセージを指定する
  - 確認後に `onRemoveRepo(path)` を呼び出し（リスト更新は `repo-paths-changed` イベント経由で自動反映）
- `src/App.tsx`: `SettingsModal` に `onRemoveRepo={removeRepo}` を追加（`useRepoList` から `removeRepo` を取得）
- `src/screens/MainLayout.tsx`: `SettingsModal` に `onRemoveRepo` を渡さない（worktreeビューでは削除操作を提供しない）

**エラーハンドリング**: `useRepoList.removeRepo` のエラーは現時点では `console.warn` でログするのみとし、UIレベルでのエラー通知は行わない。

**影響するテスト**:
- `src/components/panels/SettingsModal.test.tsx`: 削除ボタンの表示（`onRemoveRepo` 有無による出し分け）、確認ダイアログの表示/キャンセル/確認後コールバック呼び出しのテストを追加
- `src/components/panels/DeleteConfirmDialog.test.tsx`: カスタム `description` の表示テストを追加（既存テストがある場合）
- `src/hooks/useRepoList.test.ts`: 既存テストで `removeRepo` はカバー済み。追加不要
- `src-tauri/src/repo_registry.rs`: 既存テストで `remove_repo` はカバー済み。追加不要
