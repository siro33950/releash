## 要求

**種別**: バグ修正
**現在の挙動**: Reviewパネルでブラウザデフォルトのスクロールバーが表示される
**期待する挙動**: デフォルトではなくアプリのデザインに合ったカスタムスクロールバーが表示される
**背景**: デフォルトのスクロールバーがアプリのUIデザインと統一されておらず、見た目を損なっている

## 振る舞い定義

```gherkin
Feature: Reviewパネルのスクロールバー表示
  Reviewパネル内のスクロール可能な領域で、アプリのデザインに統一されたカスタムスクロールバーが表示される

  Rule: Reviewパネルのスクロールバーはアプリ内の他パネルと統一されたデザインで表示される
    Scenario: Reviewパネルのファイルツリー領域にカスタムスクロールバーが表示される
      Given Reviewパネルが表示されている
      When ファイルツリー領域の内容がスクロール可能な量である
      Then アプリのデザインに統一された細いスクロールバーが表示される

    Scenario: Reviewパネルのdiff表示領域にカスタムスクロールバーが表示される
      Given Reviewパネルが表示されている
      When diff表示領域の内容がスクロール可能な量である
      Then アプリのデザインに統一された細いスクロールバーが表示される
```

## 実装仕様

**対応方針**: Reviewパネル内のすべてのスクロール可能領域を、`overflow-auto` / `overflow-y-auto` のdivから shadcn/ui の `ScrollArea` コンポーネントに置き換えることで、アプリ全体で統一されたカスタムスクロールバーを実現する。

**対象コンポーネント**:
- **DiffFileTree.tsx**: 3箇所
  - Unstagedセクションのツリー領域（`overflow-y-auto` → `ScrollArea`）
  - Stagedセクションのツリー領域（`overflow-y-auto` → `ScrollArea`）
  - BranchBaseTreeの全体（`overflow-y-auto` → `ScrollArea`）
- **ReviewPanel.tsx**: 1箇所
  - diff表示領域（`overflow-auto` → `ScrollArea`）
- **MarkdownDiffViewer.tsx**: 2箇所
  - GutterView（`overflow-auto scrollbar-thin` → `ScrollArea`、`scrollbar-thin`クラス除去）
  - InlineView（`overflow-auto scrollbar-thin` → `ScrollArea`、`scrollbar-thin`クラス除去）
- **ImageDiffViewer.tsx**: 1箇所
  - 各ImagePaneの画像コンテナ（`overflow-auto` → `ScrollArea`）

**技術選定**:
- `ScrollArea`（shadcn/ui / Radix UI）: 既にプロジェクト内に `src/components/ui/scroll-area.tsx` として存在。追加インストール不要

**リスク**:
- ScrollAreaのViewportが `size-full` のため、親要素に明示的な高さ制約が必要。現在の `flex-1 min-h-0` 構造で各箇所確認が必要
- MarkdownDiffViewerのSplitView（`md-split-container scrollbar-thin`）はCSS Grid/Flexレイアウト上の `scrollbar-thin` であり、ScrollArea置き換え時にレイアウトとの相互作用を検証する必要がある

**影響するテスト**:
- 既存テストでScrollAreaのモックが必要になる可能性あり（Radix UIのResizeObserver等がjsdomで動作しない場合）
