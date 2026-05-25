{{project_name}} プロジェクトの Spec ディレクトリを作成し、`requirements.md` を作成または更新する。

タスク: {{task}}

## 目的

実装に進む前に、何を実現するか、なぜ必要か、どこまでを今回の範囲とするかを明確にする。

Spec は 1 変更要求につき 1 ディレクトリとし、以下の 3 文書で構成する:

```text
docs/specs/<spec-id>/
  requirements.md
  behavior.md
  design.md
```

本ステップでは `requirements.md` だけを作成・更新する。`behavior.md` と `design.md` は作成しない。

## Spec ディレクトリ決定

1. issue 番号が分かる場合: `docs/specs/issues-XXX`
2. PJT 番号が分かる場合: `docs/specs/PJT-XXXX`
3. どちらも分からない場合: `docs/specs/workflow-{execution_id}`

## requirements.md に書くこと

```markdown
# Requirements

## Type
新機能 / 改善 / バグ修正 / リファクタリング

## Goal
何を実現するか。完了時にどうなっていれば成功か。

## Background
なぜこの変更が必要か。どんな課題を解決するか。

## Users / Actors
誰が関わるか。人間ユーザー、Agent、外部 system など。

## Scope
今回含めること。

## Non-goals
今回含めないこと。

## Requirements
満たすべき要求。

## Constraints
守るべき制約。技術実装ではなく要求上の制約だけを書く。

## Success Criteria
完了と判断する条件。

## Open Questions
未決定事項。未決定事項が残る場合は具体的な質問として残す。
```

## 書かないこと

- 振る舞い定義
- Gherkin
- 実装方針
- ファイル名、関数名、内部型、実装手順
- テストケースの具体内容

## 出力

Spec ディレクトリと `requirements.md` の作成・更新後、提出に必要な値を確定し、提出は出力Contract側の手順に従う。
