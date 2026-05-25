{{project_name}} プロジェクトの `design.md` を作成または更新する。

## 入力

`spec-directory` Contract で渡される `spec_dir` を読み、以下を参照する。

- `${spec_dir}/requirements.md`
- `${spec_dir}/behavior.md`

## 目的

実装詳細ではなく、実装に進むための設計境界を定義する。Spec は実装詳細を見ない。design.md は、実装者が従う責務・契約・状態 owner・境界を明確にする。

本ステップでは `design.md` だけを作成・更新する。`requirements.md` と `behavior.md` は変更しない。

## design.md フォーマット

```markdown
# Design

## Behavior Coverage
behavior.md の各 Rule をどの設計方針で満たすか。

## Key Decisions
重要な設計判断、採用理由、採らなかった代替案。

## Responsibility Boundaries
どの領域が何を担当するか / しないか。

## Contracts
外部から見える契約。command、message、file format、document contract など。
内部 helper や詳細型は書かない。

## Data / Communication Flow
境界をまたぐ主要 flow。

## State Ownership
状態・データの owner。

## Boundaries
越えてはいけない責務境界。

## Implementation Freedom
実装に委ねること。
```

## 書かないこと

- 実装順序
- ファイルごとの編集手順
- helper 関数名
- 関数内処理
- 疑似コード
- 詳細な型定義やシグネチャ
- テストケース名
- security 実装の詳細（sanitize / escape / path traversal 対策方法など）

## 出力

`design.md` 更新後、`spec-directory` Contract に従って同じ `spec_dir` を構造化出力する。
