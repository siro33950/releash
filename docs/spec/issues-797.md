## 要求

**種別**: 新機能
**ゴール**: Diffレビュービューのフッターに「前のファイル」「次のファイル」へ移動するUIを追加し、変更ファイル間をフッターから順番にナビゲートできるようにする
**背景**: Diffレビュー時に複数の変更ファイルを順番に確認する際、現状ではファイルツリーに戻って次のファイルを選択する必要がある。フッターにナビゲーションUIを追加することで、レビューの効率を向上させる

## 振る舞い定義

```gherkin
Feature: Diffレビューのファイル間ナビゲーション
  Diffレビュー中にフッターから前後のファイルへ移動できる

  Rule: ナビゲーション対象はセクションを横断した全変更ファイル
    Scenario: 次のファイルへ移動する
      Given 複数の変更ファイルが存在する（Staged/Unstaged問わず）
      And あるファイルのdiffを表示している
      And 現在のファイルが全体の最後ではない
      When 「次のファイル」で移動する
      Then 全変更ファイルのツリー表示順で次のファイルが選択される
      And 移動先ファイルが属するセクション（Staged/Changes）のdiffが表示される

    Scenario: 前のファイルへ移動する
      Given 複数の変更ファイルが存在する（Staged/Unstaged問わず）
      And あるファイルのdiffを表示している
      And 現在のファイルが全体の最初ではない
      When 「前のファイル」で移動する
      Then 全変更ファイルのツリー表示順で前のファイルが選択される
      And 移動先ファイルが属するセクション（Staged/Changes）のdiffが表示される

    Scenario: 最後のファイルで次へ移動できない
      Given 現在のファイルが全変更ファイルの最後である
      When 「次のファイル」のUIを確認する
      Then 「次のファイル」は無効化されている

    Scenario: 最初のファイルで前へ移動できない
      Given 現在のファイルが全変更ファイルの最初である
      When 「前のファイル」のUIを確認する
      Then 「前のファイル」は無効化されている

  Rule: Stage/Unstage操作後もナビゲーションリストに含まれ続ける
    Scenario: 表示中のファイルをStageした場合
      Given Changesセクションのファイルを表示している
      When そのファイルをStageする
      Then 表示中のファイルはそのまま維持される
      And ナビゲーションリストにそのファイルは引き続き含まれる

    Scenario: 別のファイルがStageされた場合
      Given あるファイルのdiffを表示している
      When 他のファイルがStageされる
      Then ナビゲーションリストにStageされたファイルは引き続き含まれる

  Rule: ナビゲーションUIはフッターに表示される
    Scenario: フッターにファイルナビゲーションが表示される
      Given 変更ファイルが存在する
      When Diffレビュービューを表示する
      Then フッターに「前のファイル」「次のファイル」のナビゲーションUIが表示される
      And 全変更ファイル中の現在のファイル位置が表示される

    Scenario: 変更ファイルが1件のみの場合
      Given 変更ファイルが1件のみ存在する
      When Diffレビュービューを表示する
      Then 「前のファイル」「次のファイル」はどちらも無効化されている
```

## 実装仕様

**対応方針**: 振る舞い定義（全変更ファイル横断ナビゲーション）を実現するために、Rustのファイルナビゲーションコマンド + DiffToolbar拡張で対応する。

**対象コンポーネント**:
- `src-tauri/src/git/diff_tree.rs`: `get_file_navigation` Tauriコマンドを追加。DiffTreeNodeの階層ツリーと現在のファイルパスを受け取り、`{ current_index, total, prev_file, next_file }` を返す。ツリーのフラット化・インデックス算出・前後ファイル決定・境界判定を全てRust側で行う
- `src/hooks/useFileNavigation.ts`（新規）: Rustコマンドの呼び出しと結果の保持のみ。ロジックは持たない
- `src/components/panels/DiffToolbar.tsx`: ファイルナビゲーションUIを左側に追加（既存のhunkナビゲーションは中央に維持）
- `src/components/panels/ReviewPanel.tsx`: useFileNavigationの統合、DiffToolbarへのprops追加

**設計判断**:
- ナビゲーションロジックは全てRust側で行う: ツリーのフラット化・現在位置の算出・前後ファイルの決定・境界判定を`get_file_navigation`コマンドに集約。フロントエンドはコマンド呼び出しと結果の表示のみ
- ナビゲーションは循環しない: 仕様「最初のファイルで前へ移動できない」「最後のファイルで次へ移動できない」に従い、非循環方式を採用。`prev_file`/`next_file`は境界時に`None`を返す
- ナビゲーションスコープはセクション横断: HEADモードではStagedツリーとChangesツリーを結合した全変更ファイルリストを使用。重複ファイル（両セクションに存在する場合）は最初の出現のみカウント。移動先ファイルが属するセクションを自動判定してdiffを表示する
- Branch Baseモード対応: Branch BaseモードではbranchBaseTreeをナビゲーション対象とする

**影響するテスト**:
- `src-tauri/src/git/diff_tree.rs`内 `#[cfg(test)] mod tests`: ナビゲーションロジックの単体テスト（空ツリー、単一ファイル、境界条件、ネストフォルダ）
- `src/hooks/useFileNavigation.test.ts`（新規）: invokeの呼び出しと結果保持のテスト
- `src/components/panels/DiffToolbar.test.tsx`: ファイルナビゲーションUI表示・無効化条件テスト
