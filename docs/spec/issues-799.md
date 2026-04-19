## 要求

**種別**: 新機能
**ゴール**: エディタタブの上にパン屑リスト（Breadcrumb）を表示する。開いているファイルのパスを階層的に表示し（例: `src > components > panels > FileTree.tsx`）、現在のファイル位置を視覚的に把握できるようにする。クリック動作は不要で、表示のみ。
**背景**: 現在はエディタでファイルを開いた際、そのファイルがどのディレクトリに属しているか一目で把握しづらい。パン屑リストを設置することで、ファイルの階層的な位置関係を即座に確認できるようにする。

## 振る舞い定義

```gherkin
Feature: Reviewパネルのパン屑リスト
  Reviewパネルのdiffビューアでファイルを選択した際、ファイルの階層的な位置をパン屑リストで表示する

  Rule: ファイル選択時にパン屑リストが表示される
    Scenario: ファイルを選択するとパン屑リストが表示される
      Given Reviewパネルが表示されている
      When ファイルツリーからファイルを選択する
      Then diffビューアの上部にそのファイルのパス階層がパン屑リストとして表示される

    Scenario: ファイル未選択時はパン屑リストが表示されない
      Given Reviewパネルが表示されている
      When ファイルが選択されていない
      Then パン屑リストは表示されない

  Rule: パン屑リストはファイルパスを階層的に表示する
    Scenario: パン屑リストの表示内容
      Given ファイル "src/components/panels/FileTree.tsx" が選択されている
      When ユーザーがパン屑リストを確認する
      Then "src > components > panels > FileTree.tsx" の形式で階層表示される
      And 各フォルダにはフォルダアイコンが表示される
      And ファイル名にはファイルアイコンが表示される
```

## 実装仕様

**対応方針**: Reviewパネルのdiffビューア上部にパン屑リストを表示する。パス分割ロジックはRust側にTauriコマンドとして実装し、フロントはセグメント配列を受け取って表示するだけにする。

**対象コンポーネント**:
- `src-tauri/src/git/` 配下: パスをセグメント配列に分割するTauriコマンド `parse_breadcrumb_segments` を追加
  - 入力: `root_path: String`, `file_path: String`
  - 出力: `Vec<BreadcrumbSegment>` （`name: String`, `is_file: bool`）
  - ロジック: rootPath正規化、配下判定、相対パス算出、`/`で分割、最後のセグメントをファイルとして判定
- `src/components/panels/Breadcrumb.tsx`: ロジックを除去し、セグメント配列を受け取って表示するだけに変更
  - propsを `{ segments: BreadcrumbSegment[]; children?: ReactNode }` に変更
  - パス分割・相対パス算出のロジックを除去
  - 呼び出し側がRustコマンドでセグメントを取得してから渡す
- `src/components/panels/ReviewPanel.tsx`: `DiffViewerSection` の上に `Breadcrumb` を配置

**技術選定**:
- 新規ライブラリの導入は不要
- アイコンは既存の `@react-symbols/icons`（FileIcon / FolderIcon）と `lucide-react`（ChevronRight）を使用

**検討した代替案**:
- フロントエンド側でパス分割を行う案 → Rust-firstルール違反のため却下

**影響するテスト**:
- Rust: `parse_breadcrumb_segments` の単体テスト（正常系・エラー系・境界値）
- `Breadcrumb.test.tsx`: propsの変更に合わせて更新
- `ReviewPanel` のテスト: パン屑リスト表示の統合テストを追加
