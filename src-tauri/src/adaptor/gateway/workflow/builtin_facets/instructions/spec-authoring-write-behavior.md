`{{ write_requirements.spec_dir }}` の `behavior.md` をユーザーとの対話で作成または更新する。

## 入力

`spec-directory` schema で渡される `spec_dir` を読み、`${spec_dir}/requirements.md` を参照する。

## 目的

requirements.md の要求を、実装詳細を含まない観測可能な振る舞いとして Gherkin で定義する。

この node では `behavior.md` だけを作成・更新する。`requirements.md` と `design.md` は変更しない。

## 基本方針

振る舞い定義は、ビジネスルールやユーザー/Agent から観測できる状態変化をアクター視点で表現する。受け入れテストの手順や実装手順ではない。

## 進め方

### 走査順（Rule）

Rule は `requirements.md` の要求の出現順に消化する。重要度や難易度で順番を入れ替えない。

各 Rule には複数の Scenario が含まれうる（正常系、失敗系、境界的な状況など）。1 Rule あたり 1 Scenario で済ませず、その Rule 内のすべての Scenario を消化してから次の Rule に進む。「この Rule は他に確認すべき Scenario はありますか」とユーザーに尋ねて、無いと確認できてから次へ移る。

### 最初のターン

1. `requirements.md` を読み、Rule 候補を要求の出現順で把握する。
2. `behavior.md` に Feature 見出しだけの雛形を用意する。
3. 最初の Rule の最初の Scenario について、requirements からの解釈と確認質問を 1 つだけ提示する。

### 2 ターン目以降

1. ユーザーの回答を該当 Rule / Scenario として Gherkin に追記・更新する。
2. 同じ Rule 内にまだ消化していない Scenario が残っていれば、その中の 1 つを質問する。残っていなければ、ユーザーに「この Rule は他に確認すべき Scenario はありますか」と尋ねる。無いと確認できたら、次の Rule の最初の Scenario を質問する。

### 完了条件

requirements.md の全要求が振る舞いとしてカバーできたら、ユーザーに最終確認を促す。承認をもってこの node を完了とする。

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
