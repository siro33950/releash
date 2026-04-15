## 要求

**種別**: バグ修正
**現在の挙動**: AskUserQuestionのオプションカードで、ラベルテキストと説明テキストが1文字ずつ改行されて表示が崩れる（例：「docs/spec/PJT-1839.md を使用する」が縦に1文字ずつ並ぶ）
**期待する挙動**: テキストが通常の単語単位で折り返され、カード内で正しく表示される
**再現手順**:
1. AgentChatでClaudeがAskUserQuestionを送信する
2. オプションのラベルや説明文が長い場合、テキストが1文字ずつ改行されて表示される
**背景**: flexレイアウト内のテキストコンテナに`min-w-0`が未設定のため、flex子要素のデフォルト`min-width: auto`が効いてテキストが正しく折り返されない

## 振る舞い定義

```gherkin
Feature: AskUserQuestionオプションカードのテキスト表示
  AgentChatでClaudeが送信するAskUserQuestionのオプションカードにおいて、
  ラベルと説明テキストが正しく折り返されて表示される

  Rule: オプションカードのテキストは単語単位で折り返される
    Scenario: 長いラベルテキストが正しく折り返される
      Given AskUserQuestionのオプションにラベル幅を超える長いラベルテキストがある
      When オプションカードが表示される
      Then ラベルテキストはカード幅に応じて単語単位で折り返される

    Scenario: 長い説明テキストが正しく折り返される
      Given AskUserQuestionのオプションに説明幅を超える長い説明テキストがある
      When オプションカードが表示される
      Then 説明テキストはカード幅に応じて単語単位で折り返される

    Scenario: マルチセレクトモードでもテキストが正しく折り返される
      Given AskUserQuestionがマルチセレクトモードである
      And オプションにカード幅を超える長いテキストがある
      When オプションカードが表示される
      Then テキストはカード幅に応じて単語単位で折り返される
```

## 実装仕様

**対応方針**: オプションカードのテキスト折り返しを修正するために、`PermissionDialog.tsx` のflex子要素であるテキストコンテナに `min-w-0` を追加する。

**対象コンポーネント**:
- `src/components/panels/AgentChatPanel/PermissionDialog.tsx`: オプションカード内のテキストコンテナ（`<div className="flex flex-col">`）3箇所に `min-w-0` を追加
  - マルチセレクト時のオプションカード
  - シングルセレクト時のオプションカード
  - シングルセレクト時のOtherオプション

**影響するテスト**:
- CSS修正のみのため、既存テスト（`PermissionDialog.test.tsx`）への影響なし。テキスト折り返しはjsdom環境では検証不可のため、目視確認で検証する
