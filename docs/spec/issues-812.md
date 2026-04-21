## 要求

**種別**: リファクタリング
**ゴール**: monaco-editor パッケージへの依存を完全に除去し、バンドルサイズを削減する
**背景**: エディタ全般を廃止するマイルストーン方針に基づく。現在のMonaco依存箇所は全てデッドコード（どのコンポーネントからも参照されていない）であり、安全に削除できる
**影響範囲**:
- パッケージ: `@monaco-editor/react`、`monaco-editor` の削除
- デッドコード削除対象:
  - `src/hooks/useMonacoEditor.ts` + テスト
  - `src/hooks/useMonacoGutterEditor.ts` + テスト
  - `src/lib/monaco-config.ts` + テスト
  - `src/lib/commentThreadWidget.ts` + テスト
  - `src/test/setup.ts` 内のMonacoモック定義
  - `src/index.css` 内のMonaco固有CSSクラス
- 機能への影響: なし（全て未使用コード）
- バンドルサイズの削減効果を計測する

## 振る舞い定義

```gherkin
Feature: Monaco Editor パッケージ依存の除去
  エディタ廃止方針に基づき、monaco-editor パッケージへの依存を完全に除去し、
  バンドルサイズを削減する

  Rule: Monaco関連のデッドコードが除去されている
    Scenario: Monaco関連ファイルが存在しない
      Given monaco-editor 依存の除去が完了している
      When プロジェクトのソースツリーを確認する
      Then 以下のファイルが存在しない:
        | ファイル                                    |
        | src/hooks/useMonacoEditor.ts                |
        | src/hooks/useMonacoEditor.test.ts           |
        | src/hooks/useMonacoGutterEditor.ts          |
        | src/hooks/useMonacoGutterEditor.test.ts     |
        | src/lib/monaco-config.ts                    |
        | src/lib/__tests__/monaco-config.test.ts     |
        | src/lib/commentThreadWidget.ts              |
        | src/lib/commentThreadWidget.test.ts         |

    Scenario: テストセットアップからMonacoモックが除去されている
      Given monaco-editor 依存の除去が完了している
      When src/test/setup.ts を確認する
      Then Monaco Editor のモック定義が存在しない

    Scenario: CSSからMonaco固有スタイルが除去されている
      Given monaco-editor 依存の除去が完了している
      When src/index.css を確認する
      Then Monaco固有のCSSクラス定義が存在しない

  Rule: パッケージ依存が除去されている
    Scenario: package.json からMonacoパッケージが除去されている
      Given monaco-editor 依存の除去が完了している
      When package.json を確認する
      Then "@monaco-editor/react" が dependencies に存在しない
      And "monaco-editor" が dependencies に存在しない

  Rule: 既存機能が影響を受けない
    Scenario: ビルドが成功する
      Given monaco-editor 依存の除去が完了している
      When プロジェクトをビルドする
      Then ビルドがエラーなく完了する

    Scenario: 既存テストが通る
      Given monaco-editor 依存の除去が完了している
      When テストスイートを実行する
      Then Monaco関連以外の全テストがパスする

  Rule: バンドルサイズが削減される
    Scenario: 削除前後でバンドルサイズの削減を確認できる
      Given 削除前のバンドルサイズを計測済みである
      When monaco-editor 依存の除去後にビルドする
      Then バンドルサイズが削除前より削減されている
```

## 実装仕様

**対応方針**: 振る舞い定義を実現するために、Monaco関連のデッドコード・CSS・テストモック・パッケージ依存を全て削除する。全てデッドコードであるため、削除のみで既存機能に影響なし。

**対象コンポーネント**:

| # | 対象 | 変更内容 |
|---|------|---------|
| 1 | `src/hooks/useMonacoEditor.ts` + `.test.ts` | ファイル削除 |
| 2 | `src/hooks/useMonacoGutterEditor.ts` + `.test.ts` | ファイル削除 |
| 3 | `src/lib/monaco-config.ts` + `__tests__/monaco-config.test.ts` | ファイル削除 |
| 4 | `src/lib/commentThreadWidget.ts` + `.test.ts` | ファイル削除 |
| 5 | `src/test/setup.ts` | Monacoモック定義（`mockEditor`, `mockTextModel`, `mockDiffEditor`, `mockMonaco`, `MockRange`, `loader.init()` mock）を削除 |
| 6 | `src/index.css` | 244行目〜668行目のMonaco/Gutter/Hunk/Comment関連CSS全体を削除 |
| 7 | `tests/helpers/screenshot-utils.ts` | `monacoMask()` 関数を削除。呼び出し元（`worktree-dialogs.screenshot.ts`, `editor-layout.screenshot.ts`）からも参照を削除 |
| 8 | `package.json` | `@monaco-editor/react`, `monaco-editor` の依存を削除 |
| 9 | `pnpm-lock.yaml` | `pnpm install` で自動更新 |

**実装順序**:
1. 削除前のバンドルサイズ計測（`pnpm build` → distサイズ記録）
2. ファイル削除（#1〜#4: 8ファイル）
3. `src/test/setup.ts` からモック定義削除（#5）
4. `src/index.css` からCSS削除（#6）
5. `tests/helpers/screenshot-utils.ts` と呼び出し元修正（#7）
6. `package.json` から依存削除 + `pnpm install`（#8, #9）
7. 品質ゲート: `pnpm lint` → `pnpm test` → `pnpm build`
8. 削除後のバンドルサイズ計測・比較

**影響するテスト**:
- 削除対象テスト: 4ファイル（`useMonacoEditor.test.ts`, `useMonacoGutterEditor.test.ts`, `monaco-config.test.ts`, `commentThreadWidget.test.ts`）
- 修正対象テスト: `src/test/setup.ts`（モック除去）、screenshot-utils関連2ファイル（`monacoMask`参照削除）
- 既存テスト: Monaco以外は全てパスすること
