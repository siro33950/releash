{{project_name}} プロジェクトの `behavior.md` を作成または更新する。

## 入力

`spec-directory` Contract で渡される `spec_dir` を読み、`${spec_dir}/requirements.md` を参照する。

## 目的

requirements.md の要求を、実装詳細を含まない観測可能な振る舞いとして Gherkin で定義する。

本ステップでは `behavior.md` だけを作成・更新する。`requirements.md` と `design.md` は変更しない。

## 基本方針

振る舞い定義は、ビジネスルールやユーザー/Agent から観測できる状態変化をアクター視点で表現する。受け入れテストの手順や実装ステップではない。

## behavior.md フォーマット

```markdown
# Behavior

## Source
- requirements.md

## Behavior

```gherkin
Feature: [機能名]
  Rule: [ビジネスルール名]
    Scenario: [代表的な状況]
      Given [アクターの前提状況をビジネス用語で]
      When [アクターの行為をビジネス用語で]
      Then [ビジネスルール上の結果をビジネス用語で]
```
```

## 書くこと

- requirements.md の各要求に対応する Rule / Scenario
- 要求上重要な正常系と失敗系
- actor、状態、結果をビジネス用語で表したもの

## 書かないこと

- UI 部品名やクリック手順
- API、DB、Tauri command、WebSocket message などの技術呼び出し
- 具体的なレスポンスコード、画面文言、テスト assertion
- 実装手順、ファイル名、関数名
- 境界値やタイムアウト等のコードレベル edge case

## 出力

`behavior.md` 更新後、`spec-directory` Contract に従って同じ `spec_dir` を構造化出力する。
