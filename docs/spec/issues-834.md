# Issue #834: ワークスペースを指定のエディタで開くボタンを用意したい

## 要求

**種別**: 改善
**ゴール**: Reviewパネルのヘッダーにある「Send all comments」ボタンを削除し、代わりにワークツリーのディレクトリを外部エディタ（VSCode/Cursor等）でフォルダごと開くボタンを配置する
**背景**: レビュー中に該当ワークツリーを外部エディタで開いて確認・編集したい場面があるが、現在は手動でエディタからフォルダを開く必要がある。Reviewパネルから直接外部エディタを起動できるようにすることで、レビューワークフローの効率を向上させる
**制約**: 既存の外部エディタ検出・設定機能（`detect_editors` / `external_editor` config）を活用する。ファイル単位ではなくフォルダ単位で開く
**影響範囲**: ReviewPanelヘッダーのUI変更（Send all commentsボタンの削除・差し替え）

## 振る舞い定義

```gherkin
Feature: ワークスペースを外部エディタで開く
  Reviewパネルから現在のワークツリーを外部エディタでフォルダごと開く

  Rule: エディタ起動の状態遷移
    Scenario: 外部エディタが設定済みの場合、ワークツリーを設定済みエディタで開く
      Given 外部エディタが設定されている
      And Reviewパネルが表示されている
      When ユーザーが「エディタで開く」ボタンを押す
      Then 設定済みのエディタでワークツリーのディレクトリが開かれる

    Scenario: 外部エディタが未設定の場合、システムデフォルトで開く
      Given 外部エディタが設定されていない
      And Reviewパネルが表示されている
      When ユーザーが「エディタで開く」ボタンを押す
      Then システムデフォルトのアプリケーションでワークツリーのディレクトリが開かれる

  Rule: ボタンの表示
    Scenario: Reviewパネルのヘッダーにエディタで開くボタンが表示される
      Given Reviewパネルが表示されている
      When ユーザーがヘッダーを確認する
      Then 「Send all comments」ボタンの代わりに「エディタで開く」ボタンが表示されている
```

## 実装仕様

**対応方針**: ReviewパネルヘッダーのSend all commentsボタンを「エディタで開く」ボタンに差し替える。ワークツリーのディレクトリを外部エディタで開くために、macOSの`open -a 'エディタ.app' /path`コマンドを使用する新規Tauriコマンドを追加する。

**対象コンポーネント**:
- `src-tauri/src/external_editor.rs`:
  - 新規Tauriコマンド `open_folder_in_editor` を追加
  - `std::process::Command`で `open -a <editor_path> <folder_path>` を実行
  - エディタ未設定時は `open <folder_path>`（Finderが開く）にフォールバック
- `src-tauri/src/lib.rs`: 新コマンドを登録
- `src/components/panels/ReviewPanel.tsx`:
  - ヘッダーのSend all commentsボタン（497-526行）を「エディタで開く」ボタンに差し替え
  - `invoke("open_folder_in_editor", { folderPath: rootPath })`で呼び出し
  - 不要になるもの削除:
    - `Send` アイコンのimport
    - `useDiffComments`からの `unsentCount`, `sendAllUnsent` のdestructuring
  - 維持するもの:
    - `onSendToAgent` prop（インラインコメント送信で使用）
    - `useDiffComments`フック自体（コメント機能は維持）

**影響するテスト**:
- `src-tauri/src/external_editor.rs`: `open_folder_in_editor`のユニットテスト追加（コマンド構築のテスト）
- ReviewPanelのテスト（存在する場合）: Send all commentsボタン関連を更新
