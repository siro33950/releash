## 要求

**種別**: 改善
**ゴール**: 現在のMonaco DiffEditorおよびMonaco GutterEditorを廃止し、軽量な自前diff表示UIに置き換える。Gutter / Inline / Split の3つの表示モードを提供する
**背景**: Monaco Editorではhunk単位のstage/unstageやコメントUIの自由な配置など、レビュー機能に必要なカスタマイズが困難なため

**対応する表示モード**:
- **Gutter**: 変更後コードの行左端に追加/削除マーカーを表示（現`useMonacoGutterEditor`の置き換え）
- **Inline**: 追加行/削除行を1カラムで交互表示
- **Split**: 左右2カラムでbefore/afterを並べて表示

**制約**:
- diff hunk計算はRust側（git2）で行い、フロントはレンダリングに徹する
- シンタックスハイライトはShiki（`shiki/core` + `shiki/engine/javascript`、WASM不要）
- 言語は動的import（`@shikijs/langs/xxx`）、テーマはGitHub Dark系
- `codeToTokens()`で行・トークン単位の配列を取得し、自前HTMLにレンダリング
- hunk単位・行単位のインタラクション（stage/unstage、コメント）が可能なHTML構造にする

## 振る舞い定義

```gherkin
Feature: 自前Diff表示UI
  Monaco DiffEditor/GutterEditorを廃止し、Shikiベースの軽量な自前diff表示UIで
  Gutter / Inline / Split の3つの表示モードを提供する

  Background:
    Given ファイルにdiff（追加・削除・変更）が存在する

  Rule: 表示モードの切り替え
    Scenario: Gutterモードで表示する
      When ユーザーがGutterモードを選択する
      Then 変更後のコードが表示される
      And 追加行の左端に追加マーカーが表示される
      And 削除行の左端に削除マーカーが表示される

    Scenario: Inlineモードで表示する
      When ユーザーがInlineモードを選択する
      Then 削除行と追加行が1カラムで交互に表示される

    Scenario: Splitモードで表示する
      When ユーザーがSplitモードを選択する
      Then 左カラムに変更前のコードが表示される
      And 右カラムに変更後のコードが表示される

  Rule: シンタックスハイライト
    Scenario: ファイルの言語に応じたハイライトが適用される
      Given ファイルの拡張子から言語が判別できる
      When diff表示が描画される
      Then Shikiによるシンタックスハイライトが適用される

    Scenario: 言語が判別できないファイル
      Given ファイルの拡張子から言語が判別できない
      When diff表示が描画される
      Then プレーンテキストとして表示される

  Rule: Hunk単位のステージ操作による状態変化
    Scenario: ChangeGroupをステージする
      Given Unstaged変更のdiffを表示している
      When ユーザーがChangeGroupのステージ操作を行う
      Then そのChangeGroupがステージされる

    Scenario: ChangeGroupをアンステージする
      Given Staged変更のdiffを表示している
      When ユーザーがChangeGroupのアンステージ操作を行う
      Then そのChangeGroupがアンステージされる

  Rule: ステージ操作UIの表示
    Scenario: Unstaged変更表示時のステージボタン
      Given Unstaged変更のdiffを表示している
      When 各ChangeGroupが画面に表示されている
      Then 各ChangeGroupにステージボタンが表示される

    Scenario: Staged変更表示時のアンステージボタン
      Given Staged変更のdiffを表示している
      When 各ChangeGroupが画面に表示されている
      Then 各ChangeGroupにアンステージボタンが表示される

  Rule: Diff-onlyモードによる表示制御
    Scenario: Diff-onlyモードを有効にする
      When ユーザーがDiff-onlyモードを有効にする
      Then 変更がない領域が折りたたまれる
      And 変更箇所の前後にコンテキスト行が表示される

    Scenario: Diff-onlyモードを無効にする
      When ユーザーがDiff-onlyモードを無効にする
      Then ファイル全体が表示される
```

## 実装仕様

**対応方針**: 振る舞い定義（Gutter / Inline / Split の3モード、Shikiハイライト、ChangeGroup単位ステージ、Diff-onlyモード）を実現するために、`CodeDiffViewer`コンポーネントをMonaco依存からShiki + 自前HTMLレンダリングに置き換える。Rust側のdiff計算ロジック（hunk.rs）は変更なくそのまま活用する。

**対象コンポーネント**:
- **新規: `src/components/panels/ShikiDiffViewer.tsx`**: Shiki `codeToTokens()` でトークン化した行データを、Hunk情報と組み合わせて自前HTMLでレンダリング。Gutter / Inline / Split の3モードを1コンポーネントで提供
- **新規: `src/hooks/useShikiHighlighter.ts`**: Shikiハイライターインスタンスの初期化・キャッシュ管理。`createHighlighterCore` + `createJavaScriptRegexEngine`（WASM不要）で構築。言語は`@shikijs/langs/xxx`で動的import
- **新規: `src/hooks/useDiffTokens.ts`**: originalContent / modifiedContent をShikiでトークン化し、Hunk情報と結合して行ごとの表示データ（トークン配列 + 追加/削除/コンテキスト種別）を生成するフック
- **変更: `src/components/panels/CodeDiffViewer.tsx`**: `GutterDiffViewer` / `MonacoDiffViewer` を廃止し、`ShikiDiffViewer`に委譲。外部インターフェース（`CodeDiffViewerProps`）は維持
- **変更: `src/components/panels/DiffViewerSection.tsx`**: コードdiff時の描画先が変わるが、`CodeDiffViewer`のpropsが維持されるため最小限の変更
- **廃止: `src/hooks/useMonacoGutterEditor.ts`**: Monaco依存のGutter表示ロジックを廃止
- **廃止: `src/lib/monaco-config.ts`**: diff表示用のMonacoテーマ・設定。Monaco Editorがエディタ機能で引き続き使われる場合は残す
- **Rust側: 変更なし**: `hunk.rs`（`compute_diff_hunks`, `compute_hidden_ranges_from_content`, `generate_group_patch`）, `stage.rs`, `diff.rs`, `types.rs` はすべて現状のまま活用

**技術選定**:
- **Shiki（`shiki/core` + `shiki/engine/javascript`）**: WASM不要のJavaScript RegExpエンジンでシンタックスハイライト。`codeToTokens()`でトークン配列を取得し自前HTMLにレンダリング。Monaco Editorのビルトインハイライトからの移行先
- **言語ロード**: `@shikijs/langs/xxx` で動的import。拡張子→言語IDマッピングは既存の`get_language_from_path` Tauriコマンドを活用
- **テーマ**: GitHub Dark系（`github-dark`）

**検討した代替案**:
- **Monaco Editorのカスタマイズ続行**: hunk単位UIの自由度が不足、DOM直接操作によるStageボタンがMonacoレイアウト変更に脆弱。却下
- **highlight.js（既に依存に存在）**: 行・トークン単位の配列取得APIがなく、自前HTMLレンダリングとの統合が困難。却下

**リスク**:
- **Shikiの言語対応漏れ**: `createJavaScriptRegexEngine`はNode.js 20+のRegExp `v`フラグが最適。Tauriアプリ内のWebView環境で`v`フラグ非対応の場合は`u`フラグへのフォールバックが自動的に行われる
- **パフォーマンス**: 大ファイル（数千行）でのトークン化コスト → Diff-onlyモード時は表示範囲のみトークン化する最適化を検討

**影響するテスト**:
- **フロントエンド単体テスト**: `ShikiDiffViewer`の3モード切り替え、ステージボタン表示、Diff-onlyモードのテスト追加。`useShikiHighlighter`のモック方針定義
- **既存テスト修正**: `CodeDiffViewer`関連のテストがMonaco依存であれば修正
- **Rustテスト**: 変更なし（hunk.rsの既存テストはそのまま）
- **統合テスト（Playwright）**: diff表示のスクリーンショットテストがある場合は更新
