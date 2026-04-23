## 要求

**種別**: バグ修正
**現在の挙動**: Diff表示画面のコード本文でマウスドラッグしてもテキストが選択できず、コピーできない
**期待する挙動**: コード本文のテキストをマウスドラッグで選択し、Cmd+Cでコピーできる
**再現手順**:
1. Diff表示画面でファイルを表示する
2. コード本文をマウスドラッグで選択しようとする
3. テキストが選択状態にならない
**背景**: `body` に `select-none`（user-select: none）が適用されており、input/textarea以外の要素ではテキスト選択が無効化されている。Diff表示のコード本文（span要素）に `user-select: text` の上書きがないため、bodyの設定が継承される

## 振る舞い定義

```gherkin
Feature: Diffコード本文のテキスト選択・コピー
  Diff表示画面でコード本文のテキストを選択してコピーできる

  Rule: コード本文はテキスト選択可能である
    Scenario: コード本文をマウスドラッグで選択できる
      Given Diff表示画面でファイルの差分が表示されている
      When コード本文をマウスドラッグする
      Then ドラッグした範囲のテキストが選択状態になる

    Scenario: 選択したテキストをコピーできる
      Given コード本文のテキストが選択状態である
      When Cmd+Cを押す
      Then 選択したテキストがクリップボードにコピーされる

  Rule: 行番号・マーカーはテキスト選択対象外である
    Scenario: 行番号はテキスト選択されない
      Given Diff表示画面でファイルの差分が表示されている
      When コード本文を含む行全体をマウスドラッグで選択する
      Then 行番号とマーカー（+/-）は選択範囲に含まれない
```

## 実装仕様

**対応方針**: コード本文のテキスト選択を有効にするために、ShikiDiffViewerのコード本文を表示する`<span>`要素に`select-text`クラスを追加する。

**対象コンポーネント**:
- `src/components/panels/ShikiDiffViewer.tsx`: コード本文を表示する`<span>`要素（GutterLineRow、InlineView、SplitViewの各行）に`select-text`（TailwindCSS、`user-select: text`に展開）を追加

**検討した代替案**:
- `src/index.css`でbodyの`select-none`を除去する案: アプリ全体のUI要素（ツールバー、ファイルツリー等）でテキスト選択が有効になり、意図しないUI操作感の変化が起きるため却下

**影響するテスト**:
- テキスト選択はブラウザのネイティブ挙動（CSS `user-select`プロパティ）であり、jsdomでは検証できない。CSSクラスの付与をユニットテストで確認する
