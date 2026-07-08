`{{ request }}` を起点に Spec ディレクトリを作成し、`requirements.md` を一括で作成または更新する。

## 基本方針

- 最初にユーザーへヒアリングせず、与えられた task とリポジトリ内の情報から `requirements.md` を書き上げる。
- 判断できる内容は仮定を置いて進める。仮定は本文中で明確に分かるように記述する。
- 判断に迷う点だけを `Open Questions` セクションに明記する。
- `Open Questions` がある場合のみ、質問して解消する。質問は Open Question の解消に限定し、原則として一問一答で進める。
- 回答を得たら `requirements.md` を更新し、解消済みの Open Question を残さない。
- すべての Open Questions が解消したら、文書を完成状態にして人間のレビューを待つ。

## 出力

Spec は 1 変更要求につき 1 ディレクトリとし、以下の 3 文書で構成する:

```text
docs/specs/<spec-id>/
  requirements.md
  behavior.md
  design.md
```

本ステップでは `requirements.md` だけを作成・更新する。`behavior.md` と `design.md` は変更しない。

`requirements.md` には少なくとも以下を含める:

- 背景と目的
- スコープ
- 非スコープ
- 要求事項
- 受け入れ基準の概要
- 仮定
- Open Questions

Open Questions がすべて解消した後は、`Open Questions` セクションを削除するか、「なし」と明記する。
