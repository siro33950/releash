## 要求

**種別**: 改善
**ゴール**: 差分ファイルビューで、変更のあるHunk周辺のみを表示し、変更のない行を折りたたむ（省略する）「差分のみ表示」モードを追加する。全文表示と差分のみ表示を切り替え可能にする。
**背景**: 現在のgutterモードではファイル全文を表示しつつ変更箇所にマーカーを付けるが、大きなファイルでは変更箇所を見つけにくい。差分のみ表示にすることで、レビュー時に変更箇所に集中できる。
**影響範囲**: 右パネルの差分表示（CodeDiffViewer / GutterDiffViewer）、DiffToolbar（切り替えUI）

## 振る舞い定義

```gherkin
Feature: 差分のみ表示モード
  差分ファイルビューで、変更のあるHunk周辺のみを表示し、
  変更のない行を折りたたむ（省略する）モードを提供する。
  全文表示と差分のみ表示を切り替え可能にする。

  Rule: 差分のみ表示モードの切り替え
    Scenario: 全文表示から差分のみ表示に切り替える
      Given 差分ファイルビューでファイルが表示されている
      And 全文表示モードである
      When ユーザーが差分のみ表示モードに切り替える
      Then 変更のあるHunk周辺の行のみが表示される
      And 変更のない行は折りたたまれる

    Scenario: 差分のみ表示から全文表示に切り替える
      Given 差分ファイルビューでファイルが表示されている
      And 差分のみ表示モードである
      When ユーザーが全文表示モードに切り替える
      Then ファイルの全行が表示される

  Rule: 差分のみ表示における折りたたみ表示
    Scenario: 変更がないファイルを差分のみ表示で見る
      Given ファイルに変更がない
      When 差分のみ表示モードで表示する
      Then 全ての行が折りたたまれる
      And 変更がないことがわかる表示になる

    Scenario: 複数のHunkがあるファイルを差分のみ表示で見る
      Given ファイルに離れた位置に複数のHunkがある
      When 差分のみ表示モードで表示する
      Then 各Hunkの変更行と周辺のコンテキスト行が表示される
      And Hunk間の変更のない行は折りたたまれる
      And 折りたたまれた箇所には省略されている行数が表示される

  Rule: 折りたたまれた行の展開
    Scenario: 折りたたまれた箇所を個別に展開する
      Given 差分のみ表示モードで折りたたまれた箇所がある
      When ユーザーが折りたたまれた箇所をクリックする
      Then その箇所の省略されていた行が展開される
      And 他の折りたたまれた箇所はそのまま維持される

    Scenario: 全ての折りたたみを一括展開する
      Given 差分のみ表示モードで折りたたまれた箇所がある
      When ユーザーが全展開ボタンを押す
      Then 全ての折りたたまれた箇所が展開される

  Rule: 差分のみ表示モードの適用範囲
    Scenario: 全ての差分表示モード（gutter/inline/split）で差分のみ表示を使う
      Given いずれかの差分表示モード（gutter, inline, split）が選択されている
      When 差分のみ表示を有効にする
      Then 選択中の差分表示モードに関わらず差分のみ表示が適用される

  Rule: デフォルト設定と保持
    Scenario: 初回起動時のデフォルト
      Given ユーザーが差分のみ表示の設定を変更したことがない
      When 差分ファイルビューを開く
      Then 全文表示モードで表示される

    Scenario: 設定画面からデフォルトを変更する
      Given 設定画面を開いている
      When デフォルトの差分表示を「差分のみ表示」に変更する
      Then 以降新しく開く差分ファイルビューは差分のみ表示モードで表示される

    Scenario: 差分のみ表示の設定がWorkspaceごとに保持される
      Given Workspace Aで差分のみ表示モードに切り替えた
      When Workspace Bに移動する
      Then Workspace Bでは独自の設定が適用される

    Scenario: 同一Workspace内でファイルを切り替えても設定が維持される
      Given 差分のみ表示モードに切り替えた
      When 別のファイルを選択する
      Then 差分のみ表示モードが維持される
```

## 実装仕様

**対応方針**: 振る舞い定義（差分のみ表示モード）を実現するために、Monaco DiffEditorのビルトイン `hideUnchangedRegions` APIを活用し（inline/split）、GutterモードにはHunkデータベースの独自折りたたみ（setHiddenAreas + ViewZone）を実装する。

**対象コンポーネント**:

Rust（ロジック）:
- `src-tauri/src/git/hunk.rs`: `compute_hidden_ranges(hunks: Vec<Hunk>, total_lines: u32, context_lines: u32) -> Vec<HiddenRange>` を追加。Hunkデータからコンテキスト行（前後N行）外の非表示範囲を算出する
- `src-tauri/src/git/hunk.rs`: `compute_visible_markdown_blocks(original: &str, modified: &str, context_lines: u32) -> Vec<VisibleBlock>` を追加。Markdown差分で変更のあるブロックとコンテキスト行のみを抽出する
- `src-tauri/src/git/types.rs`: `HiddenRange { start_line: u32, end_line: u32, hidden_count: u32 }` と `VisibleBlock { start_line: u32, end_line: u32, content: String }` を追加
- `src-tauri/src/git/commands.rs`: 上記をTauriコマンドとして公開

フロントエンド（UI）:
- `src/types/settings.ts`: `AppSettings`に`defaultDiffOnlyMode: boolean`を追加（デフォルト: `false`）
- `src/hooks/useSettings.ts`: `updateDefaultDiffOnlyMode`アクセサを追加
- `src/screens/useWorktreeState.tsx`: Workspaceごとの`diffOnlyMode: boolean`状態と`setDiffOnlyMode`を管理
- `src/components/panels/DiffToolbar.tsx`: 差分のみ表示トグルボタンを追加（diff mode切り替えの左隣）
- `src/components/panels/CodeDiffViewer.tsx`:
  - **MonacoDiffViewer**: `hideUnchangedRegions`オプションを`diffOnlyMode`連動で`updateOptions`切り替え（`contextLineCount: 3`, `minimumLineCount: 3`, `revealLineCount: 20`）
  - **GutterDiffViewer**: `invoke("compute_hidden_ranges_from_content")`で取得した結果を`editor.setHiddenAreas()`に渡して非表示化 + ViewZoneで「… N lines hidden」バナーを表示。バナークリックで個別展開
- `src/components/panels/MarkdownDiffViewer.tsx`: `invoke("compute_visible_markdown_blocks")`で取得した可視ブロックのみを表示
- `src/components/panels/ReviewPanel.tsx`: `diffOnlyMode`状態を`DiffToolbar`・`DiffViewerSection`に伝搬

**技術選定**:
- inline/splitモード: Monaco DiffEditor `hideUnchangedRegions` API（ビルトイン機能。折りたたみ表示・個別展開・省略行数表示を標準提供）
- gutterモード: Rust側で非表示範囲を算出 → フロントは`ICodeEditor.setHiddenAreas()` + `ViewZone`で表示制御のみ（StandaloneEditorにはDiffEditor機能がないため独自実装が必要）
- markdownモード: Rust側で可視ブロックを算出 → フロントは受け取ったブロックを描画するのみ

**検討した代替案**:
- GutterモードでもDiffEditorに切り替える案: Gutterモード固有機能（コメントスレッド、Stage overlay、glyphMargin装飾）がDiffEditorでは動作しないため却下
- 全モード共通でHunk計算ベースの独自折りたたみ: inline/splitはMonaco DiffEditorのビルトインの方がUX（アニメーション、省略行数表示、展開UI）が優れるため却下

**リスク**:
- `setHiddenAreas`はMonaco公式APIとして`ICodeEditor`に存在するが、ドキュメントが薄い → 実際にVSCodeで広く使われているため低リスク

**影響するテスト**:
- Rust: `hunk.rs`内に`compute_hidden_ranges`・`compute_visible_markdown_blocks`の単体テスト（正常系・境界値・変更なし・全行変更）
- フロントエンド単体テスト: `CodeDiffViewer.test.tsx`（diffOnlyMode propsの受け渡し）、`DiffToolbar.test.tsx`（トグルボタンのクリックイベント）
- フック: `useReviewPanel`のdiffOnlyMode状態管理テスト
