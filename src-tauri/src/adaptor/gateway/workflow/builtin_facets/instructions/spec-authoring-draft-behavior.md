`{{ write_requirements.spec_dir }}` の `behavior.md` を一括で作成または更新する。

## 入力

`spec-directory` schema で渡される `spec_dir` を読み、`${spec_dir}/requirements.md` を参照する。

## 基本方針

- 最初にユーザーへヒアリングせず、`requirements.md` とリポジトリ内の情報から `behavior.md` を書き上げる。
- requirements の要求を、実装詳細を含まない観測可能な振る舞いとして Gherkin で定義する。
- 判断できる内容は仮定を置いて進める。仮定は本文中で明確に分かるように記述する。
- 判断に迷う点だけを `Open Questions` セクションに明記する。
- `Open Questions` がある場合のみ、質問して解消する。質問は Open Question の解消に限定し、原則として一問一答で進める。
- 回答を得たら `behavior.md` を更新し、解消済みの Open Question を残さない。
- すべての Open Questions が解消したら、文書を完成状態にして人間のレビューを待つ。

## 出力

本ステップでは `behavior.md` だけを作成・更新する。`requirements.md` と `design.md` は変更しない。

`behavior.md` には少なくとも以下を含める:

- Feature
- Background
- Rule
- Scenario または Scenario Outline
- 主要な正常系・異常系・境界条件
- 仮定
- Open Questions

Open Questions がすべて解消した後は、`Open Questions` セクションを削除するか、「なし」と明記する。
