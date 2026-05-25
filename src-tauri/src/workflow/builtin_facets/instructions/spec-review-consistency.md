{{project_name}} プロジェクトの Spec 3文書の整合性をレビューする。

## 入力

`spec-directory` Contract で渡される `spec_dir` を読み、以下を参照する。

- `${spec_dir}/requirements.md`
- `${spec_dir}/behavior.md`
- `${spec_dir}/design.md`

## 基本方針

Spec review は実装詳細を見ない。3文書間で要求、actor、用語、状態、scope が一貫しているかを確認する。

## 検証項目

### 1. 用語の一貫性

- 同一概念に同一用語が使われているか
- requirements.md で導入された概念が behavior.md / design.md で別名にすり替わっていないか

### 2. Scope の一貫性

- behavior.md が requirements.md の Scope / Non-goals を破っていないか
- design.md が requirements.md / behavior.md にない範囲を勝手に追加していないか

### 3. Actor / State の一貫性

- actor が3文書間で矛盾していないか
- behavior.md に登場する状態が design.md の State Ownership と対応しているか

### 4. 境界の一貫性

- design.md の Responsibility Boundaries / Contracts が requirements.md の Constraints と矛盾していないか
- design.md の Implementation Freedom が requirements.md / behavior.md の要求を曖昧化していないか

## 判定

- **LGTM**: 3文書間で用語・scope・actor・状態・境界に矛盾がない
- **NEEDS_FIX**: 用語不一致、scope creep、actor/state の矛盾、設計境界と要求制約の矛盾がある
