`{{ write_requirements.spec_dir }}` の `design.md` を一括で作成または更新する。

## 入力

`spec-directory` schema で渡される `spec_dir` を読み、以下を参照する。

- `${spec_dir}/requirements.md`
- `${spec_dir}/behavior.md`

## 基本方針

- 最初にユーザーへヒアリングせず、requirements / behavior とリポジトリ内の情報から `design.md` を書き上げる。
- 実装方針、責務分割、データ構造、エラー処理、テスト方針を具体化する。
- 判断できる内容は仮定を置いて進める。仮定は本文中で明確に分かるように記述する。
- 判断に迷う点だけを `Open Questions` セクションに明記する。
- `Open Questions` がある場合のみ、質問して解消する。質問は Open Question の解消に限定し、原則として一問一答で進める。
- 回答を得たら `design.md` を更新し、解消済みの Open Question を残さない。
- すべての Open Questions が解消したら、文書を完成状態にして人間のレビューを待つ。

## 出力

この node では `design.md` だけを作成・更新する。`requirements.md` と `behavior.md` は変更しない。

`design.md` には少なくとも以下を含める:

- 概要
- 変更対象
- アーキテクチャと責務分割
- データモデルまたは型
- 処理フロー
- エラー処理
- テスト方針
- リスクと代替案
- 仮定
- Open Questions

Open Questions がすべて解消した後は、`Open Questions` セクションを削除するか、「なし」と明記する。
