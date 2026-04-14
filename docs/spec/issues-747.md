## 要求

**種別**: バグ修正
**現在の挙動**: AskQuestion UIで選択肢がボタン形式で横一列に並び、各ボタンの幅が不均等。descriptionテキストがボタンの下に長い1行として表示され、どのボタンに対応するか判別しにくい。
**期待する挙動**: Claude Code本体のAskQuestion UIのように、選択肢が縦にリスト表示される形式にする。各選択肢がラベル付きで縦に並び、視認性・操作性が向上する。
**再現手順**:
1. ReleashアプリでAgent機能を使用
2. AgentがAskQuestion（複数選択肢付き）を発行
3. 選択肢がボタン形式で横並びに表示され、幅が不正
**背景**: 選択肢のdescriptionが長い場合にボタンレイアウトが破綻し、ユーザーが内容を把握しづらい。リスト形式にすることでClaude Code本体と一貫したUXを提供する。

## 振る舞い定義

```gherkin
Feature: AskQuestion選択肢のリスト表示
  AgentがAskQuestionを発行した際、選択肢を縦リスト形式で表示し、
  各選択肢のラベルとdescriptionの対応関係を明確にする。

  Rule: 選択肢は縦リスト形式で表示される
    Scenario: 複数の選択肢が縦に並んで表示される
      Given Agentが3つの選択肢付きAskQuestionを発行している
      When ユーザーが選択肢を確認する
      Then 各選択肢がラベルとdescriptionのペアとして縦に一覧表示される

    Scenario: Otherオプションもリスト内に表示される
      Given Agentが単一選択のAskQuestionを発行している
      When ユーザーが選択肢を確認する
      Then 定義済み選択肢の後にOtherオプションが同じリスト形式で表示される

  Rule: 選択肢の選択状態が視覚的に区別される
    Scenario: 単一選択で選択肢を選ぶ
      Given 単一選択のAskQuestionが表示されている
      When ユーザーが1つの選択肢を選択する
      Then 選択された選択肢が選択状態として視覚的に区別される

    Scenario: 複数選択で複数の選択肢を選ぶ
      Given 複数選択のAskQuestionが表示されている
      When ユーザーが複数の選択肢を選択する
      Then 選択された各選択肢が選択状態として視覚的に区別される

  Rule: Otherを選択すると自由入力欄が表示される
    Scenario: Otherを選択してテキストを入力する
      Given 単一選択のAskQuestionが表示されている
      When ユーザーがOtherを選択する
      Then テキスト入力欄が表示される
```

## 実装仕様

**対応方針**: AskQuestion選択肢の縦リスト表示を実現するために、`PermissionDialog.tsx` の選択肢レンダリング部分を `flex flex-wrap`（横並びボタン）から、既存の `RadioGroup` / `Checkbox` コンポーネントを使った縦リスト形式に変更する。

**対象コンポーネント**:
- `src/components/panels/AgentChatPanel/PermissionDialog.tsx`: 選択肢のレイアウトを縦リスト形式に変更。単一選択は `RadioGroup` + `RadioGroupItem`、複数選択は `Checkbox` を使用。各アイテムの右にラベル + description を配置。"Other" オプションも同じリスト内にアイテムとして表示。

**技術選定**:
- `src/components/ui/radio-group.tsx`（既存）: 単一選択の UI
- `src/components/ui/checkbox.tsx`（既存）: 複数選択の UI
- 新規ライブラリの導入なし

**検討した代替案**:
- カード型リスト: 各選択肢をボーダー付きカードとして縦に積む方式。よりリッチだが実装量が多く、既存のshadcn/uiコンポーネントを活用できない

**影響するテスト**:
- `PermissionDialog.test.tsx`: 既存テストのUI要素セレクタ（ボタンからラジオ/チェックボックスへ）を更新。複数選択（`multiSelect: true`）のテストケースを新規追加
