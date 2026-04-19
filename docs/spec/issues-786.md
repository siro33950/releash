## 要求

**種別**: 改善
**ゴール**: 右パネル下部にコメント一覧とターミナルを並列配置し、レビュー画面からコメント・ターミナル機能にアクセスできるようにする
**背景**: #784（旧UI廃止）でコメントとターミナルの既存配置先が廃止されるため、新しいUI配置先として右パネル下部に統合する
**影響範囲**:
- 既存の `CommentThread.tsx`, `CommentList.tsx` を流用
- 既存の `TerminalPanel.tsx`, `RightSidebarBottom.tsx` を流用
- Rust側コマンド（Thread/Comment系、PTY系）は変更なし

## 振る舞い定義

```gherkin
Feature: 右パネル下部のコメント・ターミナル分割配置
  レビュー画面の右パネル下部にコメント一覧とターミナルを左右に並列配置して同時表示し、
  レビュー中にコメント確認とターミナル操作を切り替えなしで行えるようにする

  Rule: コメント一覧とターミナルは左右に並列配置で同時表示される
    Scenario: 右パネル下部を表示する
      Given レビュー画面が表示されている
      When 右パネル下部が展開される
      Then コメント一覧とターミナルが左右に並列配置で表示される

  Rule: 分割サイズはドラッグで調整可能である
    Scenario: 分割比率を変更する
      Given コメント一覧とターミナルが分割表示されている
      When ユーザーが分割バーをドラッグする
      Then コメント一覧とターミナルのサイズ比率が変更される

  Rule: 右パネル下部は折りたたみ可能である
    Scenario: パネルを折りたたむ
      Given 右パネル下部が展開されている
      When ユーザーが折りたたみボタンを押す
      Then ヘッダーのみが表示される

    Scenario: パネルを展開する
      Given 右パネル下部が折りたたまれている
      When ユーザーが展開ボタンを押す
      Then コメント一覧とターミナルが分割表示で復元される

  Rule: コメント一覧はファイルごとにグループ化して未解決コメントを表示する
    Scenario: 未解決コメントの表示
      Given ワークツリーに複数ファイルへのコメントがある
      When ユーザーがコメント一覧を表示する
      Then ファイルパスごとにグループ化された未解決コメントが表示される

    Scenario: 解決済みコメントの表示切り替え
      Given 解決済みコメントが存在する
      When ユーザーが解決済み表示をトグルする
      Then 解決済みコメントが一覧に含まれる

  Rule: コメント一覧からターミナルにコメントを送信できる
    Scenario: 未解決コメントをターミナルに送信する
      Given 未解決コメントが存在する
      When ユーザーが送信ボタンを押す
      Then 未解決コメントがテキスト形式でターミナルに入力される

    Scenario: 未解決コメントをクリップボードにコピーする
      Given 未解決コメントが存在する
      When ユーザーがコピーボタンを押す
      Then 未解決コメントがテキスト形式でクリップボードにコピーされる
```

## 実装仕様

**対応方針**: コメント一覧とターミナルの同時表示を実現するために、`RightSidebarBottom.tsx` のタブUIを `react-resizable-panels` による左右分割レイアウトに置き換える。

**対象コンポーネント**:
- `src/components/panels/RightSidebarBottom.tsx`: タブUI (`Tabs`) → `Group orientation="horizontal"` + 2つの `Panel`（ターミナル左・コメント右）に変更。タブ関連のprops（`initialActiveTab`, `onActiveTabChange`）を削除。Copy/Sendボタンはコメントパネル内のフッターに配置。スクロールは既存の `ScrollArea` コンポーネントを使用
- `src/screens/MainLayout.tsx`: `RightSidebarBottom` に渡しているタブ関連propsの削除
- `src/screens/useWorktreeState.tsx`: `rightBottomActiveTab` 状態管理の削除
- `src/types/workspace-state.ts`: `rightBottomActiveTab` フィールドの削除、`RightBottomActiveTab` 型の削除、`normalizeRightBottomActiveTab` 関数の削除

**レイアウト構造**:
```
Panel "right-bottom" (既存、折りたたみ可能)
  └─ RightSidebarBottom
       ├─ ヘッダー（折りたたみボタンのみ）
       └─ Group orientation="horizontal"
            ├─ Panel "terminal" (50% default)
            │    └─ TerminalPanel
            ├─ Separator
            └─ Panel "comments" (50% default)
                 └─ CommentList（ScrollArea使用）
                      └─ フッター（Copy / Send ボタン）
```

**設計判断**:
- `ReviewPanel` の既存パターン（ファイルツリー/Diffビューの分割）を踏襲
- コメント・ターミナル各サブパネルは折りたたみ不可（パネル全体の折りたたみのみ対応）。Specに個別折りたたみの要求がないため

**影響するテスト**:
- `src/components/panels/RightSidebarBottom.test.tsx`: タブ切り替えテスト → 分割表示テストに書き換え。`react-resizable-panels` のモックが必要（jsdom非対応のため）
