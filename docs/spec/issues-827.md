## 要求

**種別**: バグ修正
**現在の挙動**: 外部プロセス（組み込みAI Agent含む）がファイルを変更しても、Diff一覧（SourceControlPanel）に変更が反映されないことがある
**期待する挙動**: 何がファイルを変更しても（外部プロセス、Agent、手動編集問わず）、Diff一覧に常に即座に反映される
**再現手順**: 不明（間欠的に発生）
**背景**: 現在のファイル変更監視（watcher）が外部プロセスによるファイル変更を検知できないケースがあり、Diff一覧が最新の状態と乖離する

## 振る舞い定義

```gherkin
Feature: ファイル変更のDiff一覧即時反映
  ワーキングツリー内のファイルが変更されたとき、
  変更元がエディタ操作・外部プロセス・AI Agentのいずれであっても
  Diff一覧（SourceControlPanel）に即座に反映される

  Rule: ファイル変更はDiff一覧の状態に反映される
    Scenario: 外部プロセスがファイルを新規作成する
      Given ワーキングツリーにuntracked fileがない
      When 外部プロセスが新規ファイルを作成する
      Then Diff一覧にそのファイルが新規ファイルとして表示される

    Scenario: 外部プロセスが既存ファイルを変更する
      Given ワーキングツリーにtracked fileが存在する
      When 外部プロセスがそのファイルを変更する
      Then Diff一覧にそのファイルが変更ファイルとして表示される

    Scenario: 外部プロセスがファイルを削除する
      Given ワーキングツリーにtracked fileが存在する
      When 外部プロセスがそのファイルを削除する
      Then Diff一覧にそのファイルが削除ファイルとして表示される

    Scenario: 外部プロセスが複数ファイルを一括変更する
      Given ワーキングツリーにtracked fileが複数存在する
      When 外部プロセスが複数ファイルを短時間に連続で変更する
      Then Diff一覧に全ての変更ファイルが表示される

  Rule: Diff一覧はファイル変更前の状態に戻らない
    Scenario: 変更検知後にDiff一覧がリセットされない
      Given 外部プロセスがファイルを変更してDiff一覧に反映された
      When 追加のファイル変更が発生しない
      Then Diff一覧は変更ファイルを表示し続ける
```

## 実装仕様

**対応方針**: ファイル変更のDiff一覧即時反映を実現するために、watcher.rs（バックエンド）とuseGitEventRefresh（フロントエンド）に対して、イベント検知の信頼性向上で対応する。

**対象コンポーネント**:
- `src-tauri/src/watcher.rs`: file-changeイベントのパスを正規化して発行する
- `src/hooks/useGitEventRefresh.ts`: パスマッチングの堅牢化、デバウンス後の確実なリフレッシュ保証

**具体的な変更:**

1. **watcher.rs — パスの正規化**
   - `start_watching` のイベント発行時に `event.path.canonicalize()` でパスを正規化
   - シンボリックリンクやOS依存のパス表現の差異を解消

2. **useGitEventRefresh.ts — パスマッチングの堅牢化**
   - `rootPath` もマウント時にTauriコマンド経由で正規化パスを取得し比較に使用
   - あるいは、パス前方一致ではなくwatcher_idベースのフィルタリングに変更（useFileWatcherと同じ方式）

3. **useGitEventRefresh.ts — デバウンスの trailing edge 保証**
   - 現在のsetTimeoutベースのデバウンスで、連続イベント時に最後のイベント後に確実に1回実行されることを確認・修正

**検討した代替案**:
- ポーリング方式（定期的にgit statusを取得）: 不要な呼び出しが増えるため却下
- ファイルウォッチャーのデバウンスを短縮: OS負荷が増えるため却下

**影響するテスト**:
- Rust単体テスト: watcher.rsのパス正規化ロジック
- フロントエンド単体テスト: useGitEventRefreshのイベントフィルタリング
