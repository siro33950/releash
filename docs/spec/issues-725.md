## 要求

**種別**: バグ修正
**現在の挙動**: AgentChatをBypassモードで開始すると、セレクトボックスがcodeに戻ってしまう。Agent側がPlanモードに変更した場合はPlanに正しく同期される。実際にBypass指定時にどのモードで動作しているかは不明。
**期待する挙動**: セレクトボックスの表示とAgentの実際の動作モードが常に同期されること。Bypassで開始した場合はセレクトボックスもBypassを表示し、AgentもBypassモードで動作すべき。
**再現手順**:
1. AgentChatをBypassモードで開始する
2. セレクトボックスがcodeに戻っている（常に発生）
**背景**: UIの表示とAgentの実際の動作モードが不一致になり、ユーザーが現在のモードを正確に把握できない。

## 振る舞い定義

```gherkin
Feature: AgentChatのモード同期
  AgentChatのセレクトボックス表示とAgentの実際の動作モードが常に一致すること。

  Rule: ユーザーがモードを変更するとAgentの動作モードに反映される
    Scenario: セッション開始時にBypassモードを指定する
      Given ユーザーがモードセレクタでBypassを選択している
      When 新しいセッションを開始する
      Then Agentの動作モードがBypassである

    Scenario: セッション途中でBypassモードに変更する
      Given Codeモードでセッションが進行中である
      When ユーザーがモードセレクタでBypassに変更する
      Then Agentの動作モードがBypassである

  Rule: セレクトボックスはユーザーが指定したモードを表示する
    Scenario: Bypassモードでセッション開始後のセレクトボックス表示
      Given ユーザーがBypassモードでセッションを開始した
      When セッションが開始された状態を表示する
      Then セレクトボックスにBypassが表示されている

    Scenario: セッション途中でBypassに変更後のセレクトボックス表示
      Given セッション途中でユーザーがBypassモードに変更した
      When 変更後の状態を表示する
      Then セレクトボックスにBypassが表示されている

  Rule: Agentによるモード変更後にユーザー指定モードへ復帰する
    Scenario: BypassモードからAgentがPlanモードに変更した後に復帰する
      Given ユーザーがBypassモードを指定している
      And AgentがPlanモードに変更した
      When Agentがモードをdefaultに戻す
      Then Agentの動作モードがBypassに復帰する

  Rule: モード復帰後のセレクトボックス表示
    Scenario: Bypass復帰後のセレクトボックス表示
      Given BypassモードからAgentのモード変更を経て復帰した
      When 復帰後の状態を表示する
      Then セレクトボックスにBypassが表示されている
```

## 実装仕様

**対応方針**: モードセレクタの表示とAgentの実際の動作モードを同期するために、SDKの`Query.setPermissionMode()` APIを使って既存プロセスにpermissionMode変更を動的に反映する。

**対象コンポーネント**:
- `src-tauri/resources/claude-sdk-bridge.mjs`: `setMode`コマンドハンドラ追加、`canUseTool`内のpermissionMode参照を動的変数化
- `src-tauri/src/agent_sdk.rs`: `execute_agent_query`で既存プロセスに`setMode`コマンドを送信

**検討した代替案**:
- プロセス再起動: permissionMode変更時に既存プロセスをkillして新モードで再起動する案。却下理由: SDKに`Query.setPermissionMode()`が存在し動的変更が可能なため不要。プロセス再起動はセッション継続性のリスクもある。

**リスク**:
- `canUseTool`コールバックはBypass時は未設定、非Bypass時は設定という相互排他の設計のため、Bypass↔非Bypass切替時は`canUseTool`内部の分岐で対応が必要: 初期化時に常に`canUseTool`を設定し、内部で現在のpermissionModeを参照して動的に分岐する

**影響するテスト**:
- フロントエンド: `agentChatReducer.test.ts`にモード切替時の状態遷移テスト追加
- Rust: `agent_sdk.rs`に`setMode`コマンド送信のテスト追加
