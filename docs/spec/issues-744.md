## 要求

**種別**: リファクタリング
**ゴール**: AgentTerminal関連のコード・UIを全削除する
**背景**: AgentChatに機能統合済みで、AgentTerminalは不要となっているため

## 振る舞い定義

```gherkin
Feature: AgentTerminal関連コード・UIの削除
  AgentChatに機能統合済みのAgentTerminalを全削除し、
  AgentChat（チャットパネル）のみのシンプルな構成にする

  Rule: AgentTabのChat/Terminal切り替えが存在しない
    Scenario: デスクトップアプリでAgentビューを表示する
      Given ワークスペースが開かれている
      When ユーザーがAgentビューを表示する
      Then AgentChatが直接表示される
      And Chat/Terminalの切り替えUIは存在しない

  Rule: リモートアプリにAgentタブが存在しない
    Scenario: リモートアプリでタブ一覧を表示する
      Given リモートアプリに接続している
      When タブ一覧を確認する
      Then Agentタブは存在しない
      And Terminalタブは引き続き存在する

  Rule: PTYセッションのkindからagentが除外される
    Scenario: 新しいPTYセッションを作成する
      Given ターミナルを使用している
      When PTYセッションが作成される
      Then セッションのkindはterminalまたはone_shotのみである

  Rule: ビルド・テストが通る
    Scenario: 削除後にビルドが成功する
      Given AgentTerminal関連コードが削除されている
      When ビルドを実行する
      Then フロントエンド・Rustともにビルドが成功する

    Scenario: 削除後にテストが成功する
      Given AgentTerminal関連コードが削除されている
      When テストを実行する
      Then 全てのテストが成功する
```

## 実装仕様

**対応方針**: AgentChatに統合済みのAgentTerminal関連コード・UIを全削除するために、フロントエンド（デスクトップ・リモート）とRustバックエンドの両方からAgent Terminal専用コードを除去し、Terminal/OneShotのみのシンプルな構成にする。

**対象コンポーネント**:

### 削除対象ファイル

| ファイル | 理由 |
|---|---|
| `src/components/panels/AgentTab.tsx` | AgentのChat/Terminal切り替えUI（全体がAgent専用） |
| `src/components/panels/AgentTab.test.tsx` | 上記のテスト |
| `src/remote/hooks/useAgentState.ts` | リモートアプリのAgent状態購読フック |

### 修正対象ファイル（Agent関連コードの除去）

**フロントエンド（デスクトップ）:**

| ファイル | 変更内容 |
|---|---|
| `src/screens/MainLayout.tsx` | AgentTabのimport・レンダリング削除、centerTabデフォルト値変更 |
| `src/components/panels/TerminalTabPanel.tsx` | `agentType` prop削除、`tabPrefix`固定化、Agent状態管理（`agentStateByPtyId`、`session-status-changed`リスナー）削除 |

**フロントエンド（リモート）:**

| ファイル | 変更内容 |
|---|---|
| `src/remote/RemoteApp.tsx` | Agentタブ定義・`agentSessions`・`activeAgentPtyId`状態管理・Agentパネル描画削除 |
| `src/remote/hooks/usePtyManagement.ts` | `agentSessions`・`activeAgentPtyId`状態・Agentフィルタリング削除 |
| `src/remote/hooks/usePtyManagement.test.ts` | Agent関連テストケース削除 |

**Rust（バックエンド）:**

| ファイル | 変更内容 |
|---|---|
| `src-tauri/src/pty/mod.rs` | `PtyKind::Agent`除去、`register_pre_spawned`/`claim_pre_spawned`削除、`batch_spawn_agent_ptys`コマンド削除、`get_or_spawn_pty`内Agent事前生成ロジック削除、`gc_ptys_for_worktree`のkindフィルタ引数削除、Agent関連テスト削除 |
| `src-tauri/src/protocol/pty.rs` | `PtyReady`の`kind`フィールド削除、関連テスト削除 |

**型定義:**

| ファイル | 変更内容 |
|---|---|
| `src/types/protocol.ts` | `PtyReady.kind`フィールド削除 |

**影響するテスト**:
- フロントエンド: `AgentTab.test.tsx`削除、`usePtyManagement.test.ts`からAgent関連ケース削除
- Rust: `pty/mod.rs`内のAgent事前生成テスト群削除、`protocol/pty.rs`のAgent kindテスト削除
- 削除後に `pnpm test`、`pnpm build`、`cargo test`、`cargo clippy -- -D warnings` が全てPASSすること
