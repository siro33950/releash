## 要求

**種別**: バグ修正
**現在の挙動**: Monaco Editorから ShikiDiffViewerへの移行後、ファイル内検索（Cmd+F / Ctrl+F）機能が失われている。ShikiDiffViewerにはFind Widget相当の機能が未実装のため、表示中のdiffビュー内でテキスト検索ができない。
**期待する挙動**: ShikiDiffViewerで表示中のdiffビュー内で、Cmd+F（macOS）/ Ctrl+F（Windows/Linux）でテキスト検索ができる。
**再現手順**:
1. diffビューでファイルを開く
2. Cmd+F（またはCtrl+F）を押す
3. 何も起きない（検索UIが表示されない）
**背景**: Monaco Editor除去（#812）に伴い、Monaco組み込みのFind Widgetが利用不可になった。ShikiDiffViewerへの移行で検索機能の代替実装が行われていない。

## 振る舞い定義

```gherkin
Feature: ファイル内検索
  diffビューで表示中のファイル内容をテキスト検索し、マッチ箇所をハイライト・ナビゲーションできる。

  Rule: 検索バーの表示制御
    Scenario: キーボードショートカットで検索バーを開く
      Given diffビューでファイルが表示されている
      When Cmd+F（macOS）またはCtrl+F（Windows/Linux）を押す
      Then 検索バーが表示される

    Scenario: Escapeキーで検索バーを閉じる
      Given 検索バーが表示されている
      When Escapeキーを押す
      Then 検索バーが非表示になる
      And マッチのハイライトがすべて消える

  Rule: テキスト検索とマッチハイライト
    Scenario: 検索テキストに一致する箇所がハイライトされる
      Given 検索バーが表示されている
      When 検索テキストを入力する
      Then ファイル内容中の一致箇所がすべてハイライトされる
      And 現在のマッチ位置が強調表示される
      And マッチ件数が表示される（例: "1/5"）

    Scenario: 一致箇所がない場合
      Given 検索バーが表示されている
      When ファイル内容に存在しないテキストを入力する
      Then マッチ件数が"0"と表示される

  Rule: マッチ間のナビゲーション
    Scenario: 次のマッチに移動する
      Given 検索結果が複数ある
      When 次へボタンを押す（またはEnter）
      Then 次のマッチ位置に移動し、そのマッチが強調表示される

    Scenario: 前のマッチに移動する
      Given 検索結果が複数ある
      When 前へボタンを押す（またはShift+Enter）
      Then 前のマッチ位置に移動し、そのマッチが強調表示される

    Scenario: 最後のマッチから次へ移動すると最初に戻る
      Given 最後のマッチが強調表示されている
      When 次へボタンを押す
      Then 最初のマッチ位置に移動する
```

## 実装仕様

**対応方針**: ファイル内検索機能を実現するために、ShikiDiffViewerに検索バーUIと検索ハイライト機能を追加する。検索ロジック（テキストマッチ位置の算出）は、表示中コンテンツのUI上のハイライト処理であり、既にフロント側に展開済みのDiffLine.contentに対して実行するため、フロント側で実装する。

**対象コンポーネント**:
- `src/components/panels/ShikiDiffViewer.tsx`: 検索バーUI表示、マッチハイライト表示、Cmd+F/Escapeキーバインド処理
- `src/hooks/useDiffSearch.ts`（新規）: 検索状態管理（クエリ、マッチ一覧、現在位置）と検索ロジック
- `src/components/panels/DiffSearchBar.tsx`（新規）: 検索バーUIコンポーネント（入力欄、件数表示、前後ナビゲーション）

**検討した代替案**:
- Rust側でテキスト検索: 原則に沿うが、表示済みテキストの再送信によるラウンドトリップが発生し、キーストロークごとのレスポンスに影響する。却下。

**影響するテスト**:
- ユニットテスト: `useDiffSearch` フックのマッチ算出・ナビゲーションロジック
- コンポーネントテスト: `DiffSearchBar` の表示・操作テスト
- コンポーネントテスト: `ShikiDiffViewer` でCmd+F→検索バー表示→ハイライトの統合テスト
