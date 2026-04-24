## 要求

**種別**: バグ修正
**現在の挙動**: ReviewPanelのDiffFileTreeで削除済みファイルを選択すると、DiffViewerSectionが空表示になる（ファイル自体は一覧に表示される）
**期待する挙動**: 削除済みファイルを選択したとき、削除された全行が赤（削除行）としてDiff表示される
**再現手順**:
1. ファイルを削除する（git rmまたはファイルシステムから削除）
2. ReviewPanelのDiffFileTreeで削除済みファイルを選択する
3. DiffViewerSectionが空で表示される（差分内容が表示されない）
**背景**: 削除済みファイルの差分を確認できないと、何が削除されたかをレビューできない

## 振る舞い定義

```gherkin
Feature: 削除済みファイルの差分表示
  削除されたファイルの差分をReviewPanelで確認できる

  Rule: 削除済みファイルの差分は削除前の全行が削除行として算出される
    Scenario: ワーキングツリーから削除されたファイルの差分取得
      Given ファイルがHEADに存在する
      And ワーキングツリーからファイルが削除されている
      When 削除済みファイルの差分を取得する
      Then 削除前の全行が削除行として差分に含まれる

    Scenario: ステージで削除されたファイルの差分取得
      Given ファイルがHEADに存在する
      And ステージでファイルが削除されている
      When 削除済みファイルの差分を取得する
      Then 削除前の全行が削除行として差分に含まれる

  Rule: 削除行を含む差分は赤色の削除行としてDiffビューに表示される
    Scenario: 削除済みファイルのDiff表示
      Given 削除済みファイルの差分が存在する
      When DiffFileTreeで削除済みファイルを選択する
      Then DiffViewerSectionに全行が削除行（赤）として表示される
```

## 実装仕様

**対応方針**: 削除済みファイルの差分が空表示になるバグを修正するために、`useFileDiffContent`のコンテンツ取得ロジックを改修し、削除済みファイルの場合にHEADから元の内容を取得できるようにする。

**対象コンポーネント**:
- `src/hooks/useFileDiffContent.ts`: 削除済みファイルの場合にフォールバックでHEADから内容を取得するロジックを追加
  - Changesセクション(Staged→WorkingTree)で、`get_staged_content`が失敗（ステージに無い）かつ`readTextFile`が失敗（ワーキングツリーに無い）の場合、originalContentとしてHEAD(`get_file_at_ref`)から取得する
  - Staged Changesセクション(HEAD→Staged)では、`get_staged_content`が失敗した場合、modifiedContentを空文字とする（現状通り。削除=空文字は正しい）

**検討した代替案**:
- Rust側で`get_staged_content`を修正し、ステージ削除時にHEADのコンテンツを返す案 → ステージ内容の取得という責務に反するため却下
- `computeDiffBlocks`で両方空の場合に特別処理する案 → データ取得側で正しい値を返すべきであり、表示側での回避策は根本解決にならないため却下

**影響するテスト**:
- `src/hooks/useFileDiffContent.test.ts`: 削除済みファイルケースのテスト追加（存在する場合は修正、無い場合は新規作成）
