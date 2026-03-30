## 要求

**種別**: バグ修正
**現在の挙動**: AskUserQuestion の UI でマークダウンが正しくレンダリングされず、生のマークダウン記法（バッククォートなど）がそのまま表示される
**期待する挙動**: AskUserQuestion の UI 上で、マークダウン記法が正しく HTML に変換されて表示される（例: バッククォートはインラインコードとして表示される）
**再現手順**:
1. エージェントがAskUserQuestionツールを使って質問を送信する
2. questions フィールドにマークダウン記法を含むテキストが含まれている
3. UI上でマークダウンが変換されずに生テキストとして表示される
**背景**: マークダウンを含む質問文がそのまま表示されるため、可読性が低下しユーザー体験を損なっている。影響範囲は question 文のみか options の description 等も含むかは不明のため、調査の上で全テキスト箇所に対応する

## 振る舞い定義

```gherkin
Feature: AskUserQuestion UIのマークダウンレンダリング
  エージェントがAskUserQuestionツールで送信した質問のUI上で、
  マークダウン記法が正しくHTMLに変換されて表示される

  Rule: マークダウンを含むテキストはHTMLに変換して表示する
    Scenario: 質問文のマークダウンが正しくレンダリングされる
      Given エージェントがマークダウン記法を含む質問文を送信している
      When ユーザーがAskUserQuestionの質問UIを表示する
      Then 質問文のマークダウンがHTMLに変換されて表示される

    Scenario: ヘッダーのマークダウンが正しくレンダリングされる
      Given エージェントがマークダウン記法を含むヘッダーを送信している
      When ユーザーがAskUserQuestionの質問UIを表示する
      Then ヘッダーのマークダウンがHTMLに変換されて表示される

    Scenario: 選択肢説明文のマークダウンが正しくレンダリングされる
      Given エージェントがマークダウン記法を含む選択肢説明文を送信している
      When ユーザーがAskUserQuestionの質問UIを表示する
      Then 選択肢説明文のマークダウンがHTMLに変換されて表示される

  Rule: 回答済みの質問でもマークダウンが正しく表示される
    Scenario: 回答済み質問文のマークダウンが正しくレンダリングされる
      Given ユーザーがAskUserQuestionに回答済みである
      When 回答済みの質問UIが表示される
      Then 質問文のマークダウンがHTMLに変換されて表示される

    Scenario: 回答テキストのマークダウンが正しくレンダリングされる
      Given ユーザーがAskUserQuestionに回答済みである
      When 回答済みの質問UIが表示される
      Then 回答テキストのマークダウンがHTMLに変換されて表示される
```

## 実装仕様

**対応方針**: AskUserQuestion UIのマークダウンレンダリングを実現するために、`PermissionDialog.tsx` に対して、生テキスト表示の `<p>` タグを既存の `react-markdown` の `<Markdown>` コンポーネントに置き換えるアプローチで対応する。

**対象コンポーネント**:
- `src/components/panels/AgentChatPanel/PermissionDialog.tsx`:
  - `q.question`（Pending/Resolved）: `<p>` → `<Markdown>` に置き換え
  - `q.header`（Pending）: `<p>` → `<Markdown>` に置き換え
  - `opt.description`（Pending）: `title` 属性から表示要素に変更し、`<Markdown>` でレンダリング
  - `resolvedAnswers[q.question]`（Resolved）: `<p>` → `<Markdown>` に置き換え

**技術選定**:
- 新規ライブラリ導入なし。既存の `react-markdown` + `src/lib/markdownConfig.ts` の共有プラグイン設定（`remarkPluginList` / `rehypePluginList`）を再利用

**検討した代替案**:
- `MarkdownPreview` 汎用コンポーネントの利用: `useDeferredValue` のオーバーヘッドが不要な短いテキストであるため、直接 `<Markdown>` コンポーネントを使用する方が適切

**影響するテスト**:
- `PermissionDialog.test.tsx`: AskUserQuestionの各フィールドでマークダウンがHTMLに変換されるテストを追加（ExitPlanModeの既存テストパターンを参考）
