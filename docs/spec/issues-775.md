## 要求

**種別**: バグ修正
**現在の挙動**: AgentChatでPlanモード時にAIが計画を出すと横スクロールが出現する。AskUserQuestion表示時にも同様の現象が発生する見込み。
**期待する挙動**: Plan表示・AskUserQuestion表示ともに、コンテンツは領域幅で折り返されて表示され、横スクロールは発生しない。
**再現手順**:
1. AgentChatを開く
2. Planモードに切り替えてAIに計画を出させる
3. 計画メッセージに対して横スクロールバーが出現する
**背景**: AgentChatでは常に横スクロールを出さない方針（Issue #750 でScrollAreaベースの折り返し対応済）だが、Plan表示とAskUserQuestion表示がこの原則から外れているバグ。
**影響範囲**: AgentChat内のPlan表示コンポーネントとAskUserQuestion表示コンポーネント。

## 振る舞い定義

```gherkin
Feature: AgentChat内のPlan・AskUserQuestion表示における横スクロール防止
  AgentChatではすべてのコンテンツが領域幅で折り返され、
  横スクロールが発生しない方針である（Issue #750）。
  Plan表示とAskUserQuestion表示もこの方針に従う。

  Rule: コンテンツは表示領域の幅で折り返される
    Scenario: Plan表示のコンテンツが領域幅で折り返される
      Given AgentChatでPlanモードのメッセージが表示されている
      When 計画内容に領域幅を超える長さのテキストが含まれている
      Then コンテンツは領域幅で折り返されて表示される

    Scenario: AskUserQuestion表示のコンテンツが領域幅で折り返される
      Given AgentChatでAskUserQuestionのメッセージが表示されている
      When 質問内容に領域幅を超える長さのテキストが含まれている
      Then コンテンツは領域幅で折り返されて表示される

  Rule: 横スクロールバーは表示されない
    Scenario: Plan表示で横スクロールバーが出現しない
      Given AgentChatでPlanモードのメッセージが表示されている
      When 計画内容にコードブロックや長い単語が含まれている
      Then 横スクロールバーは表示されない

    Scenario: AskUserQuestion表示で横スクロールバーが出現しない
      Given AgentChatでAskUserQuestionのメッセージが表示されている
      When 質問内容にコードブロックや長い単語が含まれている
      Then 横スクロールバーは表示されない
```

## 実装仕様

**対応方針**: Plan表示・AskUserQuestion表示の横スクロールを防止するために、InlineMarkdownコンポーネントにデフォルトの折り返し指定を追加し、PermissionDialogのコンテナに幅制御を追加する。Issue #750 のパターン（コンポーネント側での折り返し徹底）に準拠。

**対象コンポーネント**:
- `src/components/panels/AgentChatPanel/PermissionDialog.tsx`:
  - `InlineMarkdown`: デフォルトクラスに `break-words` を追加（58行目）
  - AskUserQuestionコンテナ（290行目）: `overflow-hidden` を追加
  - ExitPlanModeコンテナ（417行目）: `overflow-hidden` を追加

**検討した代替案**:
- 各InlineMarkdown呼び出し箇所に個別に `break-words` を追加する案 → 漏れのリスクがあり、InlineMarkdown本体への追加のほうが一貫性が高いため却下
- グローバルCSSで `.markdown-preview` に `overflow-wrap: break-word` を追加する案 → Issue #750 で `contain: inline-size` は既に適用済みだが、親コンテナの幅制御がないと効かないため、コンポーネント側の修正も必要

**影響するテスト**:
- 既存テスト: `PermissionDialog.test.tsx` が存在すれば回帰確認。スタイル変更のみのため既存テストの期待値変更は不要
- 新規テスト: CSSクラスの変更のみで視覚的な確認が主体のため、自動テスト追加は不要。手動で横スクロールが発生しないことを確認
