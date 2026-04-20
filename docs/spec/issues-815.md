## 要求

**種別**: バグ修正
**現在の挙動**: ワークツリーがRebase中にdetached HEAD状態になると、Workspaces一覧から表示が消える
**期待する挙動**: Rebase中（detached HEAD状態）でも、ワークツリーがWorkspaces一覧に表示され続ける
**再現手順**:
1. ワークツリーでRebaseを開始する
2. Rebase中にdetached HEAD状態になる
3. Workspaces一覧を確認すると、該当ワークツリーが表示されていない
**背景**: Rebaseは日常的なGit操作であり、その間ワークツリーが一覧から消えると、ユーザーはそのワークツリーにアクセスできなくなる

## 振る舞い定義

```gherkin
Feature: ワークツリー一覧の表示
  全てのワークツリーがWorkspaces一覧に正しく表示される

  Rule: detached HEAD状態のワークツリーもワークツリー一覧に含まれる
    Scenario: Rebase中のワークツリーが一覧に含まれる
      Given ワークツリーでRebaseが進行中である
      And そのワークツリーがdetached HEAD状態である
      When ワークツリー一覧を取得する
      Then そのワークツリーのworktree_pathが返される

  Rule: メインリポジトリもWorkspaces一覧に表示される
    Scenario: メインリポジトリが一覧に含まれる
      Given メインリポジトリが存在する
      When ユーザーがWorkspaces一覧を表示する
      Then メインリポジトリが一覧に表示される

  Rule: メインリポジトリはメインであることが識別できる
    Scenario: メインリポジトリにメイン識別アイコンが表示される
      Given メインリポジトリがWorkspaces一覧に表示されている
      When ユーザーがWorkspaces一覧を表示する
      Then メインリポジトリにメインであることを示すアイコンが表示される

  Rule: 全てのワークツリーがWorkspaces一覧に表示される
    Scenario: detached HEAD状態のワークツリーがWorkspaces一覧に表示される
      Given ワークツリーがdetached HEAD状態でworktree_pathを持っている
      When ユーザーがWorkspaces一覧を表示する
      Then そのワークツリーが一覧に表示される
```

## 実装仕様

**対応方針**: detached HEADのワークツリーが一覧から欠落するバグを修正するために、Rust側の `list_branches_with_status` で、ローカルブランチのイテレーション後に `wt_map` に残ったエントリ（ブランチにマッチしなかったワークツリー）を `WorktreeBranch` として追加する。

**対象コンポーネント**:
- `src-tauri/src/git/worktree.rs` (`list_branches_with_status`): ローカルブランチイテレーション後、`wt_map` からブランチにマッチしなかったワークツリー（detached HEAD等）を検出し、`WorktreeBranch` として `cards` に追加する
- `src/hooks/useWorktreeList.ts`: メインリポジトリの表示対応 — `worktree_path` を持つエントリを一覧に残し、`is_main_worktree` をフロントエンドへ伝搬する
- `src/components/workspace/WorkspaceList.tsx`: メインリポジトリのアイコン表示対応

**検討した代替案**:
- フロントエンド側のフィルタ修正のみ: Rust側からdetached HEADワークツリーのデータが返却されないため、フロントのフィルタ修正だけでは解決不可。却下

**影響するテスト**:
- Rust: `src-tauri/src/git/worktree.rs` にdetached HEAD状態のワークツリーが `list_branches_with_status` の結果に含まれることを検証するテストを追加
- フロントエンド: `useWorktreeList` のフィルタロジック変更に伴うテスト修正（メインリポジトリが含まれることの検証）
