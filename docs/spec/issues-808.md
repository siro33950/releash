## 要求

**種別**: バグ修正
**現在の挙動**: AgentChatパネルのマークダウン表示で、テーブルが横に長い場合にブラウザデフォルトのスクロールバーが表示される
**期待する挙動**: アプリのUIテーマに合ったカスタムスクロールバーで横スクロールできる
**再現手順**:
1. AgentChatパネルでテーブルを含むマークダウンを表示させる
2. テーブルの列数が多く横幅がパネル幅を超える場合、ブラウザデフォルトのスクロールバーが表示される
**背景**: デスクトップアプリのUIとしてブラウザデフォルトのスクロールバーは見た目が統一されておらず、UXが損なわれる

## 振る舞い定義

```gherkin
Feature: AgentChatパネルのテーブル横スクロールバー表示
  AgentChatパネルでマークダウンテーブルが横幅を超える場合、
  アプリのUIテーマに統一されたカスタムスクロールバーで横スクロールできる

  Rule: テーブルの横スクロールバーはアプリのカスタムスタイルで表示される
    Scenario: 横幅を超えるテーブルのスクロールバー表示
      Given AgentChatパネルにパネル幅を超える列数のテーブルが表示されている
      When ユーザーがテーブルを確認する
      Then テーブルの横スクロールバーがアプリのカスタムスクロールバースタイルで表示される

    Scenario: 横幅に収まるテーブルにはスクロールバーが表示されない
      Given AgentChatパネルにパネル幅に収まるテーブルが表示されている
      When ユーザーがテーブルを確認する
      Then 横スクロールバーは表示されない
```

## 実装仕様

**対応方針**: テーブルのラッパーを現在のインラインスタイル付き `<div>` から、既存の `ScrollArea` + `ScrollBar orientation="horizontal"` コンポーネントに置き換える。ImageDiffViewer等と同じRadix UIベースのカスタムスクロールバーをテーブルにも適用する。

**対象コンポーネント**:
- `src/components/panels/AgentChatPanel/StreamMessage.tsx`: テーブルラッパーを `ScrollArea` + `ScrollBar orientation="horizontal"` に変更
- `src/index.css`: `.markdown-preview table` から `display: block; overflow-x: auto` を削除（ScrollAreaが制御するため不要）

**影響するテスト**:
- `StreamMessage` 関連テスト: テーブルラッパーの構造変更に伴う更新（該当テストがある場合）
