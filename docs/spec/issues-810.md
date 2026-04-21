## 要求

**種別**: 改善
**ゴール**: Diffビューを開いた時のデフォルト表示をDiff表示からコード（ファイル内容）表示に変更する
**背景**: 現在はファイルを開くとDiff表示がデフォルトだが、コード表示をデフォルトにした方が使いやすい

## 振る舞い定義

```gherkin
Feature: Markdownファイルのデフォルト表示モード
  Markdownファイルを開いた時のデフォルト表示をプレビューからコード表示に変更する

  Rule: Markdownファイルのデフォルト表示はコード表示である
    Scenario: Markdownファイルを開くとコード表示になる
      Given 変更のあるMarkdownファイルが存在する
      When ユーザーがそのファイルを選択する
      Then コード表示がデフォルトで表示される

  Rule: プレビュー表示への切り替えは手動で行える
    Scenario: コード表示からプレビュー表示に切り替える
      Given Markdownファイルがコード表示で表示されている
      When ユーザーがプレビュー表示に切り替える
      Then プレビュー表示に変わる
```

## 実装仕様

**対応方針**: Markdownファイルのデフォルト表示をプレビューからコード表示に変更するために、`ReviewPanel.tsx` の `showMarkdownPreview` の初期値を `true` → `false` に変更する。

**対象コンポーネント**:
- `src/components/panels/ReviewPanel.tsx`: `showMarkdownPreview` の `useState(true)` → `useState(false)` に変更

**影響するテスト**:
- `ReviewPanel` のテストで `showMarkdownPreview` のデフォルト値に依存するケースがあれば修正
