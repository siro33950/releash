## 要求

**種別**: 新機能
**ゴール**: Diffビューに「エディタで開く」ボタンを追加し、Diffビューからエディタへスムーズに遷移できるようにする
**背景**: Diffビューとエディタ間のナビゲーションを改善し、変更確認から編集へのワークフローをスムーズにする

## 振る舞い定義

```gherkin
Feature: Diffビューから外部エディタで開く
  Diffビューで確認中のファイルを外部エディタで開き、変更確認から編集へスムーズに遷移する

  Rule: Diffビューで選択中のファイルを外部エディタで開くことができる
    Scenario: Diffビューから外部エディタで開く
      Given Diffビューでファイルが選択されている
      When ユーザーが「エディタで開く」を実行する
      Then 選択中のファイルが設定された外部エディタで開かれる

  Rule: デフォルトではシステムデフォルトのアプリケーションでファイルを開く
    Scenario: エディタ未変更時はシステムデフォルトで開く
      Given 外部エディタの設定がデフォルトのままである
      When ファイルを外部エディタで開く
      Then システムデフォルトのアプリケーションでファイルが開かれる

  Rule: 設定画面でデフォルトエディタを変更できる
    Scenario: デフォルトエディタの変更
      Given 利用可能なエディタが検出されている
      When ユーザーが設定画面でエディタを選択する
      Then 選択されたエディタがデフォルトとして保存される

    Scenario: 変更後のエディタで開く
      Given デフォルトエディタが変更されている
      When ファイルを外部エディタで開く
      Then 変更後のエディタでファイルが開かれる

  Rule: 利用可能な外部エディタを検出できる
    Scenario: インストール済みエディタの検出
      Given システムにエディタアプリがインストールされている
      When 利用可能なエディタを検出する
      Then インストール済みのエディタが一覧として返される
```

## 実装仕様

**対応方針**: Diffビューから外部エディタでファイルを開く機能を実現するために、Rust側に外部エディタ検出・設定管理のロジックを実装し、フロントエンドのReviewPanelにボタンを追加する。ファイルオープンには既存の`tauri_plugin_opener`の`open_path`を使用する。

**対象コンポーネント**:
- `src-tauri/src/external_editor.rs`（新規）: エディタ検出ロジック（macOS: `/Applications`および`~/Applications`のアプリバンドル検索、スキャンディレクトリは注入可能）、`open_in_editor` Tauriコマンド（`tauri_plugin_opener::open_path`で`with`にエディタパスを渡す）
- `src-tauri/src/config.rs`: `AppSection`に`external_editor: String`フィールド追加（デフォルト空=システムデフォルト）
- `src-tauri/src/lib.rs`: 新規コマンド登録（`detect_editors`, `open_in_editor`）
- `src/types/settings.ts`: `AppSettings`に`externalEditor: string`追加
- `src/components/panels/ReviewPanel.tsx`: ヘッダー部にDiffBaseToggle横に「エディタで開く」ボタン追加
- `src/components/panels/SettingsModal.tsx`: Editorタブに外部エディタ選択ドロップダウン追加

**技術選定**:
- `tauri_plugin_opener::open_path(with)`: 既にプロジェクトに導入済み。`with`パラメータでエディタ実行パスを指定可能。新規依存なし

**検討した代替案**:
- `std::process::Command`で直接エディタを起動: エディタごとの起動引数の差異を吸収する必要がありメンテコストが高い。`open_path`は各OSのデフォルトオープン機構を利用するため安定性が高い → 却下

**影響するテスト**:
- Rust単体テスト: `external_editor.rs`のエディタ検出ロジック（`scan_applications_in`にTempDirを注入した決定的テスト）
- フロントエンドテスト: ReviewPanelのボタン表示・クリック時のinvoke呼び出し確認
- 設定テスト: `AppSettings`のexternalEditorフィールドの永続化テスト（既存`app_section_roundtrip`テストの拡張）
