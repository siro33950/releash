# Behavior

Issue #1260 / WorkspaceList の Worktree 展開ノード配下が空のときの明示表示。

requirements.md の R1〜R4 を、実装詳細に踏み込まない観測可能な振る舞いとして Gherkin で定義する。

## 仮定

- **A1 (文言)**: Worktree 配下の空状態プレースホルダ文言は `No sessions or workflows` とする (requirements.md 仮定 A1)。本 behavior 内では当該文言を前提に記述する。
- **A2 (対象範囲)**: 対象は Worktree 展開ノードの配下 (Session / Workflow) のみ。`RepoTreeSection` のブランチ一覧は対象外 (requirements.md 仮定 A2)。
- **A3 (状態の供給元)**: 「読み込み中」「エラー」「完了かつ件数」は既存のツリーノード取得が返す状態 (loading / error / nodes) から判定でき、新たなバックエンド変更は不要 (requirements.md 仮定 A3)。

## Feature

```gherkin
Feature: Worktree 展開ノードの空状態表示
  WorkspaceList で Worktree を展開したとき、配下の Session / Workflow が
  1 件も無い場合に「空である」ことを明示し、
  「読み込み中」「エラー」と視覚的に区別できるようにする。

  Background:
    Given WorkspaceList に 1 件以上のリポジトリが表示されている
    And そのリポジトリに 1 件以上の Worktree が表示されている
    And ユーザーが対象の Worktree ノードを展開している

  Rule: 配下が空のとき空状態プレースホルダを表示する

    Scenario: 配下に Session も Workflow も無い場合は空状態を明示する
      Given Worktree 配下のノード取得が正常に完了している
      And 配下の Session が 0 件である
      And 配下の Workflow が 0 件である
      When 展開した Worktree ノードの配下が描画される
      Then 空状態プレースホルダ "No sessions or workflows" が表示される
      And Session 行も Workflow 行も 1 つも表示されない

    Scenario: 配下に 1 件以上のノードがある場合は空状態を表示しない
      Given Worktree 配下のノード取得が正常に完了している
      And 配下に 1 件以上の Session または Workflow が存在する
      When 展開した Worktree ノードの配下が描画される
      Then 配下のノード一覧が従来どおり表示される
      And 空状態プレースホルダ "No sessions or workflows" は表示されない

  Rule: 空状態は読み込み中・エラーと排他であり混同させない

    Scenario: 読み込み中は空状態プレースホルダを表示しない
      Given Worktree 配下のノードを読み込み中である
      When 展開した Worktree ノードの配下が描画される
      Then 読み込み中表示 (スピナー) が表示される
      And 空状態プレースホルダ "No sessions or workflows" は表示されない

    Scenario: エラーかつ 0 件のときは空状態プレースホルダを表示しない
      Given Worktree 配下のノード取得がエラーで終了している
      And 配下のノードが 0 件である
      When 展開した Worktree ノードの配下が描画される
      Then エラー表示が表示される
      And 空状態プレースホルダ "No sessions or workflows" は表示されない

    Scenario Outline: 状態ごとに表示される要素が一意に定まる
      Given Worktree 配下の状態が "<state>" である
      When 展開した Worktree ノードの配下が描画される
      Then "<visible>" が表示される
      And それ以外の状態の表示要素は表示されない

      Examples:
        | state                | visible                   |
        | 読み込み中           | スピナー                  |
        | エラーかつ0件        | エラー表示                |
        | 完了かつ0件          | 空状態プレースホルダ      |
        | 完了かつ1件以上      | ノード一覧                |

  Rule: 空状態表示は既存の空状態表示と一貫した見た目とする

    Scenario: 空状態プレースホルダは控えめなスタイルで表示される
      Given Worktree 配下のノード取得が正常に完了し配下が 0 件である
      When 空状態プレースホルダが表示される
      Then 文言は既存の空状態表示 (例 "No Repository") と同系統の英語短文である
      And 控えめな配色 (muted-foreground 系) で表示される
      And Worktree 配下ノードと同等のインデントで表示される

  Rule: 折りたたみ時は空状態を含め配下を一切描画しない

    Scenario: Worktree を折りたたんでいる場合は何も描画しない
      Given ユーザーが対象の Worktree ノードを折りたたんでいる
      And 配下の Session / Workflow が 0 件である
      When Worktree ノードが描画される
      Then 空状態プレースホルダを含め配下要素は一切表示されない
```

## 受け入れ条件 (検証観点)

- Worktree 展開かつ完了かつ 0 件: `No sessions or workflows` が表示される。
- Worktree 展開かつ完了かつ 1 件以上: ノード一覧が表示され、プレースホルダは出ない。
- 読み込み中: スピナーのみ。プレースホルダは出ない。
- エラーかつ 0 件: エラー表示のみ。プレースホルダは出ない。
- 折りたたみ時: 配下は一切描画されない。
- 上記をカバーする `WorkspaceList.test.tsx` を追加し、`pnpm lint` / `pnpm test` が通る。

## Open Questions

なし。
